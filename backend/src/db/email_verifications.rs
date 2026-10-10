// SPDX-License-Identifier: AGPL-3.0-or-later
//! Email verification links (migration 0097): the same shape as password
//! resets — only a hash is kept, the newest link is the one that works, and
//! [`take`] spends it in one statement.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn insert(pool: &PgPool, user_id: Uuid, token_hash: &str, expires_at: DateTime<Utc>) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin email verification")?;
    sqlx::query("UPDATE email_verifications SET used_at = now() WHERE user_id = $1 AND used_at IS NULL")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .context("db: void earlier email verifications")?;
    sqlx::query("INSERT INTO email_verifications (user_id, token_hash, expires_at) VALUES ($1, $2, $3)")
        .bind(user_id)
        .bind(token_hash)
        .bind(expires_at)
        .execute(&mut *tx)
        .await
        .context("db: insert email verification")?;
    tx.commit().await.context("db: commit email verification")?;
    Ok(())
}

/// The user a live, unused link belongs to; spent on the way out.
pub async fn take(pool: &PgPool, token_hash: &str) -> anyhow::Result<Option<Uuid>> {
    sqlx::query_scalar(
        "UPDATE email_verifications SET used_at = now()
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()
         RETURNING user_id",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .context("db: take email verification")
}
