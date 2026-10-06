// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::audiobooks::models::{AudiobookBook, AudiobookChapter, AudiobookFile};
use crate::db::access::{AUDIOBOOK, VISIBLE, Viewer};
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) const BOOK_COLS: &str =
    "id, user_id, family_id, title, author, narrator, description, cover_object_id,
     total_duration_secs, source_url, google_books_volume_id, isbn, publisher,
     published_year, created_at, updated_at";

/// Bind the four leading parameters every [`VISIBLE`] query expects.
macro_rules! bind_viewer {
    ($q:expr, $viewer:expr) => {
        $q.bind($viewer.user_id)
            .bind($viewer.family_id)
            .bind($viewer.is_family_admin)
            .bind(AUDIOBOOK)
    };
}

// ── Books ──────────────────────────────────────────────────────────────────

pub async fn list_books(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<AudiobookBook>> {
    let sql =
        format!("SELECT {BOOK_COLS} FROM audiobook_books t WHERE {VISIBLE} ORDER BY t.title");

    bind_viewer!(sqlx::query_as::<_, AudiobookBook>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list audiobooks")
}

pub async fn find_book(
    pool: &PgPool,
    id: Uuid,
    viewer: Viewer,
) -> anyhow::Result<Option<AudiobookBook>> {
    let sql =
        format!("SELECT {BOOK_COLS} FROM audiobook_books t WHERE t.id = $5 AND {VISIBLE}");

    bind_viewer!(sqlx::query_as::<_, AudiobookBook>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find audiobook")
}

/// Owner-scoped lookup for mutations (edit, delete, re-share).
pub async fn find_book_owned(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<AudiobookBook>> {
    sqlx::query_as::<_, AudiobookBook>(&format!(
        "SELECT {BOOK_COLS} FROM audiobook_books WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find owned audiobook")
}

/// Create a book inside the caller's transaction.
///
/// Both upload paths — multipart and direct-to-storage — create a book exactly
/// this way. They each carried their own copy of the `RETURNING` list, which went
/// stale the moment 0059 added the identify columns: every audiobook upload then
/// failed to decode the returned row and answered 500. That is the same failure
/// `db::music::TRACK_COLS` exists to prevent, so the list lives here only.
#[allow(clippy::too_many_arguments)]
pub async fn insert_book_full<'e, E>(
    executor: E,
    user_id: Uuid,
    family_id: Option<Uuid>,
    title: &str,
    author: Option<&str>,
    narrator: Option<&str>,
    description: Option<&str>,
    total_duration_secs: Option<i32>,
) -> anyhow::Result<AudiobookBook>
where
    E: sqlx::PgExecutor<'e>,
{
    sqlx::query_as::<_, AudiobookBook>(&format!(
        "INSERT INTO audiobook_books
         (user_id, family_id, title, author, narrator, description, total_duration_secs)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING {BOOK_COLS}"
    ))
    .bind(user_id)
    .bind(family_id)
    .bind(title)
    .bind(author)
    .bind(narrator)
    .bind(description)
    .bind(total_duration_secs)
    .fetch_one(executor)
    .await
    .context("db: insert audiobook")
}

pub async fn insert_book(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Option<Uuid>,
    title: &str,
    author: Option<&str>,
) -> anyhow::Result<AudiobookBook> {
    sqlx::query_as::<_, AudiobookBook>(&format!(
        "INSERT INTO audiobook_books (user_id, family_id, title, author)
         VALUES ($1, $2, $3, $4)
         RETURNING {BOOK_COLS}"
    ))
    .bind(user_id)
    .bind(family_id)
    .bind(title)
    .bind(author)
    .fetch_one(pool)
    .await
    .context("db: insert audiobook")
}

/// The book, if this caller may **administer** it: its owner, or a family admin when the book is
/// shared with that family.
///
/// Deliberately not "any book an admin can see". A member's private book is not family content —
/// nobody else can even play it — so an admin reaching into one would be overreach, not
/// administration. The family_id match is what draws that line.
pub async fn find_book_manageable(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    family_id: Uuid,
    is_family_admin: bool,
) -> anyhow::Result<Option<AudiobookBook>> {
    sqlx::query_as::<_, AudiobookBook>(&format!(
        "SELECT {BOOK_COLS} FROM audiobook_books
         WHERE id = $1
           AND (user_id = $2 OR ($3 AND family_id = $4))"
    ))
    .bind(id)
    .bind(user_id)
    .bind(is_family_admin)
    .bind(family_id)
    .fetch_optional(pool)
    .await
    .context("db: find manageable audiobook")
}

pub async fn update_book(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    title: &str,
    author: Option<&str>,
    narrator: Option<&str>,
    description: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_books
         SET title = $3,
             author = $4,
             narrator = $5,
             description = $6,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(author)
    .bind(narrator)
    .bind(description)
    .execute(pool)
    .await
    .context("db: update audiobook")?;

    Ok(())
}

/// Writes the fields a Google Books match was allowed to touch.
///
/// Every value is optional and `NULL` means "leave what is there", not "clear
/// it": the client picks per field which parts of a match to accept, and an
/// unpicked field must survive the apply untouched. `narrator` is absent by
/// design — Google Books describes the print edition and has no narrator.
#[allow(clippy::too_many_arguments)]
pub async fn apply_google_books_metadata(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    volume_id: &str,
    title: Option<&str>,
    author: Option<&str>,
    description: Option<&str>,
    publisher: Option<&str>,
    published_year: Option<i32>,
    isbn: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_books
         SET title          = COALESCE($4, title),
             author         = COALESCE($5, author),
             description    = COALESCE($6, description),
             publisher      = COALESCE($7, publisher),
             published_year = COALESCE($8, published_year),
             isbn           = COALESCE($9, isbn),
             google_books_volume_id = $3,
             updated_at     = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(volume_id)
    .bind(title)
    .bind(author)
    .bind(description)
    .bind(publisher)
    .bind(published_year)
    .bind(isbn)
    .execute(pool)
    .await
    .context("db: apply google books metadata to audiobook")?;

    Ok(())
}

// ── Files ──────────────────────────────────────────────────────────────────

pub async fn list_files(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<AudiobookFile>> {
    sqlx::query_as::<_, AudiobookFile>(
        "SELECT f.id, f.book_id, f.position, f.title, f.duration_secs, f.audio_object_id,
                f.created_at, m.size_bytes, m.sha256
         FROM audiobook_files f
         JOIN media_objects m ON m.id = f.audio_object_id
         WHERE f.book_id = $1 ORDER BY f.position",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list audiobook files")
}

pub async fn find_file(pool: &PgPool, book_id: Uuid, file_id: Uuid) -> anyhow::Result<Option<AudiobookFile>> {
    sqlx::query_as::<_, AudiobookFile>(
        "SELECT f.id, f.book_id, f.position, f.title, f.duration_secs, f.audio_object_id,
                f.created_at, m.size_bytes, m.sha256
         FROM audiobook_files f
         JOIN media_objects m ON m.id = f.audio_object_id
         WHERE f.book_id = $1 AND f.id = $2",
    )
    .bind(book_id)
    .bind(file_id)
    .fetch_optional(pool)
    .await
    .context("db: find audiobook file")
}

pub async fn update_file_title(
    pool: &PgPool,
    book_id: Uuid,
    user_id: Uuid,
    file_id: Uuid,
    title: &str,
) -> anyhow::Result<()> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM audiobook_books WHERE id = $1 AND user_id = $2)",
    )
    .bind(book_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: verify book ownership")?;

    if !exists {
        anyhow::bail!("book not found or not owned by user");
    }

    sqlx::query("UPDATE audiobook_files SET title = $1 WHERE id = $2 AND book_id = $3")
        .bind(title)
        .bind(file_id)
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: update audiobook file title")?;

    Ok(())
}

/// Removes one file from a multi-file book — a fix for a botched or duplicate upload, not a way
/// to retire the whole book (that goes through the 30-day trash, see `audiobooks::delete_book`).
/// Hard delete: a stray, wrongly split file doesn't belong sitting in the trash for a month.
/// Returns the file's `audio_object_id` so the caller can free the underlying object once
/// nothing else references it — this function only ever touches the row.
pub async fn delete_file(
    pool: &PgPool,
    book_id: Uuid,
    user_id: Uuid,
    file_id: Uuid,
) -> anyhow::Result<Option<Uuid>> {
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM audiobook_books WHERE id = $1 AND user_id = $2)",
    )
    .bind(book_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: verify book ownership")?;

    if !exists {
        anyhow::bail!("book not found or not owned by user");
    }

    let object_id = sqlx::query_scalar::<_, Uuid>(
        "DELETE FROM audiobook_files WHERE id = $1 AND book_id = $2 RETURNING audio_object_id",
    )
    .bind(file_id)
    .bind(book_id)
    .fetch_optional(pool)
    .await
    .context("db: delete audiobook file")?;

    recalculate_total_duration(pool, book_id).await?;

    Ok(object_id)
}

/// The storage key of one audiobook file's audio, or None once it is gone.
pub async fn file_audio_key(pool: &PgPool, file_id: Uuid) -> anyhow::Result<Option<(Uuid, String)>> {
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT f.book_id, m.object_key
         FROM audiobook_files f
         JOIN media_objects m ON m.id = f.audio_object_id
         WHERE f.id = $1",
    )
    .bind(file_id)
    .fetch_optional(pool)
    .await
    .context("db: audiobook file audio key")
}

pub async fn set_file_duration(
    pool: &PgPool,
    file_id: Uuid,
    duration_secs: Option<i32>,
) -> anyhow::Result<()> {
    sqlx::query("UPDATE audiobook_files SET duration_secs = $2 WHERE id = $1")
        .bind(file_id)
        .bind(duration_secs)
        .execute(pool)
        .await
        .context("db: set audiobook file duration")?;
    Ok(())
}

/// Queue up the files whose duration nobody ever measured.
///
/// The duration of an audiobook file is whatever the **uploading client** said it was: the
/// server reads it from a form field and stores it, and never looks at the audio itself. When a
/// client does not send one the column stays NULL, `recalculate_total_duration` then correctly
/// refuses to invent a total, and every client shows a book with no length at all.
///
/// That is not hypothetical: the Mac app's Audiobookshelf import dropped the field entirely
/// until audio2-mac d6531ac (2026-09-20), so every book imported before then arrived blank —
/// five of fourteen on canary — and the fix only applies to imports made after it. This sweep
/// is what repairs the ones already sitting there, and what keeps a future client's omission
/// from being permanent.
///
/// Unguarded, unlike the analysis sweep: how long a book is is not a feature anyone opts into,
/// it is the most basic thing a shelf shows.
pub async fn enqueue_missing_file_durations(pool: &PgPool, limit: i64) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "INSERT INTO jobs (job_type, payload, status)
         SELECT 'book_file_duration',
                jsonb_build_object('file_id', f.id::text),
                'pending'
         FROM audiobook_files f
         -- Zero counts as missing on purpose. A real file is never zero seconds long, and an
         -- earlier build of this job wrote zero whenever the probe could not reach storage; a
         -- strict `IS NULL` would leave every one of those stamped files unmeasured forever.
         WHERE (f.duration_secs IS NULL OR f.duration_secs = 0)
           AND f.audio_object_id IS NOT NULL
           AND NOT EXISTS (
               SELECT 1 FROM jobs j
               WHERE j.job_type = 'book_file_duration'
                 AND j.status IN ('pending', 'running')
                 AND j.payload->>'file_id' = f.id::text
           )
         ORDER BY f.book_id, f.position
         LIMIT $1",
    )
    .bind(limit)
    .execute(pool)
    .await
    .context("db: enqueue missing audiobook file durations")?;

    Ok(result.rows_affected())
}

pub async fn recalculate_total_duration(pool: &PgPool, book_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_books
         SET total_duration_secs = (
                SELECT CASE
                    WHEN COUNT(*) = 0 THEN NULL
                    WHEN COUNT(duration_secs) = 0 THEN NULL
                    ELSE SUM(duration_secs)
                END
                FROM audiobook_files
                WHERE book_id = $1
             ),
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1",
    )
    .bind(book_id)
    .execute(pool)
    .await
    .context("db: recalculate audiobook duration")?;

    Ok(())
}

// ── Chapters ───────────────────────────────────────────────────────────────

pub async fn reorder_files(
    pool: &PgPool,
    book_id: Uuid,
    user_id: Uuid,
    file_ids: &[Uuid],
) -> anyhow::Result<()> {
    // Verify book belongs to user
    let exists = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS(SELECT 1 FROM audiobook_books WHERE id = $1 AND user_id = $2)",
    )
    .bind(book_id)
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: verify book ownership")?;

    if !exists {
        anyhow::bail!("book not found or not owned by user");
    }

    let mut tx = pool.begin().await.context("db: begin tx for reorder")?;

    // Park the positions in negative space first. `audiobook_files_book_position_uq` is
    // checked per statement, not at commit, so assigning the new positions directly makes
    // the very first swap collide with the row that still holds that position — a 500 on
    // every reorder. Same trick as `reorder_playlist_tracks`.
    sqlx::query("UPDATE audiobook_files SET position = -position WHERE book_id = $1")
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .context("db: clear audiobook file positions")?;

    for (index, file_id) in file_ids.iter().enumerate() {
        sqlx::query(
            "UPDATE audiobook_files SET position = $1 WHERE id = $2 AND book_id = $3",
        )
        .bind((index + 1) as i32)
        .bind(file_id)
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .context("db: reorder audiobook file")?;
    }

    tx.commit().await.context("db: commit reorder")?;
    Ok(())
}

pub async fn list_chapters(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<AudiobookChapter>> {
    sqlx::query_as::<_, AudiobookChapter>(
        "SELECT id, book_id, file_id, position, title, start_time_secs, created_at
         FROM audiobook_chapters WHERE book_id = $1 ORDER BY position",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list audiobook chapters")
}
