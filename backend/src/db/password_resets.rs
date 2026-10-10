// SPDX-License-Identifier: AGPL-3.0-or-later
//! Password reset links (migration 0096). Only the hash of a link's secret
//! is stored; [`take`] spends it in one statement so two uses cannot race.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// Records a new link and voids the user's earlier unused ones: the newest
/// mail is the one that works.
pub async fn insert(pool: &PgPool, user_id: Uuid, token_hash: &str, expires_at: DateTime<Utc>) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin password reset")?;
    sqlx::query("UPDATE password_resets SET used_at = now() WHERE user_id = $1 AND used_at IS NULL")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .context("db: void earlier password resets")?;
    sqlx::query("INSERT INTO password_resets (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .context("db: insert password reset")?;
    tx.commit().await.context("db: commit password reset")?;
    Ok(())
}

/// The user a live, unused link belongs to; the link is spent on the way out.
pub async fn take(pool: &PgPool, token_hash: &str) -> anyhow::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "UPDATE password_resets SET used_at = now()
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .context("db: take password reset")
}
