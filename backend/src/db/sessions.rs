// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::users::models::Session;
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn insert(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
    user_agent: Option<&str>,
    ip_address: Option<&str>,
) -> anyhow::Result<Session> {
    insert_with_chain(pool, id, user_id, expires_at, user_agent, ip_address, None).await
}

pub async fn insert_with_chain(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    expires_at: DateTime<Utc>,
    user_agent: Option<&str>,
    ip_address: Option<&str>,
    chain_id: Option<Uuid>,
) -> anyhow::Result<Session> {
    sqlx::query_as::<_, Session>(
        "INSERT INTO sessions (id, user_id, expires_at, user_agent, ip_address, chain_id)
         VALUES ($1, $2, $3, $4, CAST($5 AS INET), $6)
         RETURNING id, user_id, created_at, expires_at, revoked_at,
                   user_agent, CAST(ip_address AS TEXT) as ip_address",
    )
    .bind(id)
    .bind(user_id)
    .bind(expires_at)
    .bind(user_agent)
    .bind(ip_address)
    .bind(chain_id)
    .fetch_one(pool)
    .await
    .context("db: insert session")
}

/// True when the session row exists and has been revoked. A missing row is
/// treated as *not* revoked for compatibility with tokens whose best-effort
/// session insert failed before this check existed.
pub async fn is_revoked(pool: &PgPool, id: Uuid) -> anyhow::Result<bool> {
    let revoked_at: Option<Option<DateTime<Utc>>> =
        sqlx::query_scalar("SELECT revoked_at FROM sessions WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("db: check session revocation")?;

    Ok(matches!(revoked_at, Some(Some(_))))
}

pub async fn find(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Session>> {
    sqlx::query_as::<_, Session>(
        "SELECT id, user_id, created_at, expires_at, revoked_at,
                user_agent, CAST(ip_address AS TEXT) as ip_address
         FROM sessions WHERE id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("db: find session")
}

pub async fn revoke(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("UPDATE sessions SET revoked_at = CURRENT_TIMESTAMP WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("db: revoke session")?;
    Ok(())
}

pub async fn revoke_all_for_user(pool: &PgPool, user_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE sessions SET revoked_at = CURRENT_TIMESTAMP WHERE user_id = $1 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: revoke all sessions for user")?;
    Ok(())
}

/// Revoke all of a user's sessions except one — used by password change so
/// the device performing the change stays logged in.
pub async fn revoke_all_except(pool: &PgPool, user_id: Uuid, keep_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE sessions SET revoked_at = CURRENT_TIMESTAMP
         WHERE user_id = $1 AND id <> $2 AND revoked_at IS NULL",
    )
    .bind(user_id)
    .bind(keep_id)
    .execute(pool)
    .await
    .context("db: revoke other sessions for user")?;
    Ok(())
}

/// Logins by rolling window (last 1/7/30 days), as `(d1, d7, d30)` — for the
/// instance-admin dashboard. A login is the *first* row of a `chain_id`
/// chain, not every `sessions` row: `issue_tokens` (auth/mod.rs) inserts a
/// new session row on every access-token refresh too, reusing the same
/// chain_id, so counting raw `created_at` would massively overcount (one
/// signed-in device can refresh many times a day). The `MIN(created_at)` per
/// chain has to run over the whole table before filtering by window — a
/// chain's true first-seen can't be determined from a truncated slice.
pub async fn login_counts(pool: &PgPool) -> anyhow::Result<(i64, i64, i64)> {
    sqlx::query_as::<_, (i64, i64, i64)>(
        "WITH chains AS (
             SELECT chain_id, MIN(created_at) AS first_seen
             FROM sessions
             WHERE chain_id IS NOT NULL
             GROUP BY chain_id
         )
         SELECT
             COUNT(*) FILTER (WHERE first_seen >= now() - interval '1 day')::BIGINT,
             COUNT(*) FILTER (WHERE first_seen >= now() - interval '7 days')::BIGINT,
             COUNT(*) FILTER (WHERE first_seen >= now() - interval '30 days')::BIGINT
         FROM chains",
    )
    .fetch_one(pool)
    .await
    .context("db: login counts")
}
