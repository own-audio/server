// SPDX-License-Identifier: AGPL-3.0-or-later
//! The security audit trail (migration 0099). See `auth::audit` for what is
//! recorded and how.
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;
use utoipa::ToSchema;
use uuid::Uuid;

pub struct NewEvent<'a> {
    pub user_id: Option<Uuid>,
    pub actor_id: Option<Uuid>,
    pub kind: &'a str,
    pub detail: serde_json::Value,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

pub async fn insert(pool: &PgPool, e: NewEvent<'_>) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO security_events (user_id, actor_id, kind, detail, ip, user_agent)
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(e.user_id)
    .bind(e.actor_id)
    .bind(e.kind)
    .bind(e.detail)
    .bind(e.ip)
    .bind(e.user_agent)
    .execute(pool)
    .await
    .context("db: insert security event")?;
    Ok(())
}

#[derive(Debug, Serialize, ToSchema, sqlx::FromRow)]
pub struct SecurityEvent {
    pub id: Uuid,
    pub at: DateTime<Utc>,
    pub kind: String,
    pub detail: serde_json::Value,
    /// Set when someone other than the account's owner did it (an admin).
    pub actor_id: Option<Uuid>,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

pub async fn list_for_user(pool: &PgPool, user_id: Uuid, limit: i64) -> anyhow::Result<Vec<SecurityEvent>> {
    sqlx::query_as(
        "SELECT id, at, kind, detail, actor_id, ip, user_agent FROM security_events
         WHERE user_id = $1 ORDER BY at DESC LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list security events")
}

/// Retention: rows older than `days` go. Returns how many.
pub async fn purge_older_than(pool: &PgPool, days: i64) -> anyhow::Result<u64> {
    let rows = sqlx::query("DELETE FROM security_events WHERE at < now() - make_interval(days => $1::int)")
        .bind(days)
        .execute(pool)
        .await
        .context("db: purge security events")?
        .rows_affected();
    Ok(rows)
}
