// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tracks set aside because their file is broken (migration 0124): hidden from
//! every list and player by the `music_tracks` view, kept with the reason.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Sets a track aside, with why. A track already in quarantine keeps its first reason.
pub async fn quarantine(pool: &PgPool, track_id: Uuid, reason: &str) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE music_tracks_all SET quarantined_at = now(), quarantine_reason = $2 WHERE id = $1 AND quarantined_at IS NULL")
        .bind(track_id)
        .bind(reason)
        .execute(pool)
        .await
        .context("db: quarantine track")?;
    Ok(done.rows_affected() > 0)
}

/// Lets a track back: an admin's decision, or a file that changed on disk and reads again.
pub async fn release(pool: &PgPool, track_id: Uuid) -> anyhow::Result<bool> {
    let done = sqlx::query("UPDATE music_tracks_all SET quarantined_at = NULL, quarantine_reason = NULL WHERE id = $1 AND quarantined_at IS NOT NULL")
        .bind(track_id)
        .execute(pool)
        .await
        .context("db: release track from quarantine")?;
    Ok(done.rows_affected() > 0)
}

#[derive(Debug, sqlx::FromRow)]
pub struct Quarantined {
    pub track_id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The file: a library folder's `folder/<id>/<path>` or an upload's key.
    pub file: String,
    pub size_bytes: Option<i64>,
    pub reason: String,
    pub since: DateTime<Utc>,
}

/// The household's quarantined tracks, newest first.
pub async fn list(pool: &PgPool, family_id: Uuid, user_id: Uuid) -> anyhow::Result<Vec<Quarantined>> {
    sqlx::query_as::<_, Quarantined>(
        "SELECT t.id AS track_id, t.title, t.artist, t.album, mo.object_key AS file, mo.size_bytes,
                t.quarantine_reason AS reason, t.quarantined_at AS since
           FROM music_tracks_all t
           JOIN media_objects mo ON mo.id = t.audio_object_id
          WHERE t.quarantined_at IS NOT NULL AND t.trashed_at IS NULL
            AND (t.family_id = $1 OR t.user_id = $2)
          ORDER BY t.quarantined_at DESC, t.title",
    )
    .bind(family_id)
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list quarantine")
}

/// Whether the track is the household's, in quarantine or not.
pub async fn belongs(pool: &PgPool, track_id: Uuid, family_id: Uuid, user_id: Uuid) -> anyhow::Result<bool> {
    let found: Option<Uuid> = sqlx::query_scalar("SELECT id FROM music_tracks_all WHERE id = $1 AND trashed_at IS NULL AND (family_id = $2 OR user_id = $3)")
        .bind(track_id)
        .bind(family_id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .context("db: quarantine lookup")?;
    Ok(found.is_some())
}
