// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use crate::audiobooks::models::{Author, BookAuthor, Tag};

// ── Authors ─────────────────────────────────────────────────────────────────

pub async fn list_authors(pool: &PgPool) -> anyhow::Result<Vec<Author>> {
    sqlx::query_as::<_, Author>(
        "SELECT id, name, sort_name, bio, image_object_id, created_at, updated_at
         FROM audiobook_authors ORDER BY COALESCE(sort_name, name) ASC",
    )
    .fetch_all(pool)
    .await
    .context("db: list authors")
}

pub async fn find_author(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Author>> {
    sqlx::query_as::<_, Author>(
        "SELECT id, name, sort_name, bio, image_object_id, created_at, updated_at
         FROM audiobook_authors WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("db: find author")
}

pub async fn find_author_by_name(pool: &PgPool, name: &str) -> anyhow::Result<Option<Author>> {
    sqlx::query_as::<_, Author>(
        "SELECT id, name, sort_name, bio, image_object_id, created_at, updated_at
         FROM audiobook_authors WHERE lower(name) = lower($1)",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .context("db: find author by name")
}

pub async fn create_author(
    pool: &PgPool,
    name: &str,
    sort_name: Option<&str>,
    bio: Option<&str>,
) -> anyhow::Result<Author> {
    sqlx::query_as::<_, Author>(
        "INSERT INTO audiobook_authors (name, sort_name, bio)
         VALUES ($1, $2, $3)
         RETURNING id, name, sort_name, bio, image_object_id, created_at, updated_at",
    )
    .bind(name)
    .bind(sort_name)
    .bind(bio)
    .fetch_one(pool)
    .await
    .context("db: create author")
}

pub async fn update_author(
    pool: &PgPool,
    id: Uuid,
    name: &str,
    sort_name: Option<&str>,
    bio: Option<&str>,
) -> anyhow::Result<Author> {
    sqlx::query_as::<_, Author>(
        "UPDATE audiobook_authors SET name = $2, sort_name = $3, bio = $4, updated_at = CURRENT_TIMESTAMP
         WHERE id = $1
         RETURNING id, name, sort_name, bio, image_object_id, created_at, updated_at",
    )
    .bind(id)
    .bind(name)
    .bind(sort_name)
    .bind(bio)
    .fetch_one(pool)
    .await
    .context("db: update author")
}

pub async fn delete_author(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_authors WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("db: delete author")?;
    Ok(())
}

// How many books does this author have?
// ── Author photos ────────────────────────────────────────────────────────────
//
// Same shape as music_artist_images (see db/music.rs), just keyed by the
// author's own id instead of (user_id, artist_name) — an author is already
// one global row, unlike a music artist, which has none of its own.

/// A remembered miss expires after this many days; a picture we hold does
/// not. Same constant and same reasoning as music's MISS_TTL_DAYS: Commons
/// gains photographs of long-tail people all the time.
const MISS_TTL_DAYS: i32 = 30;

/// Three-state, which is the point: `None` means never looked up, `Some(None)`
/// means looked up and nothing was found, `Some(Some(..))` means we have one.
pub async fn find_author_image(
    pool: &PgPool,
    author_id: Uuid,
) -> anyhow::Result<Option<Option<(String, String)>>> {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT mo.object_key, mo.content_type
         FROM audiobook_authors a
         LEFT JOIN media_objects mo ON mo.id = a.image_object_id
         WHERE a.id = $1
           AND (a.image_object_id IS NOT NULL
                OR a.is_user_set
                OR a.image_fetched_at > now() - make_interval(days => $2))",
    )
    .bind(author_id)
    .bind(MISS_TTL_DAYS)
    .fetch_optional(pool)
    .await
    .context("db: find author image")?;

    Ok(row.map(|(key, content_type)| match (key, content_type) {
        (Some(key), Some(content_type)) => Some((key, content_type)),
        _ => None,
    }))
}

/// The automatic path — records a miss (`image_object_id = NULL`) too, so an
/// author nothing has a picture of stops costing a network round trip on
/// every render. Never overwrites a user-set image.
pub async fn upsert_author_image(
    pool: &PgPool,
    author_id: Uuid,
    image_object_id: Option<Uuid>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_authors
         SET image_object_id = $2, image_fetched_at = now()
         WHERE id = $1 AND NOT is_user_set",
    )
    .bind(author_id)
    .bind(image_object_id)
    .execute(pool)
    .await
    .context("db: upsert author image")?;
    Ok(())
}

/// The automatic path, with the attribution its licence requires.
pub async fn upsert_fetched_author_image(
    pool: &PgPool,
    author_id: Uuid,
    image_object_id: Uuid,
    attribution: &crate::metadata::wikimedia::Attribution,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_authors
         SET image_object_id = $2, image_fetched_at = now(),
             image_source = 'Wikimedia Commons', image_author = $3,
             image_license = $4, image_license_url = $5, image_source_url = $6
         WHERE id = $1 AND NOT is_user_set",
    )
    .bind(author_id)
    .bind(image_object_id)
    .bind(attribution.author.as_deref())
    .bind(attribution.license.as_deref())
    .bind(attribution.license_url.as_deref())
    .bind(&attribution.source_url)
    .execute(pool)
    .await
    .context("db: upsert fetched author image")?;
    Ok(())
}

/// What a client must display alongside the image. `None` for a user's own
/// upload, which has nothing to attribute.
pub async fn find_author_image_attribution(
    pool: &PgPool,
    author_id: Uuid,
) -> anyhow::Result<Option<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, bool)>> {
    Ok(sqlx::query_as(
        "SELECT image_source, image_author, image_license, image_license_url, image_source_url, is_user_set
         FROM audiobook_authors
         WHERE id = $1 AND image_object_id IS NOT NULL",
    )
    .bind(author_id)
    .fetch_optional(pool)
    .await?)
}

/// The user's own picture. Unconditional, unlike the automatic path above:
/// choosing one is exactly the act that should win, and it clears whatever
/// attribution a previous automatic fetch had recorded.
pub async fn set_user_author_image(
    pool: &PgPool,
    author_id: Uuid,
    image_object_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_authors
         SET image_object_id = $2, image_fetched_at = now(), is_user_set = TRUE,
             image_source = NULL, image_author = NULL, image_license = NULL,
             image_license_url = NULL, image_source_url = NULL
         WHERE id = $1",
    )
    .bind(author_id)
    .bind(image_object_id)
    .execute(pool)
    .await
    .context("db: set user author image")?;
    Ok(())
}

/// Clears the image back to nothing (not just unpicks `is_user_set`) so the
/// automatic lookup runs fresh rather than being suppressed by a stale
/// cached-miss timestamp — the row itself stays, unlike music's cache-only
/// table, since this is the author.
pub async fn clear_author_image(pool: &PgPool, author_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_authors
         SET image_object_id = NULL, image_fetched_at = NULL, is_user_set = FALSE,
             image_source = NULL, image_author = NULL, image_license = NULL,
             image_license_url = NULL, image_source_url = NULL
         WHERE id = $1",
    )
    .bind(author_id)
    .execute(pool)
    .await
    .context("db: clear author image")?;
    Ok(())
}

pub async fn author_book_count(pool: &PgPool, author_id: Uuid) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(DISTINCT book_id) FROM audiobook_book_authors WHERE author_id = $1",
    )
    .bind(author_id)
    .fetch_one(pool)
    .await
    .context("db: author book count")
}

// ── Book ↔ Author links ────────────────────────────────────────────────────

pub async fn list_book_authors(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<BookAuthor>> {
    sqlx::query_as::<_, BookAuthor>(
        "SELECT book_id, author_id, role FROM audiobook_book_authors WHERE book_id = $1",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list book authors")
}

pub async fn list_author_books(pool: &PgPool, author_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT DISTINCT book_id FROM audiobook_book_authors WHERE author_id = $1",
    )
    .bind(author_id)
    .fetch_all(pool)
    .await
    .context("db: list author books")
}

pub async fn link_book_author(
    pool: &PgPool,
    book_id: Uuid,
    author_id: Uuid,
    role: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audiobook_book_authors (book_id, author_id, role)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(book_id)
    .bind(author_id)
    .bind(role)
    .execute(pool)
    .await
    .context("db: link book author")?;
    Ok(())
}

pub async fn unlink_book_author(
    pool: &PgPool,
    book_id: Uuid,
    author_id: Uuid,
    role: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM audiobook_book_authors WHERE book_id = $1 AND author_id = $2 AND role = $3",
    )
    .bind(book_id)
    .bind(author_id)
    .bind(role)
    .execute(pool)
    .await
    .context("db: unlink book author")?;
    Ok(())
}

/// Get or create an author by name (case-insensitive).
pub async fn get_or_create_author(pool: &PgPool, name: &str) -> anyhow::Result<Author> {
    if let Some(existing) = find_author_by_name(pool, name).await? {
        return Ok(existing);
    }
    create_author(pool, name, None, None).await
}

// ── Tags ────────────────────────────────────────────────────────────────────

pub async fn list_tags(pool: &PgPool) -> anyhow::Result<Vec<Tag>> {
    sqlx::query_as::<_, Tag>("SELECT id, name FROM audiobook_tags ORDER BY name ASC")
        .fetch_all(pool)
        .await
        .context("db: list tags")
}

pub async fn get_or_create_tag(pool: &PgPool, name: &str) -> anyhow::Result<Tag> {
    let existing = sqlx::query_as::<_, Tag>(
        "SELECT id, name FROM audiobook_tags WHERE lower(name) = lower($1)",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .context("db: find tag")?;

    if let Some(t) = existing {
        return Ok(t);
    }

    sqlx::query_as::<_, Tag>(
        "INSERT INTO audiobook_tags (name) VALUES ($1) RETURNING id, name",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .context("db: create tag")
}

pub async fn list_book_tags(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<Tag>> {
    sqlx::query_as::<_, Tag>(
        "SELECT t.id, t.name FROM audiobook_tags t
         JOIN audiobook_book_tags bt ON bt.tag_id = t.id
         WHERE bt.book_id = $1
         ORDER BY t.name",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list book tags")
}

pub async fn set_book_tags(pool: &PgPool, book_id: Uuid, tag_ids: &[Uuid]) -> anyhow::Result<()> {
    // Remove existing
    sqlx::query("DELETE FROM audiobook_book_tags WHERE book_id = $1")
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: clear book tags")?;

    // Insert new
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO audiobook_book_tags (book_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(book_id)
            .bind(tag_id)
            .execute(pool)
            .await
            .context("db: add book tag")?;
    }
    Ok(())
}
