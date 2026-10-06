// SPDX-License-Identifier: AGPL-3.0-or-later
/// Which of a user's devices hold which items offline — docs/file-sync-plan.md
/// §5.9. Groundwork for "where is this file": the Mac reports from day one,
/// the UI comes later. A user only ever sees their own devices.
use super::paths::Kind;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

#[derive(Debug, Clone, Deserialize)]
pub struct HoldingKey {
    pub kind: String,
    pub id: Uuid,
}

/// `PUT /sync/holdings` body: the device's whole set, or a change to it.
#[derive(Debug, Deserialize)]
pub struct HoldingsUpdate {
    #[serde(default)]
    pub items: Option<Vec<HoldingKey>>,
    #[serde(default)]
    pub added: Vec<HoldingKey>,
    #[serde(default)]
    pub removed: Vec<HoldingKey>,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct HoldingDevice {
    pub chain_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: Option<String>,
    pub since: DateTime<Utc>,
    #[sqlx(skip)]
    pub current: bool,
}

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct Holding {
    pub kind: String,
    pub id: Uuid,
    pub since: DateTime<Utc>,
}

fn split(keys: &[HoldingKey]) -> Result<(Vec<String>, Vec<Uuid>), String> {
    let mut kinds = Vec::with_capacity(keys.len());
    let mut ids = Vec::with_capacity(keys.len());
    for k in keys {
        if Kind::parse(&k.kind).is_none() {
            return Err(format!("unknown kind '{}'", k.kind));
        }
        kinds.push(k.kind.clone());
        ids.push(k.id);
    }
    Ok((kinds, ids))
}

/// Apply an update from the device `chain_id`. `Err` names a bad kind.
pub async fn update(pool: &PgPool, user_id: Uuid, chain_id: Uuid, body: HoldingsUpdate) -> anyhow::Result<Result<(), String>> {
    let mut tx = pool.begin().await.context("db: begin holdings")?;
    if let Some(items) = &body.items {
        let (kinds, ids) = match split(items) {
            Ok(v) => v,
            Err(e) => return Ok(Err(e)),
        };
        // Rows that stay keep their `since`.
        sqlx::query(
            "DELETE FROM device_holdings h
              WHERE h.chain_id = $1 AND h.user_id = $2
                AND NOT EXISTS (SELECT 1 FROM unnest($3::text[], $4::uuid[]) AS w(kind, id)
                                 WHERE w.kind = h.kind AND w.id = h.item_id)",
        )
        .bind(chain_id)
        .bind(user_id)
        .bind(&kinds)
        .bind(&ids)
        .execute(&mut *tx)
        .await
        .context("db: replace holdings")?;
        insert(&mut tx, user_id, chain_id, &kinds, &ids).await?;
    }
    if !body.added.is_empty() {
        let (kinds, ids) = match split(&body.added) {
            Ok(v) => v,
            Err(e) => return Ok(Err(e)),
        };
        insert(&mut tx, user_id, chain_id, &kinds, &ids).await?;
    }
    if !body.removed.is_empty() {
        let (kinds, ids) = match split(&body.removed) {
            Ok(v) => v,
            Err(e) => return Ok(Err(e)),
        };
        sqlx::query(
            "DELETE FROM device_holdings h
              USING unnest($3::text[], $4::uuid[]) AS w(kind, id)
              WHERE h.chain_id = $1 AND h.user_id = $2 AND h.kind = w.kind AND h.item_id = w.id",
        )
        .bind(chain_id)
        .bind(user_id)
        .bind(&kinds)
        .bind(&ids)
        .execute(&mut *tx)
        .await
        .context("db: remove holdings")?;
    }
    tx.commit().await.context("db: commit holdings")?;
    Ok(Ok(()))
}

async fn insert(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    user_id: Uuid,
    chain_id: Uuid,
    kinds: &[String],
    ids: &[Uuid],
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO device_holdings (chain_id, user_id, kind, item_id)
         SELECT $1, $2, w.kind, w.id FROM unnest($3::text[], $4::uuid[]) AS w(kind, id)
         ON CONFLICT DO NOTHING",
    )
    .bind(chain_id)
    .bind(user_id)
    .bind(kinds)
    .bind(ids)
    .execute(&mut **tx)
    .await
    .context("db: add holdings")?;
    Ok(())
}

/// The caller's own signed-in devices that hold this item.
pub async fn devices_holding(pool: &PgPool, user_id: Uuid, kind: &str, id: Uuid) -> anyhow::Result<Vec<HoldingDevice>> {
    sqlx::query_as::<_, HoldingDevice>(
        "SELECT h.chain_id, d.device_name, d.device_kind, h.since
           FROM device_holdings h
           JOIN LATERAL (SELECT rt.device_name, rt.device_kind FROM refresh_tokens rt
                          WHERE rt.chain_id = h.chain_id AND rt.user_id = h.user_id
                            AND rt.revoked_at IS NULL AND rt.expires_at > now()
                          ORDER BY rt.created_at DESC LIMIT 1) d ON TRUE
          WHERE h.user_id = $1 AND h.kind = $2 AND h.item_id = $3
          ORDER BY h.since",
    )
    .bind(user_id)
    .bind(kind)
    .bind(id)
    .fetch_all(pool)
    .await
    .context("db: devices holding item")
}

/// What this device has reported holding.
pub async fn held_by(pool: &PgPool, user_id: Uuid, chain_id: Uuid) -> anyhow::Result<Vec<Holding>> {
    sqlx::query_as::<_, Holding>(
        "SELECT kind, item_id AS id, since FROM device_holdings
          WHERE user_id = $1 AND chain_id = $2 ORDER BY kind, item_id",
    )
    .bind(user_id)
    .bind(chain_id)
    .fetch_all(pool)
    .await
    .context("db: holdings of device")
}

/// A purged item is held nowhere.
pub async fn forget_item(pool: &PgPool, kind: &str, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM device_holdings WHERE kind = $1 AND item_id = $2")
        .bind(kind)
        .bind(id)
        .execute(pool)
        .await
        .context("db: forget holdings of item")?;
    Ok(())
}
