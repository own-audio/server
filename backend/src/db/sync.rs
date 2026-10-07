// SPDX-License-Identifier: AGPL-3.0-or-later
/// Mobile-sync primitives: play queue, delete tombstones, push registration.
/// See migration 0021. Nothing here is platform-specific — iOS and Android
/// use the same endpoints, differing only in `platform`/`device_kind` values.
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

// ── Play queue ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlayQueue {
    pub items: Value,
    pub current_index: i32,
    pub position_secs: f64,
    pub updated_by_device: Option<String>,
    pub updated_by_device_label: Option<String>,
    pub updated_at: DateTime<Utc>,
}

pub async fn get_queue(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Option<PlayQueue>> {
    sqlx::query_as::<_, PlayQueue>(
        "SELECT items, current_index, position_secs, updated_by_device,
                updated_by_device_label, updated_at
         FROM play_queues WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: get play queue")
}

/// Replace the queue wholesale. Last write wins; the response echoes
/// `updated_at` so a client can detect that another device got there first.
pub async fn put_queue(
    pool: &PgPool,
    user_id: Uuid,
    items: &Value,
    current_index: i32,
    position_secs: f64,
    device: Option<&str>,
    device_label: Option<&str>,
) -> anyhow::Result<PlayQueue> {
    sqlx::query_as::<_, PlayQueue>(
        "INSERT INTO play_queues
             (user_id, items, current_index, position_secs, updated_by_device,
              updated_by_device_label, updated_at)
         VALUES ($1, $2, $3, $4, $5, $6, now())
         ON CONFLICT (user_id) DO UPDATE
             SET items                   = EXCLUDED.items,
                 current_index           = EXCLUDED.current_index,
                 position_secs           = EXCLUDED.position_secs,
                 updated_by_device       = EXCLUDED.updated_by_device,
                 updated_by_device_label = EXCLUDED.updated_by_device_label,
                 updated_at              = now()
         RETURNING items, current_index, position_secs, updated_by_device,
                   updated_by_device_label, updated_at",
    )
    .bind(user_id)
    .bind(items)
    .bind(current_index)
    .bind(position_secs)
    .bind(device)
    .bind(device_label)
    .fetch_one(pool)
    .await
    .context("db: put play queue")
}

// ── Delta sync ────────────────────────────────────────────────────────────
//
// Rows changed since a timestamp, scoped by the same visibility rule as the
// full listings. A client stores the `now` it was handed and passes it back
// as `since` next time, so each sync transfers only what moved.

use crate::audiobooks::models::AudiobookBook;
use crate::db::access::{AUDIOBOOK, MUSIC, PODCAST, VISIBLE, Viewer};
use crate::music::models::MusicTrack;
use crate::podcasts::models::PodcastFeed;

macro_rules! bind_viewer {
    ($q:expr, $viewer:expr, $kind:expr) => {
        $q.bind($viewer.user_id)
            .bind($viewer.family_id)
            .bind($viewer.is_family_admin)
            .bind($kind)
    };
}

pub async fn changed_books(
    pool: &PgPool,
    viewer: Viewer,
    since: DateTime<Utc>,
) -> anyhow::Result<Vec<AudiobookBook>> {
    let cols = crate::db::audiobooks::BOOK_COLS;
    let sql = format!(
        "SELECT {cols} FROM audiobook_books t
         WHERE {VISIBLE} AND t.updated_at > $5
         ORDER BY t.updated_at"
    );

    bind_viewer!(sqlx::query_as::<_, AudiobookBook>(&sql), viewer, AUDIOBOOK)
        .bind(since)
        .fetch_all(pool)
        .await
        .context("db: changed audiobooks")
}

pub async fn changed_feeds(
    pool: &PgPool,
    viewer: Viewer,
    since: DateTime<Utc>,
) -> anyhow::Result<Vec<PodcastFeed>> {
    let cols = crate::db::podcasts::FEED_COLS;
    let sql = format!(
        "SELECT {cols} FROM podcast_feeds t
         WHERE {VISIBLE} AND t.updated_at > $5
         ORDER BY t.updated_at"
    );

    bind_viewer!(sqlx::query_as::<_, PodcastFeed>(&sql), viewer, PODCAST)
        .bind(since)
        .fetch_all(pool)
        .await
        .context("db: changed podcast feeds")
}

/// Streamed into `out`: a full sync is the whole catalog.
pub async fn send_changed_tracks(
    pool: PgPool,
    viewer: Viewer,
    since: DateTime<Utc>,
    out: tokio::sync::mpsc::Sender<anyhow::Result<MusicTrack>>,
) {
    let cols = crate::db::music::TRACK_COLS;
    let sql = format!(
        "SELECT {cols} FROM music_tracks t
         WHERE {VISIBLE} AND t.updated_at > $5
         ORDER BY t.updated_at"
    );
    let rows = bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer, MUSIC).bind(since).fetch(&pool);
    crate::http::json_stream::send_all(rows, out, "db: changed music tracks").await;
}

// ── Tombstones ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Tombstone {
    pub media_kind: String,
    pub item_id: Uuid,
    pub deleted_at: DateTime<Utc>,
}

/// Record that an item was deleted, so delta-syncing clients can purge it.
/// Best-effort by design: failing to write a tombstone must never fail the
/// delete itself.
pub async fn record_deletion(
    pool: &PgPool,
    media_kind: &str,
    item_id: Uuid,
    user_id: Uuid,
    family_id: Option<Uuid>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO deleted_items (media_kind, item_id, user_id, family_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(media_kind)
    .bind(item_id)
    .bind(user_id)
    .bind(family_id)
    .execute(pool)
    .await
    .context("db: record deletion")?;
    Ok(())
}

/// Deletions this user needs to know about since `since`: their own, plus
/// anything that was shared with their family when it went away.
pub async fn deletions_since(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Uuid,
    since: DateTime<Utc>,
) -> anyhow::Result<Vec<Tombstone>> {
    sqlx::query_as::<_, Tombstone>(
        "SELECT media_kind, item_id, deleted_at
         FROM deleted_items
         WHERE deleted_at > $3 AND (user_id = $1 OR family_id = $2)
         ORDER BY deleted_at",
    )
    .bind(user_id)
    .bind(family_id)
    .bind(since)
    .fetch_all(pool)
    .await
    .context("db: deletions since")
}

/// Drop tombstones older than `days`. A client offline for longer than that
/// must do a full resync; keeping them forever would grow without bound.
pub async fn prune_tombstones(pool: &PgPool, days: i32) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "DELETE FROM deleted_items WHERE deleted_at < now() - make_interval(days => $1)",
    )
    .bind(days)
    .execute(pool)
    .await
    .context("db: prune tombstones")?;

    Ok(result.rows_affected())
}

// ── Push registration ─────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PushToken {
    pub id: Uuid,
    pub platform: String,
    pub token: String,
    pub device_name: Option<String>,
    pub last_seen_at: DateTime<Utc>,
}

/// Register (or refresh) a push token. Re-registering an existing token
/// re-points it at the current user: handing a phone to someone else must not
/// keep delivering the previous owner's notifications.
pub async fn register_push_token(
    pool: &PgPool,
    user_id: Uuid,
    platform: &str,
    token: &str,
    device_name: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO device_push_tokens (user_id, platform, token, device_name)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (token) DO UPDATE
             SET user_id      = EXCLUDED.user_id,
                 platform     = EXCLUDED.platform,
                 device_name  = EXCLUDED.device_name,
                 last_seen_at = now()",
    )
    .bind(user_id)
    .bind(platform)
    .bind(token)
    .bind(device_name)
    .execute(pool)
    .await
    .context("db: register push token")?;
    Ok(())
}

pub async fn delete_push_token(pool: &PgPool, user_id: Uuid, token: &str) -> anyhow::Result<bool> {
    let result = sqlx::query("DELETE FROM device_push_tokens WHERE user_id = $1 AND token = $2")
        .bind(user_id)
        .bind(token)
        .execute(pool)
        .await
        .context("db: delete push token")?;

    Ok(result.rows_affected() > 0)
}

pub async fn list_push_tokens(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<PushToken>> {
    sqlx::query_as::<_, PushToken>(
        "SELECT id, platform, token, device_name, last_seen_at
         FROM device_push_tokens WHERE user_id = $1 ORDER BY last_seen_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list push tokens")
}

// ── Notification queue ────────────────────────────────────────────────────

/// Queue a notification. Delivery is a separate concern: a trigger such as
/// "feed refresh found new episodes" should never block on a push provider.
pub async fn queue_notification(
    pool: &PgPool,
    user_id: Uuid,
    kind: &str,
    title: &str,
    body: Option<&str>,
    data: Option<&Value>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO pending_notifications (user_id, kind, title, body, data)
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(user_id)
    .bind(kind)
    .bind(title)
    .bind(body)
    .bind(data)
    .execute(pool)
    .await
    .context("db: queue notification")?;
    Ok(())
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PendingNotification {
    pub id: Uuid,
    pub user_id: Uuid,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub data: Option<Value>,
    pub created_at: DateTime<Utc>,
}

/// Undelivered notifications for one user, newest first.
///
/// This doubles as the **polling fallback**: until a deployment configures
/// APNs/FCM credentials, clients can fetch pending notifications on resume
/// and still surface "new episode" badges without any push transport.
pub async fn pending_for_user(
    pool: &PgPool,
    user_id: Uuid,
    limit: i64,
) -> anyhow::Result<Vec<PendingNotification>> {
    sqlx::query_as::<_, PendingNotification>(
        "SELECT id, user_id, kind, title, body, data, created_at
         FROM pending_notifications
         WHERE user_id = $1 AND delivered_at IS NULL
         ORDER BY created_at DESC
         LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: pending notifications")
}

pub async fn mark_notifications_delivered(
    pool: &PgPool,
    user_id: Uuid,
    ids: &[Uuid],
) -> anyhow::Result<u64> {
    if ids.is_empty() {
        return Ok(0);
    }

    let result = sqlx::query(
        "UPDATE pending_notifications SET delivered_at = now()
         WHERE user_id = $1 AND id = ANY($2) AND delivered_at IS NULL",
    )
    .bind(user_id)
    .bind(ids)
    .execute(pool)
    .await
    .context("db: mark notifications delivered")?;

    Ok(result.rows_affected())
}
