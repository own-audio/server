// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener's queue, player settings, notification inbox and push devices
//! (docs/api-v2-completion-plan.md, phase 8).
//!
//! The queue, the settings and each notification are the listener's own state in the change
//! feed (`queue`, `settings`, `notification`; migration 0130), so another device learns of a
//! change from `/sync/changes` and a `library` event on `/events` instead of polling. They are
//! v1's rows: a queue written here plays on a v1 or Subsonic client and back.

use crate::app::AppState;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::db::user_state::Kind;
use crate::families::FamilyContext;
use crate::playback::models::{EqSettings, UserSettings};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// ── The queue ────────────────────────────────────────────────────────────────

/// The most a queue holds (v1's).
pub const QUEUE_MAX: usize = 1000;

#[derive(Serialize, Deserialize, ToSchema, Clone, Debug, PartialEq)]
pub struct QueueItem {
    /// `track`, `book` or `episode`.
    pub kind: String,
    pub id: Uuid,
    /// A book's: the file to start in.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_id: Option<Uuid>,
}

/// What is playing and what comes next, the same on every device.
#[derive(Serialize, ToSchema)]
pub struct Queue {
    pub items: Vec<QueueItem>,
    /// The item playing, from 0.
    pub index: i32,
    /// Where in it, in milliseconds.
    pub position_ms: i64,
    /// Up by one with every write, from any device or API; `0` before the first. Send it back
    /// as `If-Match` so a write made against an older queue is refused. Also the `ETag`.
    pub revision: i64,
    /// The device that wrote last (as in `GET /me/sessions`), so another can say "playing on
    /// the kitchen speaker".
    pub device_kind: Option<String>,
    pub device_name: Option<String>,
    /// Absent before the first write.
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize, ToSchema)]
pub struct QueueRequest {
    /// Up to 1000, in play order.
    pub items: Vec<QueueItem>,
    /// The item playing; kept inside the list.
    #[serde(default)]
    pub index: i32,
    #[serde(default)]
    pub position_ms: i64,
}

/// v1's stored item (`{media_kind, item_id, part_id}`) as v2's; an episode is stored by its
/// show with the episode as the part.
fn from_stored(item: &Value) -> Option<QueueItem> {
    let id = |key: &str| item.get(key).and_then(Value::as_str).and_then(|s| s.parse::<Uuid>().ok());
    match item.get("media_kind").and_then(Value::as_str)? {
        "music" => Some(QueueItem { kind: "track".into(), id: id("item_id")?, file_id: None }),
        "audiobook" => Some(QueueItem { kind: "book".into(), id: id("item_id")?, file_id: id("part_id") }),
        "podcast" => Some(QueueItem { kind: "episode".into(), id: id("part_id")?, file_id: None }),
        _ => None,
    }
}

/// The stored queue as v2 answers it: items v2 cannot name (a show without an episode) are
/// left out and the index follows its item.
fn queue_of(row: Option<db::sync::PlayQueue>) -> Queue {
    let Some(row) = row else {
        return Queue { items: Vec::new(), index: 0, position_ms: 0, revision: 0, device_kind: None, device_name: None, updated_at: None };
    };
    let stored = row.items.as_array().cloned().unwrap_or_default();
    let mut items = Vec::with_capacity(stored.len());
    let mut index = 0;
    for (i, raw) in stored.iter().enumerate() {
        if let Some(item) = from_stored(raw) {
            if (i as i32) <= row.current_index {
                index = items.len() as i32;
            }
            items.push(item);
        }
    }
    Queue {
        items,
        index,
        position_ms: (row.position_secs * 1000.0).round() as i64,
        revision: row.revision,
        device_kind: row.updated_by_device,
        device_name: row.updated_by_device_label,
        updated_at: Some(row.updated_at),
    }
}

/// What `If-Match` asks for: `None` when absent or `*` (any queue), else the revision named,
/// `Some(-1)` for one that is not a revision (it never matches).
fn if_match(headers: &HeaderMap) -> Option<i64> {
    let raw = headers.get(header::IF_MATCH)?.to_str().unwrap_or("").trim();
    if raw == "*" {
        return None;
    }
    let tag = raw.strip_prefix("W/").unwrap_or(raw).trim_matches('"');
    Some(tag.parse::<i64>().ok().filter(|n| *n >= 0).unwrap_or(-1))
}

/// The request's own checks: the size, the kinds, a file only on a book.
fn check_queue(body: &QueueRequest) -> Result<(), Problem> {
    if body.items.len() > QUEUE_MAX {
        return Err(Problem::bad_request("too_many", format!("a queue holds at most {QUEUE_MAX} items")).field("/items", "too_many", QUEUE_MAX.to_string()));
    }
    for (i, item) in body.items.iter().enumerate() {
        if !matches!(item.kind.as_str(), "track" | "book" | "episode") {
            return Err(Problem::bad_request("invalid_kind", format!("`{}` cannot be queued", item.kind))
                .field(format!("/items/{i}/kind"), "invalid", "track, book or episode"));
        }
        if item.file_id.is_some() && item.kind != "book" {
            return Err(Problem::bad_request("invalid_body", "only a book's item names a file").field(format!("/items/{i}/file_id"), "invalid", "books only"));
        }
    }
    Ok(())
}

fn with_etag(queue: Queue) -> Response {
    let tag = HeaderValue::from_str(&format!("\"{}\"", queue.revision)).unwrap_or(HeaderValue::from_static("\"0\""));
    let mut response = Json(queue).into_response();
    response.headers_mut().insert(header::ETAG, tag);
    response
}

/// The caller's queue; an empty one (revision 0) before the first write.
#[utoipa::path(get, path = "/me/queue", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, description = "The queue, with its revision as the `ETag`", body = Queue)))]
pub async fn get_queue(family: FamilyContext, State(state): State<AppState>) -> Result<Response, Problem> {
    Ok(with_etag(queue_of(db::sync::get_queue(state.db(), family.user_id).await?)))
}

/// Replace the queue. With `If-Match: "<revision>"` (or the bare number) the write is made only
/// if the queue is still at that revision: another device's newer queue answers `412` with
/// the current `revision`, to read and merge. Without it the last write wins, as on v1.
#[utoipa::path(put, path = "/me/queue", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    params(("If-Match" = Option<String>, Header, description = "The revision this queue was made from; `0` when there was none")),
    request_body = QueueRequest,
    responses((status = 200, description = "The queue as stored, with its new revision", body = Queue),
        (status = 400, description = "`too_many`, `invalid_kind`, `invalid_item` (not visible to you), `invalid_body`", body = crate::v2::problem::ProblemBody),
        (status = 412, description = "`revision_mismatch`: the queue moved on; the problem's `revision` is the current one", body = crate::v2::problem::ProblemBody)))]
pub async fn put_queue(
    auth: AuthUser,
    family: FamilyContext,
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<QueueRequest>,
) -> Result<Response, Problem> {
    check_queue(&body)?;
    let pool = state.db();
    let viewer = family.viewer();
    let ids = |kind: &str| -> Vec<Uuid> { body.items.iter().filter(|i| i.kind == kind).map(|i| i.id).collect() };
    let tracks: HashSet<Uuid> = db::user_state::visible_ids(pool, viewer, Kind::Track, &ids("track")).await?.into_iter().collect();
    let books: HashSet<Uuid> = db::user_state::visible_ids(pool, viewer, Kind::Book, &ids("book")).await?.into_iter().collect();
    let feeds: HashMap<Uuid, Uuid> = db::sync::visible_episode_feeds(pool, viewer, &ids("episode")).await?.into_iter().collect();
    let files: Vec<Uuid> = body.items.iter().filter_map(|i| i.file_id).collect();
    let file_books: HashMap<Uuid, Uuid> = db::sync::file_books(pool, &files).await?.into_iter().collect();
    let mut stored = Vec::with_capacity(body.items.len());
    for (i, item) in body.items.iter().enumerate() {
        let unknown = || Problem::bad_request("invalid_item", format!("{} {} is not one you can play", item.kind, item.id)).field(format!("/items/{i}/id"), "not_found", item.id.to_string());
        stored.push(match item.kind.as_str() {
            "track" if tracks.contains(&item.id) => json!({ "media_kind": "music", "item_id": item.id }),
            "book" if books.contains(&item.id) => {
                if let Some(file) = item.file_id.filter(|f| file_books.get(f) != Some(&item.id)) {
                    return Err(Problem::bad_request("invalid_item", "file_id is not a file of this book").field(format!("/items/{i}/file_id"), "invalid", file.to_string()));
                }
                match item.file_id {
                    Some(file) => json!({ "media_kind": "audiobook", "item_id": item.id, "part_id": file }),
                    None => json!({ "media_kind": "audiobook", "item_id": item.id }),
                }
            }
            "episode" => match feeds.get(&item.id) {
                Some(feed) => json!({ "media_kind": "podcast", "item_id": feed, "part_id": item.id }),
                None => return Err(unknown()),
            },
            _ => return Err(unknown()),
        });
    }
    // Clamped, as v1: a queue trimmed on another device can leave an index past its end.
    let index = body.index.clamp(0, (stored.len() as i32 - 1).max(0));
    let position_secs = body.position_ms.max(0) as f64 / 1000.0;
    let (device_name, device_kind) = crate::v2::state::session_device(&state, &auth).await?;
    let items = Value::Array(stored);
    let saved = match if_match(&headers) {
        None => Some(db::sync::put_queue(pool, family.user_id, &items, index, position_secs, Some(&device_kind), device_name.as_deref()).await?),
        Some(expected) if expected < 0 => None,
        Some(expected) => db::sync::put_queue_if(pool, family.user_id, expected, &items, index, position_secs, Some(&device_kind), device_name.as_deref()).await?,
    };
    match saved {
        Some(row) => Ok(with_etag(queue_of(Some(row)))),
        None => {
            let current = db::sync::get_queue(pool, family.user_id).await?.map_or(0, |q| q.revision);
            Err(Problem::new(StatusCode::PRECONDITION_FAILED, "revision_mismatch")
                .detail("the queue changed on another device: read it again")
                .with("revision", current))
        }
    }
}

/// The queue as the change feed and the snapshot carry it.
pub(crate) async fn queue_record(state: &AppState, user_id: Uuid) -> Result<Value, Problem> {
    serde_json::to_value(queue_of(db::sync::get_queue(state.db(), user_id).await?)).map_err(|e| Problem::internal(anyhow::anyhow!(e)))
}

// ── Settings ─────────────────────────────────────────────────────────────────

/// The player's settings on every device. Times are milliseconds, kept to the second.
#[derive(Serialize, ToSchema)]
pub struct Settings {
    pub playback: PlaybackSettings,
    pub audiobook: AudiobookSettings,
    /// The equaliser curves saved, one per device kind; a kind without one plays flat.
    pub eq: Vec<EqCurve>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct PlaybackSettings {
    /// 0.5 to 3.0.
    pub speed: f64,
    /// Skipped at an episode's start and cut from its end; 0 to 600000.
    pub skip_intro_ms: i64,
    pub skip_outro_ms: i64,
}

#[derive(Serialize, ToSchema)]
pub struct AudiobookSettings {
    /// 0.5 to 3.0.
    pub speed: f64,
    /// The skip buttons' steps; 1000 to 120000.
    pub skip_back_ms: i64,
    pub skip_forward_ms: i64,
}

#[derive(Serialize, ToSchema)]
pub struct EqCurve {
    /// `web`, `ios`, `android`, `macos`, `windows`, `tvos` or `other`.
    pub device_kind: String,
    pub enabled: bool,
    /// −12 to 12 dB.
    pub preamp_db: f64,
    /// Six gains, −12 to 12 dB, at 60 Hz, 150 Hz, 400 Hz, 1 kHz, 2.4 kHz and 15 kHz.
    pub bands_db: Vec<f64>,
    pub preset_name: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema, Default)]
pub struct SettingsPatch {
    pub playback: Option<PlaybackPatch>,
    pub audiobook: Option<AudiobookPatch>,
    /// One device kind's curve; the caller's own device kind when it names none.
    pub eq: Option<EqPatch>,
}

#[derive(Deserialize, ToSchema, Default)]
pub struct PlaybackPatch {
    pub speed: Option<f64>,
    pub skip_intro_ms: Option<i64>,
    pub skip_outro_ms: Option<i64>,
}

#[derive(Deserialize, ToSchema, Default)]
pub struct AudiobookPatch {
    pub speed: Option<f64>,
    pub skip_back_ms: Option<i64>,
    pub skip_forward_ms: Option<i64>,
}

#[derive(Deserialize, ToSchema, Default)]
pub struct EqPatch {
    pub device_kind: Option<String>,
    pub enabled: Option<bool>,
    pub preamp_db: Option<f64>,
    /// All six gains.
    pub bands_db: Option<Vec<f64>>,
    /// `null` takes the name away.
    #[serde(default, deserialize_with = "some")]
    #[schema(value_type = Option<String>)]
    pub preset_name: Option<Option<String>>,
}

fn some<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<T>, D::Error> {
    T::deserialize(d).map(Some)
}

pub const EQ_DEVICE_KINDS: &[&str] = &["web", "ios", "android", "macos", "windows", "tvos", "other"];

fn range_f(value: Option<f64>, field: &str, lo: f64, hi: f64) -> Result<(), Problem> {
    match value {
        Some(v) if !(v.is_finite() && (lo..=hi).contains(&v)) => {
            Err(Problem::bad_request("invalid_body", format!("{field} is {lo} to {hi}")).field(field.to_string(), "out_of_range", format!("{lo} to {hi}")))
        }
        _ => Ok(()),
    }
}

fn range_ms(value: Option<i64>, field: &str, lo: i64, hi: i64) -> Result<(), Problem> {
    match value {
        Some(v) if !(lo..=hi).contains(&v) => {
            Err(Problem::bad_request("invalid_body", format!("{field} is {lo} to {hi}")).field(field.to_string(), "out_of_range", format!("{lo} to {hi}")))
        }
        _ => Ok(()),
    }
}

/// Everything a patch says is checked before anything is stored, so a refusal changes nothing.
fn check_settings(p: &SettingsPatch) -> Result<(), Problem> {
    if let Some(pb) = &p.playback {
        range_f(pb.speed, "/playback/speed", 0.5, 3.0)?;
        range_ms(pb.skip_intro_ms, "/playback/skip_intro_ms", 0, 600_000)?;
        range_ms(pb.skip_outro_ms, "/playback/skip_outro_ms", 0, 600_000)?;
    }
    if let Some(ab) = &p.audiobook {
        range_f(ab.speed, "/audiobook/speed", 0.5, 3.0)?;
        range_ms(ab.skip_back_ms, "/audiobook/skip_back_ms", 1000, 120_000)?;
        range_ms(ab.skip_forward_ms, "/audiobook/skip_forward_ms", 1000, 120_000)?;
    }
    if let Some(eq) = &p.eq {
        if let Some(k) = eq.device_kind.as_deref().filter(|k| !EQ_DEVICE_KINDS.contains(k)) {
            return Err(Problem::bad_request("invalid_body", format!("`{k}` is not a device kind")).field("/eq/device_kind", "invalid", EQ_DEVICE_KINDS.join(", ")));
        }
        range_f(eq.preamp_db, "/eq/preamp_db", -12.0, 12.0)?;
        if let Some(bands) = &eq.bands_db {
            if bands.len() != 6 {
                return Err(Problem::bad_request("invalid_body", "bands_db holds six gains").field("/eq/bands_db", "invalid", "six numbers"));
            }
            for (i, b) in bands.iter().enumerate() {
                range_f(Some(*b), &format!("/eq/bands_db/{i}"), -12.0, 12.0)?;
            }
        }
        if eq.preset_name.as_ref().and_then(|n| n.as_ref()).is_some_and(|n| n.chars().count() > 100) {
            return Err(Problem::bad_request("invalid_body", "preset_name is at most 100 characters").field("/eq/preset_name", "too_long", "100 characters"));
        }
    }
    Ok(())
}

/// Milliseconds to the stored seconds, to the nearest.
fn secs(ms: i64) -> i32 {
    i32::try_from((ms + 500) / 1000).unwrap_or(i32::MAX)
}

fn curve(e: EqSettings) -> EqCurve {
    EqCurve {
        device_kind: e.device_kind,
        enabled: e.enabled,
        preamp_db: e.preamp_db,
        bands_db: vec![e.band1_db, e.band2_db, e.band3_db, e.band4_db, e.band5_db, e.band6_db],
        preset_name: e.preset_name,
        updated_at: e.updated_at,
    }
}

fn settings_of(s: UserSettings, eq: Vec<EqSettings>) -> Settings {
    let updated_at = eq.iter().map(|e| e.updated_at).fold(s.updated_at, DateTime::max);
    Settings {
        playback: PlaybackSettings {
            speed: s.playback_speed,
            skip_intro_ms: i64::from(s.skip_intro_secs) * 1000,
            skip_outro_ms: i64::from(s.skip_outro_secs) * 1000,
        },
        audiobook: AudiobookSettings {
            speed: s.ab_playback_speed,
            skip_back_ms: i64::from(s.ab_skip_backward_secs) * 1000,
            skip_forward_ms: i64::from(s.ab_skip_forward_secs) * 1000,
        },
        eq: eq.into_iter().map(curve).collect(),
        updated_at,
    }
}

pub(crate) async fn settings(state: &AppState, user_id: Uuid) -> Result<Settings, Problem> {
    let s = db::playback::get_or_create_settings(state.db(), user_id).await?;
    let eq = db::playback::eq_curves(state.db(), user_id).await?;
    Ok(settings_of(s, eq))
}

/// The settings as the change feed and the snapshot carry them.
pub(crate) async fn settings_record(state: &AppState, user_id: Uuid) -> Result<Value, Problem> {
    serde_json::to_value(settings(state, user_id).await?).map_err(|e| Problem::internal(anyhow::anyhow!(e)))
}

/// The caller's player settings: speeds, skip steps, the equaliser of each device kind.
#[utoipa::path(get, path = "/me/settings", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    responses((status = 200, body = Settings)))]
pub async fn get_settings(family: FamilyContext, State(state): State<AppState>) -> Result<Json<Settings>, Problem> {
    Ok(Json(settings(&state, family.user_id).await?))
}

/// Change some settings; what the patch leaves out stays. Out-of-range values are refused,
/// not clamped (v1 clamps).
#[utoipa::path(patch, path = "/me/settings", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("playback"))),
    request_body = SettingsPatch,
    responses((status = 200, body = Settings), (status = 400, description = "`invalid_body`: a value out of range, an unknown device kind, not six bands", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_settings(
    auth: AuthUser,
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<SettingsPatch>,
) -> Result<Json<Settings>, Problem> {
    check_settings(&body)?;
    let pool = state.db();
    if body.playback.is_some() || body.audiobook.is_some() {
        let now = db::playback::get_or_create_settings(pool, family.user_id).await?;
        let pb = body.playback.unwrap_or_default();
        let ab = body.audiobook.unwrap_or_default();
        db::playback::update_settings(
            pool,
            family.user_id,
            pb.speed.unwrap_or(now.playback_speed),
            pb.skip_intro_ms.map_or(now.skip_intro_secs, secs),
            pb.skip_outro_ms.map_or(now.skip_outro_secs, secs),
            ab.skip_forward_ms.map_or(now.ab_skip_forward_secs, secs),
            ab.skip_back_ms.map_or(now.ab_skip_backward_secs, secs),
            ab.speed.unwrap_or(now.ab_playback_speed),
        )
        .await?;
    }
    if let Some(eq) = body.eq {
        let kind = match eq.device_kind {
            Some(k) => k,
            None => {
                let (_, k) = crate::v2::state::session_device(&state, &auth).await?;
                if EQ_DEVICE_KINDS.contains(&k.as_str()) { k } else { "other".into() }
            }
        };
        let now = db::playback::get_eq_settings(pool, family.user_id, &kind).await?;
        let bands = eq.bands_db.unwrap_or_else(|| vec![now.band1_db, now.band2_db, now.band3_db, now.band4_db, now.band5_db, now.band6_db]);
        let preset = match eq.preset_name {
            Some(name) => name.map(|n| n.trim().to_string()).filter(|n| !n.is_empty()),
            None => now.preset_name,
        };
        db::playback::upsert_eq_settings(
            pool,
            family.user_id,
            &kind,
            eq.enabled.unwrap_or(now.enabled),
            eq.preamp_db.unwrap_or(now.preamp_db),
            bands[0],
            bands[1],
            bands[2],
            bands[3],
            bands[4],
            bands[5],
            preset.as_deref(),
        )
        .await?;
    }
    Ok(Json(settings(&state, family.user_id).await?))
}

// ── Notifications ────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Notification {
    pub id: Uuid,
    /// What happened: `new_episodes`, `item_trashed`, `playlist_shared`, `access_request`; a
    /// client shows the title and body of a kind it does not know.
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    /// What it is about, in v2's words, when it is about one thing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub item: Option<NotificationItem>,
    /// The details as the server stored them.
    #[schema(value_type = Option<Object>)]
    pub data: Option<Value>,
    pub created_at: DateTime<Utc>,
    /// When it was acknowledged; absent while unread.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_at: Option<DateTime<Utc>>,
}

#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct NotificationItem {
    /// `track`, `book`, `episode`, `podcast`, `playlist` or `file`.
    pub kind: String,
    pub id: Uuid,
}

/// The thing a notification's v1 data names, in v2's words.
fn item_of(kind: &str, data: Option<&Value>) -> Option<NotificationItem> {
    let data = data?;
    let id = |key: &str| data.get(key).and_then(Value::as_str).and_then(|s| s.parse::<Uuid>().ok());
    let media = |k: &str| match k {
        "music" => Some("track"),
        "audiobook" => Some("book"),
        "podcast" => Some("podcast"),
        _ => None,
    };
    let (k, id) = match kind {
        "new_episodes" | "access_request" => (media(data.get("media_kind")?.as_str()?)?, id("item_id")?),
        "playlist_shared" => ("playlist", id("playlist_id")?),
        "item_trashed" => {
            let k = match data.get("kind")?.as_str()? {
                "music_track" => "track",
                "audiobook" => "book",
                "playlist" => "playlist",
                "podcast_episode" => "episode",
                "companion_file" => "file",
                _ => return None,
            };
            (k, id("id")?)
        }
        _ => return None,
    };
    Some(NotificationItem { kind: k.into(), id })
}

fn notification(row: db::sync::InboxRow) -> Notification {
    Notification {
        item: item_of(&row.kind, row.data.as_ref()),
        id: row.id,
        kind: row.kind,
        title: row.title,
        body: row.body,
        data: row.data,
        created_at: row.created_at,
        read_at: row.delivered_at,
    }
}

/// These notifications of the listener, as the change feed carries them.
pub(crate) async fn notification_records(state: &AppState, user_id: Uuid, ids: &[Uuid]) -> Result<Vec<(Uuid, Value)>, Problem> {
    let mut out = Vec::new();
    for row in db::sync::inbox_by_ids(state.db(), user_id, ids).await? {
        let n = notification(row);
        out.push((n.id, serde_json::to_value(n).map_err(|e| Problem::internal(anyhow::anyhow!(e)))?));
    }
    Ok(out)
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct NotificationsQuery {
    /// Only those not acknowledged yet.
    #[serde(default)]
    pub unread_only: bool,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// The caller's notifications, newest first. Also in the change feed as `notification`.
#[utoipa::path(get, path = "/me/notifications", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    params(NotificationsQuery),
    responses((status = 200, body = Page<Notification>), (status = 400, description = "`invalid_cursor`, `invalid_limit`", body = crate::v2::problem::ProblemBody)))]
pub async fn list_notifications(auth: AuthUser, State(state): State<AppState>, Query(q): Query<NotificationsQuery>) -> Result<Json<Page<Notification>>, Problem> {
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let rows = db::sync::inbox_page(state.db(), auth.user_id, q.unread_only, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |r| (r.created_at, r.id));
    Ok(Json(Page { items: page.items.into_iter().map(notification).collect(), next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct AckRequest {
    /// These, up to 500…
    pub ids: Option<Vec<Uuid>>,
    /// …or every one up to and including this one (the newest the device has shown).
    pub up_to: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct AckAnswer {
    /// How many were unread and are now read.
    pub acknowledged: u64,
}

fn check_ack(body: &AckRequest) -> Result<(), Problem> {
    match (&body.ids, &body.up_to) {
        (Some(_), Some(_)) | (None, None) => Err(Problem::bad_request("invalid_body", "send ids or up_to, one of them")),
        (Some(ids), None) if ids.len() > crate::v2::trash::BATCH_MAX => {
            Err(Problem::bad_request("too_many", format!("at most {} ids at a time", crate::v2::trash::BATCH_MAX)).field("/ids", "too_many", crate::v2::trash::BATCH_MAX.to_string()))
        }
        _ => Ok(()),
    }
}

/// Mark notifications read, by id or everything up to one. Ids that are not the caller's, or
/// already read, count nothing.
#[utoipa::path(post, path = "/me/notifications/ack", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("playback"))),
    request_body = AckRequest,
    responses((status = 200, body = AckAnswer),
        (status = 400, description = "`invalid_body` (neither or both), `too_many`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "`up_to` is not one of the caller's notifications", body = crate::v2::problem::ProblemBody)))]
pub async fn ack_notifications(auth: AuthUser, State(state): State<AppState>, Json(body): Json<AckRequest>) -> Result<Json<AckAnswer>, Problem> {
    check_ack(&body)?;
    let acknowledged = match (body.ids, body.up_to) {
        (Some(ids), _) => db::sync::mark_notifications_delivered(state.db(), auth.user_id, &ids).await?,
        (None, Some(up_to)) => db::sync::mark_read_up_to(state.db(), auth.user_id, up_to).await?.ok_or_else(Problem::not_found)?,
        (None, None) => 0,
    };
    Ok(Json(AckAnswer { acknowledged }))
}

// ── Push devices ─────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct PushDevice {
    pub id: Uuid,
    /// `apns` or `fcm`.
    pub platform: String,
    pub device_name: Option<String>,
    /// The token's last six characters; the token itself is a delivery credential.
    pub token_suffix: String,
    pub created_at: DateTime<Utc>,
    pub last_seen_at: DateTime<Utc>,
}

fn push_device(d: db::sync::PushDevice) -> PushDevice {
    let n = d.token.chars().count();
    PushDevice {
        id: d.id,
        platform: d.platform,
        device_name: d.device_name,
        token_suffix: d.token.chars().skip(n.saturating_sub(6)).collect(),
        created_at: d.created_at,
        last_seen_at: d.last_seen_at,
    }
}

#[derive(Deserialize, ToSchema)]
pub struct PushDeviceRequest {
    /// `apns` (Apple) or `fcm` (Android).
    pub platform: String,
    pub token: String,
    pub device_name: Option<String>,
}

fn check_push(body: &PushDeviceRequest) -> Result<(&'static str, &str), Problem> {
    let platform = match body.platform.trim() {
        "apns" => "apns",
        "fcm" => "fcm",
        other => return Err(Problem::bad_request("invalid_platform", format!("`{other}` is neither `apns` nor `fcm`")).field("/platform", "invalid", "apns or fcm")),
    };
    let token = body.token.trim();
    if token.is_empty() || token.len() > 4096 {
        return Err(Problem::bad_request("invalid_body", "a token of 1 to 4096 characters is required").field("/token", "invalid", "1 to 4096 characters"));
    }
    Ok((platform, token))
}

/// The caller's devices registered for push.
#[utoipa::path(get, path = "/me/push-tokens", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    responses((status = 200, body = Vec<PushDevice>)))]
pub async fn list_push_tokens(auth: AuthUser, State(state): State<AppState>) -> Result<Json<Vec<PushDevice>>, Problem> {
    Ok(Json(db::sync::push_devices(state.db(), auth.user_id).await?.into_iter().map(push_device).collect()))
}

/// Register this device for push; call on every start, tokens rotate. Registration only:
/// `modules.push` in discovery says whether this server sends any (this one does not; read
/// `GET /me/notifications` or the change feed). A token another account registered moves to
/// the caller, so a phone passed on stops getting its old owner's notifications.
#[utoipa::path(post, path = "/me/push-tokens", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body = PushDeviceRequest,
    responses((status = 201, description = "Registered", body = PushDevice), (status = 200, description = "Already the caller's: refreshed", body = PushDevice),
        (status = 400, description = "`invalid_platform`, `invalid_body`", body = crate::v2::problem::ProblemBody)))]
pub async fn register_push_token(auth: AuthUser, State(state): State<AppState>, Json(body): Json<PushDeviceRequest>) -> Result<(StatusCode, Json<PushDevice>), Problem> {
    let (platform, token) = check_push(&body)?;
    let name = body.device_name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(100).collect::<String>());
    let (row, had) = db::sync::upsert_push_device(state.db(), auth.user_id, platform, token, name.as_deref()).await?;
    Ok((if had { StatusCode::OK } else { StatusCode::CREATED }, Json(push_device(row))))
}

/// Stop push to a device, as on signing out of it.
#[utoipa::path(delete, path = "/me/push-tokens/{id}", tag = "listening", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(("id" = Uuid, Path, description = "From `GET /me/push-tokens`")),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not one of the caller's", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_push_token(auth: AuthUser, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    if !db::sync::delete_push_device(state.db(), auth.user_id, id).await? {
        return Err(Problem::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(items: Value, index: i32) -> db::sync::PlayQueue {
        db::sync::PlayQueue {
            items,
            current_index: index,
            position_secs: 12.5,
            updated_by_device: Some("ios".into()),
            updated_by_device_label: None,
            updated_at: Utc::now(),
            revision: 3,
        }
    }

    #[test]
    fn a_v1_queue_reads_in_v2_words() {
        let (t, b, f, feed, e) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let q = queue_of(Some(row(
            json!([
                { "media_kind": "music", "item_id": t },
                { "media_kind": "podcast", "item_id": feed },
                { "media_kind": "audiobook", "item_id": b, "part_id": f },
                { "media_kind": "podcast", "item_id": feed, "part_id": e },
            ]),
            2,
        )));
        assert_eq!(q.items, vec![
            QueueItem { kind: "track".into(), id: t, file_id: None },
            QueueItem { kind: "book".into(), id: b, file_id: Some(f) },
            QueueItem { kind: "episode".into(), id: e, file_id: None },
        ]);
        assert_eq!(q.index, 1, "the index follows its item past a show v2 cannot name");
        assert_eq!((q.position_ms, q.revision), (12_500, 3));
        let empty = queue_of(None);
        assert_eq!((empty.revision, empty.items.len(), empty.updated_at), (0, 0, None));
    }

    #[test]
    fn if_match_takes_a_revision() {
        let h = |v: &str| {
            let mut m = HeaderMap::new();
            m.insert(header::IF_MATCH, HeaderValue::from_str(v).unwrap());
            m
        };
        assert_eq!(if_match(&HeaderMap::new()), None);
        assert_eq!(if_match(&h("*")), None);
        assert_eq!(if_match(&h("\"7\"")), Some(7));
        assert_eq!(if_match(&h("7")), Some(7));
        assert_eq!(if_match(&h("W/\"0\"")), Some(0));
        assert_eq!(if_match(&h("\"abc\"")), Some(-1));
    }

    #[test]
    fn a_queue_is_checked_before_the_database() {
        let item = |kind: &str, file: bool| QueueItem { kind: kind.into(), id: Uuid::new_v4(), file_id: file.then(Uuid::new_v4) };
        let req = |items| QueueRequest { items, index: 0, position_ms: 0 };
        assert!(check_queue(&req(vec![item("track", false), item("book", true), item("episode", false)])).is_ok());
        assert_eq!(check_queue(&req(vec![item("album", false)])).unwrap_err().code(), "invalid_kind");
        assert_eq!(check_queue(&req(vec![item("track", true)])).unwrap_err().code(), "invalid_body");
        assert_eq!(check_queue(&req((0..=QUEUE_MAX).map(|_| item("track", false)).collect())).unwrap_err().code(), "too_many");
    }

    #[test]
    fn settings_are_refused_out_of_range() {
        let ok = SettingsPatch {
            playback: Some(PlaybackPatch { speed: Some(1.5), skip_intro_ms: Some(30_000), skip_outro_ms: None }),
            audiobook: Some(AudiobookPatch { speed: Some(0.5), skip_back_ms: Some(10_000), skip_forward_ms: Some(120_000) }),
            eq: Some(EqPatch { device_kind: Some("tvos".into()), bands_db: Some(vec![0.0, 1.0, -12.0, 12.0, 3.5, 0.0]), ..Default::default() }),
        };
        assert!(check_settings(&ok).is_ok());
        let bad = |p: SettingsPatch| check_settings(&p).unwrap_err().code().to_string();
        assert_eq!(bad(SettingsPatch { audiobook: Some(AudiobookPatch { skip_back_ms: Some(500), ..Default::default() }), ..Default::default() }), "invalid_body");
        assert_eq!(bad(SettingsPatch { playback: Some(PlaybackPatch { speed: Some(4.0), ..Default::default() }), ..Default::default() }), "invalid_body");
        assert_eq!(bad(SettingsPatch { playback: Some(PlaybackPatch { speed: Some(f64::NAN), ..Default::default() }), ..Default::default() }), "invalid_body");
        assert_eq!(bad(SettingsPatch { eq: Some(EqPatch { bands_db: Some(vec![0.0; 5]), ..Default::default() }), ..Default::default() }), "invalid_body");
        assert_eq!(bad(SettingsPatch { eq: Some(EqPatch { device_kind: Some("toaster".into()), ..Default::default() }), ..Default::default() }), "invalid_body");
        assert_eq!(bad(SettingsPatch { eq: Some(EqPatch { preamp_db: Some(13.0), ..Default::default() }), ..Default::default() }), "invalid_body");
        assert_eq!((secs(15_000), secs(15_499), secs(15_500)), (15, 15, 16));
    }

    #[test]
    fn a_notification_names_its_item_in_v2_words() {
        let id = Uuid::new_v4();
        let is = |kind: &str, data: Value| item_of(kind, Some(&data)).map(|i| i.kind);
        assert_eq!(is("new_episodes", json!({ "media_kind": "podcast", "item_id": id.to_string() })), Some("podcast".into()));
        assert_eq!(is("access_request", json!({ "media_kind": "music", "item_id": id.to_string() })), Some("track".into()));
        assert_eq!(is("item_trashed", json!({ "kind": "audiobook", "id": id })), Some("book".into()));
        assert_eq!(is("item_trashed", json!({ "kind": "music_track", "id": id })), Some("track".into()));
        assert_eq!(is("playlist_shared", json!({ "playlist_id": id.to_string() })), Some("playlist".into()));
        assert_eq!(is("something_new", json!({ "item_id": id.to_string() })), None);
        assert_eq!(item_of("new_episodes", None), None);
    }

    #[test]
    fn ack_takes_ids_or_up_to() {
        let id = Uuid::new_v4();
        assert!(check_ack(&AckRequest { ids: Some(vec![id]), up_to: None }).is_ok());
        assert!(check_ack(&AckRequest { ids: None, up_to: Some(id) }).is_ok());
        assert_eq!(check_ack(&AckRequest { ids: None, up_to: None }).unwrap_err().code(), "invalid_body");
        assert_eq!(check_ack(&AckRequest { ids: Some(vec![]), up_to: Some(id) }).unwrap_err().code(), "invalid_body");
        assert_eq!(check_ack(&AckRequest { ids: Some(vec![id; 501]), up_to: None }).unwrap_err().code(), "too_many");
    }

    #[test]
    fn push_tokens_are_checked() {
        let req = |p: &str, t: &str| PushDeviceRequest { platform: p.into(), token: t.into(), device_name: None };
        assert_eq!(check_push(&req("apns", " abc ")).unwrap(), ("apns", "abc"));
        assert_eq!(check_push(&req("sms", "abc")).unwrap_err().code(), "invalid_platform");
        assert_eq!(check_push(&req("fcm", "  ")).unwrap_err().code(), "invalid_body");
        let d = push_device(db::sync::PushDevice {
            id: Uuid::new_v4(),
            platform: "fcm".into(),
            token: "abcdefghij".into(),
            device_name: None,
            created_at: Utc::now(),
            last_seen_at: Utc::now(),
        });
        assert_eq!(d.token_suffix, "efghij");
    }
}
