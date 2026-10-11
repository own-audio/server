// SPDX-License-Identifier: AGPL-3.0-or-later
//! Two-factor sign-in rows (migration 0098). Secrets arrive and leave sealed
//! (`auth::at_rest`); this module never sees them in the clear.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, sqlx::FromRow)]
pub struct Totp {
    pub secret: String,
    pub enabled_at: Option<DateTime<Utc>>,
    pub last_used_step: i64,
}

pub async fn find(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Option<Totp>> {
    sqlx::query_as("SELECT secret, enabled_at, last_used_step FROM user_totp WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .context("db: find totp")
}

pub async fn is_enabled(pool: &PgPool, user_id: Uuid) -> anyhow::Result<bool> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM user_totp WHERE user_id = $1 AND enabled_at IS NOT NULL)")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .context("db: totp enabled")
}

/// A new pending secret; replaces an earlier pending one, never an enabled one.
pub async fn start(pool: &PgPool, user_id: Uuid, sealed_secret: &str) -> anyhow::Result<bool> {
    let rows = sqlx::query(
        "INSERT INTO user_totp (user_id, secret) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET secret = EXCLUDED.secret, last_used_step = 0, created_at = now()
         WHERE user_totp.enabled_at IS NULL",
    )
    .bind(user_id)
    .bind(sealed_secret)
    .execute(pool)
    .await
    .context("db: start totp")?
    .rows_affected();
    Ok(rows > 0)
}

pub async fn enable(pool: &PgPool, user_id: Uuid, step: i64) -> anyhow::Result<()> {
    sqlx::query("UPDATE user_totp SET enabled_at = now(), last_used_step = $2 WHERE user_id = $1")
        .bind(user_id)
        .bind(step)
        .execute(pool)
        .await
        .context("db: enable totp")?;
    Ok(())
}

/// Accepts a code's step once: `false` when that step (or a later one) was used already.
pub async fn use_step(pool: &PgPool, user_id: Uuid, step: i64) -> anyhow::Result<bool> {
    let rows = sqlx::query("UPDATE user_totp SET last_used_step = $2 WHERE user_id = $1 AND last_used_step < $2")
        .bind(user_id)
        .bind(step)
        .execute(pool)
        .await
        .context("db: use totp step")?
        .rows_affected();
    Ok(rows > 0)
}

pub async fn remove(pool: &PgPool, user_id: Uuid) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin remove totp")?;
    sqlx::query("DELETE FROM user_totp WHERE user_id = $1").bind(user_id).execute(&mut *tx).await.context("db: remove totp")?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .context("db: remove recovery codes")?;
    tx.commit().await.context("db: commit remove totp")?;
    Ok(())
}

pub async fn replace_recovery_codes(pool: &PgPool, user_id: Uuid, hashes: &[String]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin recovery codes")?;
    sqlx::query("DELETE FROM user_recovery_codes WHERE user_id = $1")
        .bind(user_id)
        .execute(&mut *tx)
        .await
        .context("db: clear recovery codes")?;
    for h in hashes {
        sqlx::query("INSERT INTO user_recovery_codes (user_id, code_hash) VALUES ($1, $2)")
            .bind(user_id)
            .bind(h)
            .execute(&mut *tx)
            .await
            .context("db: insert recovery code")?;
    }
    tx.commit().await.context("db: commit recovery codes")?;
    Ok(())
}

/// Spends a recovery code; `false` when it is unknown or already used.
pub async fn use_recovery_code(pool: &PgPool, user_id: Uuid, code_hash: &str) -> anyhow::Result<bool> {
    let rows = sqlx::query(
        "UPDATE user_recovery_codes SET used_at = now() WHERE user_id = $1 AND code_hash = $2 AND used_at IS NULL",
    )
    .bind(user_id)
    .bind(code_hash)
    .execute(pool)
    .await
    .context("db: use recovery code")?
    .rows_affected();
    Ok(rows > 0)
}

pub async fn recovery_codes_left(pool: &PgPool, user_id: Uuid) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM user_recovery_codes WHERE user_id = $1 AND used_at IS NULL")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .context("db: recovery codes left")
}

#[derive(Debug, sqlx::FromRow)]
pub struct Challenge {
    pub user_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: Option<String>,
}

pub async fn insert_challenge(
    pool: &PgPool,
    token_hash: &str,
    user_id: Uuid,
    device_name: Option<&str>,
    device_kind: Option<&str>,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO mfa_challenges (token_hash, user_id, device_name, device_kind, expires_at) VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(token_hash)
    .bind(user_id)
    .bind(device_name)
    .bind(device_kind)
    .bind(expires_at)
    .execute(pool)
    .await
    .context("db: insert mfa challenge")?;
    Ok(())
}

/// The challenge behind a token, if it is live; not spent yet — a wrong code
/// may be retried a few times, the per-IP limit and the expiry bound that.
pub async fn find_challenge(pool: &PgPool, token_hash: &str) -> anyhow::Result<Option<Challenge>> {
    sqlx::query_as(
        "SELECT user_id, device_name, device_kind FROM mfa_challenges
         WHERE token_hash = $1 AND used_at IS NULL AND expires_at > now()",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .context("db: find mfa challenge")
}

pub async fn spend_challenge(pool: &PgPool, token_hash: &str) -> anyhow::Result<bool> {
    let rows = sqlx::query("UPDATE mfa_challenges SET used_at = now() WHERE token_hash = $1 AND used_at IS NULL")
        .bind(token_hash)
        .execute(pool)
        .await
        .context("db: spend mfa challenge")?
        .rows_affected();
    Ok(rows > 0)
}
