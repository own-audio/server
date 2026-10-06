// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::playback::models::{
    AudiobookProgress, Bookmark, EqSettings, PodcastProgress, UserSettings,
};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

// ── Podcast progress ───────────────────────────────────────────────────────

pub async fn get_podcast_progress(
    pool: &PgPool,
    user_id: Uuid,
    episode_id: Uuid,
) -> anyhow::Result<Option<PodcastProgress>> {
    sqlx::query_as::<_, PodcastProgress>(
        "SELECT user_id, episode_id, position_secs, completed, updated_at
         FROM podcast_progress WHERE user_id = $1 AND episode_id = $2",
    )
    .bind(user_id)
    .bind(episode_id)
    .fetch_optional(pool)
    .await
    .context("db: get podcast progress")
}

/// Mark many episodes played or unplayed at once. Marking as played parks the
/// position at the episode's duration so "continue listening" agrees with the
/// completed flag; marking unplayed rewinds to the start.
pub async fn bulk_set_episode_completed(
    pool: &PgPool,
    user_id: Uuid,
    episode_ids: &[Uuid],
    completed: bool,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "INSERT INTO podcast_progress (user_id, episode_id, position_secs, completed)
         SELECT $1, e.id,
                CASE WHEN $3 THEN COALESCE(e.duration_secs, 0)::double precision ELSE 0 END,
                $3
         FROM podcast_episodes e
         WHERE e.id = ANY($2)
         ON CONFLICT (user_id, episode_id) DO UPDATE
             SET position_secs = EXCLUDED.position_secs,
                 completed     = EXCLUDED.completed,
                 updated_at    = CURRENT_TIMESTAMP",
    )
    .bind(user_id)
    .bind(episode_ids)
    .bind(completed)
    .execute(pool)
    .await
    .context("db: bulk set episode completion")?;

    Ok(result.rows_affected())
}

/// The feed an episode belongs to. Listening stats are recorded against the
/// feed (the shareable item), with the episode kept as the finer-grained part.
pub async fn episode_feed_id(pool: &PgPool, episode_id: Uuid) -> anyhow::Result<Option<Uuid>> {
    sqlx::query_scalar::<_, Uuid>("SELECT feed_id FROM podcast_episodes WHERE id = $1")
        .bind(episode_id)
        .fetch_optional(pool)
        .await
        .context("db: episode feed id")
}

pub async fn upsert_podcast_progress(
    pool: &PgPool,
    user_id: Uuid,
    episode_id: Uuid,
    position_secs: f64,
    completed: bool,
) -> anyhow::Result<PodcastProgress> {
    sqlx::query_as::<_, PodcastProgress>(
        "INSERT INTO podcast_progress (user_id, episode_id, position_secs, completed)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, episode_id) DO UPDATE
             SET position_secs = EXCLUDED.position_secs,
                 completed     = EXCLUDED.completed,
                 updated_at    = CURRENT_TIMESTAMP
         RETURNING user_id, episode_id, position_secs, completed, updated_at",
    )
    .bind(user_id)
    .bind(episode_id)
    .bind(position_secs)
    .bind(completed)
    .fetch_one(pool)
    .await
    .context("db: upsert podcast progress")
}

// ── Audiobook progress ────────────────────────────────────────────────────

pub async fn get_audiobook_progress(
    pool: &PgPool,
    user_id: Uuid,
    book_id: Uuid,
) -> anyhow::Result<Option<AudiobookProgress>> {
    sqlx::query_as::<_, AudiobookProgress>(
        "SELECT user_id, book_id, file_id, position_secs, completed, updated_at
         FROM audiobook_progress WHERE user_id = $1 AND book_id = $2",
    )
    .bind(user_id)
    .bind(book_id)
    .fetch_optional(pool)
    .await
    .context("db: get audiobook progress")
}

/// "Start over": clears the row entirely rather than writing position 0 to whichever file was
/// last playing. Playback with no progress row starts at file one — see `lib/play.ts`'s
/// `playBook`, which only seeks into a specific file when `progress?.file_id` names one.
pub async fn delete_audiobook_progress(
    pool: &PgPool,
    user_id: Uuid,
    book_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_progress WHERE user_id = $1 AND book_id = $2")
        .bind(user_id)
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: delete audiobook progress")?;
    Ok(())
}

/// Every book this user has progress on, with the position counted across the whole book.
///
/// The position stored per row is scoped to one file; the LATERAL sum adds the files before it,
/// exactly as `library::continue_listening` does — a client dividing the raw per-file value by
/// the book's total duration would understate anyone past file one. One request rather than one
/// per book: a shelf of forty books is one screen, not forty round trips.
///
/// Both positions come back because a client needs both and can derive neither: the book-wide one
/// draws the progress bar, the per-file one plus `file_id` is what resuming actually seeks to, and
/// a device that only syncs the shelf has no file list to split the book-wide value back apart.
pub async fn list_audiobook_progress(
    pool: &PgPool,
    user_id: Uuid,
) -> anyhow::Result<Vec<(Uuid, Uuid, f64, f64, bool, DateTime<Utc>)>> {
    sqlx::query_as::<_, (Uuid, Uuid, f64, f64, bool, DateTime<Utc>)>(
        "SELECT ap.book_id, ap.file_id,
                COALESCE(prior.prior_secs, 0) + ap.position_secs,
                ap.position_secs, ap.completed, ap.updated_at
         FROM audiobook_progress ap
         JOIN audiobook_files cur ON cur.id = ap.file_id
         LEFT JOIN LATERAL (
             SELECT SUM(af.duration_secs) AS prior_secs
             FROM audiobook_files af
             WHERE af.book_id = ap.book_id AND af.position < cur.position
         ) prior ON true
         WHERE ap.user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list audiobook progress")
}

pub async fn upsert_audiobook_progress(
    pool: &PgPool,
    user_id: Uuid,
    book_id: Uuid,
    file_id: Uuid,
    position_secs: f64,
    completed: bool,
) -> anyhow::Result<AudiobookProgress> {
    sqlx::query_as::<_, AudiobookProgress>(
        "INSERT INTO audiobook_progress (user_id, book_id, file_id, position_secs, completed)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, book_id) DO UPDATE
             SET file_id       = EXCLUDED.file_id,
                 position_secs = EXCLUDED.position_secs,
                 completed     = EXCLUDED.completed,
                 updated_at    = CURRENT_TIMESTAMP
         RETURNING user_id, book_id, file_id, position_secs, completed, updated_at",
    )
    .bind(user_id)
    .bind(book_id)
    .bind(file_id)
    .bind(position_secs)
    .bind(completed)
    .fetch_one(pool)
    .await
    .context("db: upsert audiobook progress")
}

// ── Bookmarks ──────────────────────────────────────────────────────────────

pub async fn list_bookmarks_for_user(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<Bookmark>> {
    sqlx::query_as::<_, Bookmark>(
        "SELECT id, user_id, episode_id, book_id, file_id, position_secs, label, audio_object_id, created_at
         FROM bookmarks WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list bookmarks")
}

pub async fn list_bookmarks_for_book(
    pool: &PgPool,
    user_id: Uuid,
    book_id: Uuid,
) -> anyhow::Result<Vec<Bookmark>> {
    sqlx::query_as::<_, Bookmark>(
        "SELECT id, user_id, episode_id, book_id, file_id, position_secs, label, audio_object_id, created_at
         FROM bookmarks WHERE user_id = $1 AND book_id = $2 ORDER BY position_secs ASC",
    )
    .bind(user_id)
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list bookmarks for book")
}

#[allow(clippy::too_many_arguments)]
pub async fn create_bookmark(
    pool: &PgPool,
    user_id: Uuid,
    book_id: Option<Uuid>,
    episode_id: Option<Uuid>,
    file_id: Option<Uuid>,
    position_secs: f64,
    label: Option<&str>,
    audio_object_id: Option<Uuid>,
) -> anyhow::Result<Bookmark> {
    sqlx::query_as::<_, Bookmark>(
        "INSERT INTO bookmarks (user_id, book_id, episode_id, file_id, position_secs, label, audio_object_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING id, user_id, episode_id, book_id, file_id, position_secs, label, audio_object_id, created_at",
    )
    .bind(user_id)
    .bind(book_id)
    .bind(episode_id)
    .bind(file_id)
    .bind(position_secs)
    .bind(label)
    .bind(audio_object_id)
    .fetch_one(pool)
    .await
    .context("db: create bookmark")
}

pub async fn update_bookmark_label(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    label: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE bookmarks SET label = $3 WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(label)
    .execute(pool)
    .await
    .context("db: update bookmark label")?;
    Ok(())
}

pub async fn delete_bookmark(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM bookmarks WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete bookmark")?;
    Ok(())
}

// ── User settings ─────────────────────────────────────────────────────────

pub async fn get_or_create_settings(pool: &PgPool, user_id: Uuid) -> anyhow::Result<UserSettings> {
    sqlx::query(
        "INSERT INTO user_settings (user_id) VALUES ($1) ON CONFLICT (user_id) DO NOTHING",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: ensure user settings row")?;

    sqlx::query_as::<_, UserSettings>(
        "SELECT user_id, playback_speed, skip_intro_secs, skip_outro_secs,
                ab_skip_forward_secs, ab_skip_backward_secs, ab_playback_speed, updated_at
         FROM user_settings WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: get user settings")
}

pub async fn update_audiobook_defaults(
    pool: &PgPool,
    user_id: Uuid,
    ab_skip_forward_secs: i32,
    ab_skip_backward_secs: i32,
    ab_playback_speed: f64,
) -> anyhow::Result<UserSettings> {
    // Ensure the row exists first
    get_or_create_settings(pool, user_id).await?;

    sqlx::query_as::<_, UserSettings>(
        "UPDATE user_settings
         SET ab_skip_forward_secs = $2, ab_skip_backward_secs = $3, ab_playback_speed = $4, updated_at = CURRENT_TIMESTAMP
         WHERE user_id = $1
         RETURNING user_id, playback_speed, skip_intro_secs, skip_outro_secs,
                   ab_skip_forward_secs, ab_skip_backward_secs, ab_playback_speed, updated_at",
    )
    .bind(user_id)
    .bind(ab_skip_forward_secs)
    .bind(ab_skip_backward_secs)
    .bind(ab_playback_speed)
    .fetch_one(pool)
    .await
    .context("db: update audiobook defaults")
}

// ── Equalizer settings ────────────────────────────────────────────────────

const EQ_COLUMNS: &str = "user_id, device_kind, enabled, preamp_db, band1_db, band2_db, \
     band3_db, band4_db, band5_db, band6_db, preset_name, updated_at";

/// Flat/off defaults for a device that has never saved an EQ — not persisted
/// until the user actually changes something, so a device that never opens
/// the equalizer never gets a row.
fn default_eq_settings(user_id: Uuid, device_kind: &str) -> EqSettings {
    EqSettings {
        user_id,
        device_kind: device_kind.to_string(),
        enabled: false,
        preamp_db: 0.0,
        band1_db: 0.0,
        band2_db: 0.0,
        band3_db: 0.0,
        band4_db: 0.0,
        band5_db: 0.0,
        band6_db: 0.0,
        preset_name: Some("Flat".to_string()),
        updated_at: chrono::Utc::now(),
    }
}

pub async fn get_eq_settings(
    pool: &PgPool,
    user_id: Uuid,
    device_kind: &str,
) -> anyhow::Result<EqSettings> {
    let row = sqlx::query_as::<_, EqSettings>(&format!(
        "SELECT {EQ_COLUMNS} FROM eq_settings WHERE user_id = $1 AND device_kind = $2",
    ))
    .bind(user_id)
    .bind(device_kind)
    .fetch_optional(pool)
    .await
    .context("db: get eq settings")?;

    Ok(row.unwrap_or_else(|| default_eq_settings(user_id, device_kind)))
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_eq_settings(
    pool: &PgPool,
    user_id: Uuid,
    device_kind: &str,
    enabled: bool,
    preamp_db: f64,
    band1_db: f64,
    band2_db: f64,
    band3_db: f64,
    band4_db: f64,
    band5_db: f64,
    band6_db: f64,
    preset_name: Option<&str>,
) -> anyhow::Result<EqSettings> {
    sqlx::query_as::<_, EqSettings>(&format!(
        "INSERT INTO eq_settings ({EQ_COLUMNS})
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, CURRENT_TIMESTAMP)
         ON CONFLICT (user_id, device_kind) DO UPDATE
         SET enabled = $3, preamp_db = $4, band1_db = $5, band2_db = $6,
             band3_db = $7, band4_db = $8, band5_db = $9, band6_db = $10,
             preset_name = $11, updated_at = CURRENT_TIMESTAMP
         RETURNING {EQ_COLUMNS}",
    ))
    .bind(user_id)
    .bind(device_kind)
    .bind(enabled)
    .bind(preamp_db)
    .bind(band1_db)
    .bind(band2_db)
    .bind(band3_db)
    .bind(band4_db)
    .bind(band5_db)
    .bind(band6_db)
    .bind(preset_name)
    .fetch_one(pool)
    .await
    .context("db: upsert eq settings")
}
