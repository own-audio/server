// SPDX-License-Identifier: AGPL-3.0-or-later
/// Device-authorization requests: a TV (or anything else without a keyboard) asks for a code,
/// someone already signed in approves it. See migration 0066.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sha2::{Digest, Sha256};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DeviceAuthRequest {
    pub id: Uuid,
    pub user_code: String,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub status: String,
    pub approved_user_id: Option<Uuid>,
    pub last_polled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

impl DeviceAuthRequest {
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

/// SHA-256 hex of the device code — the only form ever stored, as for refresh tokens.
pub fn hash_code(secret: &str) -> String {
    format!("{:x}", Sha256::digest(secret.as_bytes()))
}

pub async fn create(
    pool: &PgPool,
    device_code_hash: &str,
    user_code: &str,
    device_name: Option<&str>,
    device_kind: &str,
    expires_at: DateTime<Utc>,
) -> anyhow::Result<DeviceAuthRequest> {
    sqlx::query_as::<_, DeviceAuthRequest>(
        "INSERT INTO device_auth_requests
            (device_code_hash, user_code, device_name, device_kind, expires_at)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, user_code, device_name, device_kind, status, approved_user_id,
                   last_polled_at, created_at, expires_at",
    )
    .bind(device_code_hash)
    .bind(user_code)
    .bind(device_name)
    .bind(device_kind)
    .bind(expires_at)
    .fetch_one(pool)
    .await
    .context("insert device auth request")
}

pub async fn find_by_code_hash(
    pool: &PgPool,
    device_code_hash: &str,
) -> anyhow::Result<Option<DeviceAuthRequest>> {
    sqlx::query_as::<_, DeviceAuthRequest>(
        "SELECT id, user_code, device_name, device_kind, status, approved_user_id,
                last_polled_at, created_at, expires_at
         FROM device_auth_requests WHERE device_code_hash = $1",
    )
    .bind(device_code_hash)
    .fetch_optional(pool)
    .await
    .context("find device auth request by code hash")
}

/// Pending, unexpired requests only: an approver looking up a code should never be offered one
/// that is already used, denied or stale.
pub async fn find_pending_by_user_code(
    pool: &PgPool,
    user_code: &str,
) -> anyhow::Result<Option<DeviceAuthRequest>> {
    sqlx::query_as::<_, DeviceAuthRequest>(
        "SELECT id, user_code, device_name, device_kind, status, approved_user_id,
                last_polled_at, created_at, expires_at
         FROM device_auth_requests
         WHERE user_code = $1 AND status = 'pending' AND expires_at > now()",
    )
    .bind(user_code)
    .fetch_optional(pool)
    .await
    .context("find pending device auth request by user code")
}

/// Approve or deny, but only while still pending — so a second approver can't overwrite the
/// first, and an expired request can't be revived.
pub async fn resolve(
    pool: &PgPool,
    id: Uuid,
    status: &str,
    approved_user_id: Option<Uuid>,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE device_auth_requests
         SET status = $2, approved_user_id = $3
         WHERE id = $1 AND status = 'pending' AND expires_at > now()",
    )
    .bind(id)
    .bind(status)
    .bind(approved_user_id)
    .execute(pool)
    .await
    .context("resolve device auth request")?;
    Ok(result.rows_affected() == 1)
}

/// Marks an approved request used, and says whether this caller is the one that got there first:
/// the tokens are handed out exactly once, however many pollers are racing.
pub async fn consume(pool: &PgPool, id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE device_auth_requests SET status = 'consumed' WHERE id = $1 AND status = 'approved'",
    )
    .bind(id)
    .execute(pool)
    .await
    .context("consume device auth request")?;
    Ok(result.rows_affected() == 1)
}

pub async fn touch_poll(pool: &PgPool, id: Uuid, at: DateTime<Utc>) -> anyhow::Result<()> {
    sqlx::query("UPDATE device_auth_requests SET last_polled_at = $2 WHERE id = $1")
        .bind(id)
        .bind(at)
        .execute(pool)
        .await
        .context("touch device auth poll")?;
    Ok(())
}

/// Housekeeping: nothing depends on old rows, and the short codes should be free to reuse.
pub async fn delete_expired(pool: &PgPool) -> anyhow::Result<u64> {
    let result = sqlx::query("DELETE FROM device_auth_requests WHERE expires_at < now() - interval '1 day'")
        .execute(pool)
        .await
        .context("delete expired device auth requests")?;
    Ok(result.rows_affected())
}
