// SPDX-License-Identifier: AGPL-3.0-or-later
//! The gPodder sync API's state (migration 0115): devices, the subscription
//! log, episode actions, and applying an app's play action to the listener's
//! own progress.
use crate::db::access::{PODCAST, VISIBLE, Viewer};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// How far before a client's `since` the logs are read again, so a transaction
/// that committed a moment late is not missed. Actions are idempotent.
const OVERLAP_SECS: i64 = 5;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Device {
    pub device_id: String,
    pub caption: String,
    pub kind: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Action {
    pub podcast_url: String,
    pub episode_url: String,
    pub guid: Option<String>,
    pub action: String,
    pub device_id: Option<String>,
    pub happened_at: DateTime<Utc>,
    pub started: Option<i32>,
    pub position: Option<i32>,
    pub total: Option<i32>,
}

/// The server's clock in unix seconds: the `timestamp` every answer carries.
pub async fn now_secs(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT floor(extract(epoch FROM now()))::bigint")
        .fetch_one(pool)
        .await
        .context("db: gpodder clock")
}

fn since_time(since: i64) -> DateTime<Utc> {
    DateTime::from_timestamp(since.saturating_sub(OVERLAP_SECS).max(0), 0).unwrap_or_default()
}

// ── Devices ──────────────────────────────────────────────────────────────

pub async fn upsert_device(pool: &PgPool, user_id: Uuid, device_id: &str, caption: Option<&str>, kind: Option<&str>) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO gpodder_devices (user_id, device_id, caption, kind)
         VALUES ($1, $2, COALESCE($3, ''), COALESCE($4, 'other'))
         ON CONFLICT (user_id, device_id) DO UPDATE
            SET caption = COALESCE($3, gpodder_devices.caption),
                kind = COALESCE($4, gpodder_devices.kind),
                updated_at = now()",
    )
    .bind(user_id)
    .bind(device_id)
    .bind(caption)
    .bind(kind)
    .execute(pool)
    .await
    .context("db: upsert gpodder device")?;
    Ok(())
}

pub async fn list_devices(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<Device>> {
    sqlx::query_as("SELECT device_id, caption, kind FROM gpodder_devices WHERE user_id = $1 ORDER BY device_id")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .context("db: list gpodder devices")
}

// ── Subscriptions ────────────────────────────────────────────────────────

/// The RSS feeds the listener owns, by URL, with their ids.
pub async fn owned_feeds(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<(Uuid, String)>> {
    sqlx::query_as("SELECT id, feed_url FROM podcast_feeds WHERE user_id = $1 AND source_type = 'rss' ORDER BY feed_url")
        .bind(user_id)
        .fetch_all(pool)
        .await
        .context("db: owned rss feeds")
}

/// Whether a feed at `url` is already visible to the viewer: their own, or
/// one their family shares with them. A shared show is not copied.
pub async fn visible_feed_exists(pool: &PgPool, viewer: Viewer, url: &str) -> anyhow::Result<bool> {
    let sql = format!("SELECT EXISTS (SELECT 1 FROM podcast_feeds t WHERE t.feed_url = $5 AND {VISIBLE})");
    sqlx::query_scalar(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(PODCAST)
        .bind(url)
        .fetch_one(pool)
        .await
        .context("db: visible feed by url")
}

/// The last change to each feed URL since `since`: `(url, added)`. A change
/// `device` made itself is left out: it knows.
pub async fn subscription_changes(pool: &PgPool, user_id: Uuid, since: i64, device: &str) -> anyhow::Result<Vec<(String, bool)>> {
    sqlx::query_as(
        "SELECT feed_url, added FROM (
            SELECT DISTINCT ON (feed_url) feed_url, added, device_id FROM gpodder_subscription_changes
             WHERE user_id = $1 AND changed_at >= $2
             ORDER BY feed_url, id DESC
         ) last WHERE device_id IS DISTINCT FROM $3",
    )
    .bind(user_id)
    .bind(since_time(since))
    .bind(device)
    .fetch_all(pool)
    .await
    .context("db: gpodder subscription changes")
}

/// An app's subscription changes in one transaction, logged as that app's:
/// new feeds for `add` (private to the listener, the URL as their title until
/// the first refresh reads the real one), and `remove` (feed ids the listener
/// owns) deleted. Returns the new feeds' ids.
pub async fn apply_subscriptions(pool: &PgPool, user_id: Uuid, device: &str, add: &[String], remove: &[Uuid]) -> anyhow::Result<Vec<Uuid>> {
    let mut tx = pool.begin().await.context("db: begin gpodder subscriptions")?;
    sqlx::query("SELECT set_config('own_audio.gpodder_device', $1, true)")
        .bind(device)
        .execute(&mut *tx)
        .await
        .context("db: name gpodder device")?;
    let mut created = Vec::with_capacity(add.len());
    for url in add {
        let id: Uuid = sqlx::query_scalar("INSERT INTO podcast_feeds (user_id, family_id, feed_url, title) VALUES ($1, NULL, $2, $2) RETURNING id")
            .bind(user_id)
            .bind(url)
            .fetch_one(&mut *tx)
            .await
            .context("db: insert gpodder feed")?;
        created.push(id);
    }
    if !remove.is_empty() {
        sqlx::query("DELETE FROM podcast_feeds WHERE user_id = $1 AND id = ANY($2)")
            .bind(user_id)
            .bind(remove)
            .execute(&mut *tx)
            .await
            .context("db: delete gpodder feeds")?;
    }
    tx.commit().await.context("db: commit gpodder subscriptions")?;
    Ok(created)
}

// ── Episode actions ──────────────────────────────────────────────────────

pub async fn insert_action(pool: &PgPool, user_id: Uuid, a: &Action) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO gpodder_episode_actions
            (user_id, podcast_url, episode_url, guid, action, device_id, happened_at, started, position, total)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
    )
    .bind(user_id)
    .bind(&a.podcast_url)
    .bind(&a.episode_url)
    .bind(&a.guid)
    .bind(&a.action)
    .bind(&a.device_id)
    .bind(a.happened_at)
    .bind(a.started)
    .bind(a.position)
    .bind(a.total)
    .execute(pool)
    .await
    .context("db: insert gpodder action")?;
    Ok(())
}

/// Actions received since `since`, oldest first, optionally for one podcast
/// or from one device.
pub async fn list_actions(pool: &PgPool, user_id: Uuid, since: i64, podcast: Option<&str>, device: Option<&str>) -> anyhow::Result<Vec<Action>> {
    sqlx::query_as(
        "SELECT podcast_url, episode_url, guid, action, device_id, happened_at, started, position, total
           FROM gpodder_episode_actions
          WHERE user_id = $1 AND received_at >= $2
            AND ($3::text IS NULL OR podcast_url = $3)
            AND ($4::text IS NULL OR device_id = $4)
          ORDER BY received_at, id",
    )
    .bind(user_id)
    .bind(since_time(since))
    .bind(podcast)
    .bind(device)
    .fetch_all(pool)
    .await
    .context("db: list gpodder actions")
}

/// An app's play (or `new`) action as the listener's own progress in the
/// episode it names, when a feed the listener can see has that episode, and
/// only when it is newer than what is kept. Logged as the app sent it, not a
/// second time by the progress trigger. Returns whether it was applied.
#[allow(clippy::too_many_arguments)]
pub async fn apply_progress(
    pool: &PgPool,
    viewer: Viewer,
    podcast_url: &str,
    episode_url: &str,
    guid: Option<&str>,
    position_secs: f64,
    completed: bool,
    at: DateTime<Utc>,
) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin gpodder apply")?;
    sqlx::query("SELECT set_config('own_audio.gpodder_apply', 'on', true)")
        .execute(&mut *tx)
        .await
        .context("db: mark gpodder apply")?;
    let sql = format!(
        "SELECT e.id FROM podcast_episodes e JOIN podcast_feeds t ON t.id = e.feed_id
          WHERE t.feed_url = $5 AND (e.audio_url = $6 OR ($7::text IS NOT NULL AND e.guid = $7))
            AND e.trashed_at IS NULL AND {VISIBLE}
          ORDER BY (t.user_id = $1) DESC, (e.audio_url = $6) DESC
          LIMIT 1"
    );
    let episode: Option<Uuid> = sqlx::query_scalar(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(PODCAST)
        .bind(podcast_url)
        .bind(episode_url)
        .bind(guid)
        .fetch_optional(&mut *tx)
        .await
        .context("db: gpodder episode by url")?;
    let Some(episode_id) = episode else {
        tx.commit().await.context("db: commit gpodder apply")?;
        return Ok(false);
    };
    let applied = sqlx::query(
        "INSERT INTO podcast_progress (user_id, episode_id, position_secs, completed, updated_at)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, episode_id) DO UPDATE
            SET position_secs = EXCLUDED.position_secs, completed = EXCLUDED.completed, updated_at = EXCLUDED.updated_at
          WHERE podcast_progress.updated_at < EXCLUDED.updated_at",
    )
    .bind(viewer.user_id)
    .bind(episode_id)
    .bind(position_secs)
    .bind(completed)
    .bind(at)
    .execute(&mut *tx)
    .await
    .context("db: gpodder progress")?
    .rows_affected()
        == 1;
    tx.commit().await.context("db: commit gpodder apply")?;
    Ok(applied)
}
