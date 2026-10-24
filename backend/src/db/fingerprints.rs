// SPDX-License-Identifier: AGPL-3.0-or-later
//! Acoustic fingerprints of songs (migration 0119): which songs still need one,
//! and what a fingerprint run found.

use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

/// Songs without a fingerprint of their current file, oldest first: never
/// fingerprinted, or the file was replaced or changed since. With
/// `only_unidentified`, only songs no identify has matched (hosted: a read of
/// object storage is a request, and AcoustID is paid beyond its free lookups).
pub async fn pending(pool: &PgPool, only_unidentified: bool, limit: i64) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT t.id
           FROM music_tracks_all t
           JOIN media_objects mo ON mo.id = t.audio_object_id
           LEFT JOIN track_fingerprints f ON f.track_id = t.id
          WHERE t.trashed_at IS NULL AND t.missing_at IS NULL
            AND (f.track_id IS NULL
                 OR f.audio_object_id <> t.audio_object_id
                 OR f.size_bytes IS DISTINCT FROM mo.size_bytes)
            AND (NOT $1 OR NOT EXISTS (SELECT 1 FROM music_track_identity i WHERE i.track_id = t.id))
          ORDER BY t.created_at
          LIMIT $2",
    )
    .bind(only_unidentified)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: songs without a fingerprint")
}

/// What a fingerprint run needs about one song: its file and length.
#[derive(Debug, sqlx::FromRow)]
pub struct Source {
    pub object_key: String,
    pub audio_object_id: Uuid,
    pub size_bytes: Option<i64>,
    pub duration_secs: Option<i32>,
}

pub async fn source(pool: &PgPool, track_id: Uuid) -> anyhow::Result<Option<Source>> {
    sqlx::query_as(
        "SELECT mo.object_key, t.audio_object_id, mo.size_bytes, t.duration_secs
           FROM music_tracks_all t
           JOIN media_objects mo ON mo.id = t.audio_object_id
          WHERE t.id = $1 AND t.trashed_at IS NULL",
    )
    .bind(track_id)
    .fetch_optional(pool)
    .await
    .context("db: the file to fingerprint")
}

/// A run's result: the fingerprint, or (`fingerprint` None) why there is none.
pub async fn save(
    pool: &PgPool,
    track_id: Uuid,
    source: &Source,
    fingerprint: Option<&str>,
    error: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO track_fingerprints (track_id, audio_object_id, size_bytes, fingerprint, duration_secs, error)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (track_id) DO UPDATE
            SET audio_object_id = EXCLUDED.audio_object_id, size_bytes = EXCLUDED.size_bytes,
                fingerprint = EXCLUDED.fingerprint, duration_secs = EXCLUDED.duration_secs,
                error = EXCLUDED.error, computed_at = now()",
    )
    .bind(track_id)
    .bind(source.audio_object_id)
    .bind(source.size_bytes)
    .bind(fingerprint)
    .bind(source.duration_secs)
    .bind(error)
    .execute(pool)
    .await
    .context("db: save a fingerprint")?;
    Ok(())
}
