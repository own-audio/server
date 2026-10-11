// SPDX-License-Identifier: AGPL-3.0-or-later
/// Refresh-token persistence. A chain = one signed-in device; tokens rotate
/// on every refresh but keep their chain_id. See migration 0017.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RefreshToken {
    pub id: Uuid,
    pub user_id: Uuid,
    pub chain_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub created_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub replaced_by: Option<Uuid>,
}

/// One row per signed-in device (the newest active token of each chain).
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeviceSession {
    pub chain_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub signed_in_at: DateTime<Utc>,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: DateTime<Utc>,
}

/// SHA-256 hex of a token secret — the only form ever stored.
pub fn hash_token(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

pub async fn create(
    pool: &PgPool,
    user_id: Uuid,
    chain_id: Uuid,
    token_hash: &str,
    device_name: Option<&str>,
    device_kind: &str,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<RefreshToken> {
    sqlx::query_as::<_, RefreshToken>(
        "INSERT INTO refresh_tokens
             (user_id, chain_id, token_hash, device_name, device_kind, expires_at)
         VALUES ($1, $2, $3, $4, $5, $6)
         RETURNING id, user_id, chain_id, device_name, device_kind,
                   created_at, last_used_at, expires_at, revoked_at, replaced_by",
    )
    .bind(user_id)
    .bind(chain_id)
    .bind(token_hash)
    .bind(device_name)
    .bind(device_kind)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .context("db: create refresh token")
}

pub async fn find_by_hash(pool: &PgPool, token_hash: &str) -> anyhow::Result<Option<RefreshToken>> {
    sqlx::query_as::<_, RefreshToken>(
        "SELECT id, user_id, chain_id, device_name, device_kind,
                created_at, last_used_at, expires_at, revoked_at, replaced_by
         FROM refresh_tokens WHERE token_hash = $1",
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await
    .context("db: find refresh token")
}

/// Atomically retire `old_id` in favour of `new_id`. Returns `false` when the
/// old token was already rotated/revoked by a concurrent request — the caller
/// must treat that as token reuse and revoke the chain.
pub async fn mark_rotated(pool: &PgPool, old_id: Uuid, new_id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE refresh_tokens
         SET revoked_at = now(), replaced_by = $2, last_used_at = now()
         WHERE id = $1 AND revoked_at IS NULL",
    )
    .bind(old_id)
    .bind(new_id)
    .execute(pool)
    .await
    .context("db: rotate refresh token")?;

    Ok(result.rows_affected() == 1)
}

/// Revoke every refresh token in a chain and every access session minted from
/// it (a device sign-out). Scoped by user_id so users can only kill their own.
pub async fn revoke_chain(pool: &PgPool, chain_id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = now()
         WHERE chain_id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(chain_id)
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: revoke refresh chain")?;

    sqlx::query(
        "UPDATE sessions SET revoked_at = now()
         WHERE chain_id = $1 AND user_id = $2 AND revoked_at IS NULL",
    )
    .bind(chain_id)
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: revoke chain sessions")?;

    // A signed-out device holds nothing any more (file-sync-plan §5.9).
    sqlx::query("DELETE FROM device_holdings WHERE chain_id = $1 AND user_id = $2")
        .bind(chain_id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: drop chain holdings")?;

    Ok(())
}

/// Revoke all of a user's refresh chains (and their sessions), optionally
/// sparing one chain — used by password change to keep the current device.
pub async fn revoke_all_for_user(
    pool: &PgPool,
    user_id: Uuid,
    except_chain: Option<Uuid>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE refresh_tokens SET revoked_at = now()
         WHERE user_id = $1 AND revoked_at IS NULL
           AND ($2::uuid IS NULL OR chain_id <> $2)",
    )
    .bind(user_id)
    .bind(except_chain)
    .execute(pool)
    .await
    .context("db: revoke all refresh chains")?;

    sqlx::query(
        "DELETE FROM device_holdings
         WHERE user_id = $1 AND ($2::uuid IS NULL OR chain_id <> $2)",
    )
    .bind(user_id)
    .bind(except_chain)
    .execute(pool)
    .await
    .context("db: drop revoked holdings")?;

    Ok(())
}

pub async fn list_devices(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<DeviceSession>> {
    sqlx::query_as::<_, DeviceSession>(
        // `last_used_at` is stamped on the token being rotated *away* (see `mark_rotated`),
        // while the row this picks is the newest live one in the chain — which by definition
        // has not been rotated yet, and so always carried NULL. Every device therefore read as
        // "never used", including the one making the request. Both the first sign-in and the
        // last use are properties of the chain, not of one token, so both come from the
        // aggregate.
        "SELECT DISTINCT ON (rt.chain_id)
                rt.chain_id, rt.device_name, rt.device_kind,
                chains.signed_in_at, chains.last_used_at, rt.expires_at
         FROM refresh_tokens rt
         JOIN (SELECT chain_id, MIN(created_at) AS signed_in_at, MAX(last_used_at) AS last_used_at
               FROM refresh_tokens WHERE user_id = $1 GROUP BY chain_id) chains
           USING (chain_id)
         WHERE rt.user_id = $1 AND rt.revoked_at IS NULL AND rt.expires_at > now()
         ORDER BY rt.chain_id, rt.created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list device sessions")
}

/// Whether the account has signed in from a device like this one (same kind
/// and name) in the last `days` days, or has never signed in at all — the
/// cases where a sign-in is not news worth a mail (`mail::new_device`).
pub async fn device_is_familiar(
    pool: &PgPool,
    user_id: Uuid,
    device_kind: &str,
    device_name: Option<&str>,
    days: i64,
) -> anyhow::Result<bool> {
    let (any, same): (bool, bool) = sqlx::query_as(
        "SELECT EXISTS (SELECT 1 FROM refresh_tokens WHERE user_id = $1),
                EXISTS (SELECT 1 FROM refresh_tokens
                         WHERE user_id = $1 AND device_kind = $2
                           AND COALESCE(device_name, '') = COALESCE($3, '')
                           AND created_at > now() - make_interval(days => $4::int))",
    )
    .bind(user_id)
    .bind(device_kind)
    .bind(device_name)
    .bind(days)
    .fetch_one(pool)
    .await
    .context("db: familiar device")?;
    Ok(!any || same)
}
