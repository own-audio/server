// SPDX-License-Identifier: AGPL-3.0-or-later
//! Presigned uploads that may still be in flight (migration 0094).
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

pub async fn insert_intent(
    pool: &PgPool,
    object_key: &str,
    family_id: Uuid,
    user_id: Uuid,
    declared_size: Option<i64>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO upload_intents (object_key, family_id, user_id, declared_size)
         VALUES ($1, $2, $3, $4) ON CONFLICT (object_key) DO NOTHING",
    )
    .bind(object_key)
    .bind(family_id)
    .bind(user_id)
    .bind(declared_size)
    .execute(pool)
    .await
    .context("db: record upload intent")?;
    Ok(())
}

/// The declared size, if the intent exists; removes it either way, since the
/// upload is complete.
pub async fn take_intent(pool: &PgPool, object_key: &str) -> anyhow::Result<Option<Option<i64>>> {
    sqlx::query_scalar("DELETE FROM upload_intents WHERE object_key = $1 RETURNING declared_size")
        .bind(object_key)
        .fetch_optional(pool)
        .await
        .context("db: take upload intent")
}

/// Intents older than `older_than_hours` whose object was never completed.
pub async fn stale_intents(pool: &PgPool, older_than_hours: i64, limit: i64) -> anyhow::Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT i.object_key FROM upload_intents i
         WHERE i.created_at < now() - make_interval(hours => $1::int)
           AND NOT EXISTS (SELECT 1 FROM media_objects m WHERE m.object_key = i.object_key)
         ORDER BY i.created_at LIMIT $2",
    )
    .bind(older_than_hours)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: stale upload intents")
}

pub async fn delete_intent(pool: &PgPool, object_key: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM upload_intents WHERE object_key = $1")
        .bind(object_key)
        .execute(pool)
        .await
        .context("db: delete upload intent")?;
    Ok(())
}
