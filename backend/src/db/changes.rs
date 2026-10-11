// SPDX-License-Identifier: AGPL-3.0-or-later
//! Reading the change log of migration 0109 (docs/api-v2-plan.md, V5).
//!
//! The cursor is the oldest transaction still running when the previous round
//! began, as in the file-sync feed (`filesync::tree`): a transaction that
//! commits late is still picked up, at the price of an item now and then
//! arriving twice, which a client applies idempotently.
use crate::db::access::Viewer;
use anyhow::Context;
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

/// The log is kept this long; an older cursor answers `410 sync_cursor_expired`.
pub const RETENTION_DAYS: i64 = 180;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Cursor {
    /// Oldest transaction this round must still look at.
    from: i64,
    /// Log rows already sent in this round.
    #[serde(default)]
    seq: i64,
    /// Where the next round starts; fixed when a round begins.
    next: i64,
    /// When `from` was taken (unix seconds), for the retention check.
    at: i64,
}

impl Cursor {
    pub fn encode(&self) -> String {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("cursor serialises"))
    }

    pub fn decode(raw: &str) -> Option<Self> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw.trim()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }

    pub fn expired(&self) -> bool {
        Utc::now().timestamp() - self.at > RETENTION_DAYS * 86_400
    }
}

async fn snapshot_xmin(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot())::text::bigint")
        .fetch_one(pool)
        .await
        .context("db: snapshot xmin")
}

/// A cursor for "from now on": what a snapshot starts from.
pub async fn now(pool: &PgPool) -> anyhow::Result<Cursor> {
    let x = snapshot_xmin(pool).await?;
    Ok(Cursor { from: x, seq: 0, next: x, at: Utc::now().timestamp() })
}

/// What changed after `cursor` that the viewer's household or the viewer is
/// named on: (kind, item), each once, in the order they changed.
pub async fn read(pool: &PgPool, viewer: Viewer, cursor: &Cursor, limit: i64) -> anyhow::Result<(Vec<(String, Uuid)>, Cursor, bool)> {
    let (next, at) = if cursor.seq == 0 { (snapshot_xmin(pool).await?, Utc::now().timestamp()) } else { (cursor.next, cursor.at) };
    let rows: Vec<(i64, String, Uuid)> = sqlx::query_as(
        "SELECT seq, kind, item_id FROM changes
          WHERE xid >= $1::text::xid8 AND seq > $2 AND (family_id = $3 OR user_id = $4)
          ORDER BY seq LIMIT $5",
    )
    .bind(cursor.from)
    .bind(cursor.seq)
    .bind(viewer.family_id)
    .bind(viewer.user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: read changes")?;
    let has_more = rows.len() as i64 == limit;
    let new_cursor = if has_more {
        Cursor { from: cursor.from, seq: rows.last().map_or(cursor.seq, |r| r.0), next, at: cursor.at }
    } else {
        Cursor { from: next, seq: 0, next, at }
    };
    let mut seen = std::collections::HashSet::new();
    let items = rows.into_iter().filter(|(_, k, id)| seen.insert((k.clone(), *id))).map(|(_, k, id)| (k, id)).collect();
    Ok((items, new_cursor, has_more))
}

/// Log rows older than the retention, by the daily sweep.
pub async fn prune(pool: &PgPool) -> anyhow::Result<u64> {
    Ok(sqlx::query("DELETE FROM changes WHERE changed_at < now() - make_interval(days => $1::int)")
        .bind(RETENTION_DAYS as i32)
        .execute(pool)
        .await
        .context("db: prune changes")?
        .rows_affected())
}
