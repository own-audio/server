// SPDX-License-Identifier: AGPL-3.0-or-later
//! The gPodder sync API (gpodder.net's API v2) at `/api/2`, for podcast apps
//! that keep subscriptions and positions on a sync server: AntennaPod, Kasts,
//! gPodder, own.audio Podcasts. See `docs/gpodder.md`.
//!
//! A listener's subscriptions are the RSS feeds they own here; a show added
//! through the API becomes one of their podcasts, and one they add anywhere
//! else shows up in their apps. Play actions become their own progress in the
//! episode when a feed they can see has it, and every position recorded here
//! is handed to the apps as a play action (migration 0115's triggers).
//!
//! Apps sign in with HTTP basic authentication on every request: the account's
//! email and its app password, the same one Subsonic apps use — never the
//! account password, which stays behind two-factor sign-in.
use crate::app::AppState;
use crate::db;
use crate::db::access::Viewer;
use axum::Router;
use axum::body::Bytes;
use axum::extract::{FromRequestParts, Path, Query, State};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use base64::Engine;
use chrono::{DateTime, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/auth/{user}/login.json", post(login))
        .route("/auth/{user}/logout.json", post(logout))
        .route("/devices/{user}", get(list_devices))
        .route("/devices/{user}/{device}", post(update_device))
        .route("/subscriptions/{user}/{device}", get(get_subscriptions).post(upload_subscriptions))
        .route("/episodes/{user}", get(get_actions).post(upload_actions))
        .fallback(|| async { GError::NotFound })
}

// ── Errors ───────────────────────────────────────────────────────────────

#[derive(Debug)]
pub enum GError {
    Unauthorized,
    Forbidden,
    BadRequest(String),
    NotFound,
    Internal(anyhow::Error),
}

impl IntoResponse for GError {
    fn into_response(self) -> Response {
        match self {
            GError::Unauthorized => {
                let mut response = (StatusCode::UNAUTHORIZED, axum::Json(json!({ "error": "unauthorized" }))).into_response();
                response.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Basic realm=\"own.audio\""));
                response
            }
            GError::Forbidden => (StatusCode::FORBIDDEN, axum::Json(json!({ "error": "forbidden" }))).into_response(),
            GError::BadRequest(why) => (StatusCode::BAD_REQUEST, axum::Json(json!({ "error": why }))).into_response(),
            GError::NotFound => (StatusCode::NOT_FOUND, axum::Json(json!({ "error": "not_found" }))).into_response(),
            GError::Internal(e) => {
                tracing::error!("gpodder: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, axum::Json(json!({ "error": "internal" }))).into_response()
            }
        }
    }
}

impl From<anyhow::Error> for GError {
    fn from(e: anyhow::Error) -> Self {
        GError::Internal(e)
    }
}

// ── Signing in ───────────────────────────────────────────────────────────

/// The listener a request's basic authentication names.
pub struct GUser {
    pub user_id: Uuid,
    pub family_id: Uuid,
    pub is_family_admin: bool,
    pub email: String,
    pub read_only: bool,
}

impl GUser {
    fn viewer(&self) -> Viewer {
        Viewer { user_id: self.user_id, family_id: self.family_id, is_family_admin: self.is_family_admin }
    }

    /// The `{user}` in the path must be the account signed in.
    fn check(&self, path_user: &str) -> Result<(), GError> {
        let name = strip_json(path_user);
        if name.trim().eq_ignore_ascii_case(self.email.trim()) { Ok(()) } else { Err(GError::Forbidden) }
    }

    fn check_writable(&self) -> Result<(), GError> {
        if self.read_only { Err(GError::Forbidden) } else { Ok(()) }
    }
}

fn basic_credentials(parts: &Parts) -> Option<(String, String)> {
    let value = parts.headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let encoded = value.strip_prefix("Basic ").or_else(|| value.strip_prefix("basic "))?;
    let decoded = base64::engine::general_purpose::STANDARD.decode(encoded.trim()).ok()?;
    let text = String::from_utf8(decoded).ok()?;
    let (user, password) = text.split_once(':')?;
    Some((user.to_string(), password.to_string()))
}

/// Compares in time that does not depend on where the strings first differ.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

impl FromRequestParts<AppState> for GUser {
    type Rejection = GError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let (email, password) = basic_credentials(parts).ok_or(GError::Unauthorized)?;
        let user = db::users::find_by_email_ci(state.db(), &email).await?.ok_or(GError::Unauthorized)?;
        if !user.is_active {
            return Err(GError::Unauthorized);
        }
        let key = db::subsonic::get_key(state.db(), state.at_rest(), user.id).await?.ok_or(GError::Unauthorized)?;
        // A key pasted on a phone often brings a space along.
        if !same_secret(password.trim(), &key) {
            return Err(GError::Unauthorized);
        }
        let membership = db::families::ensure_membership(state.db(), user.id, state.hooks().one_family()).await?;
        Ok(GUser {
            user_id: user.id,
            family_id: membership.family_id,
            is_family_admin: membership.role == "family_admin" || user.role == "admin",
            read_only: crate::demo::is_read_only_account(state, &user.email),
            email: user.email,
        })
    }
}

/// `jo@example.com.json` → `jo@example.com`: the API puts the format on the
/// last segment, and JSON is the one served.
fn strip_json(segment: &str) -> &str {
    segment.strip_suffix(".json").unwrap_or(segment)
}

fn valid_device(id: &str) -> Result<&str, GError> {
    let id = strip_json(id);
    let ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if ok { Ok(id) } else { Err(GError::BadRequest("invalid device id".into())) }
}

fn parse_json<T: for<'de> Deserialize<'de>>(body: &Bytes) -> Result<T, GError> {
    serde_json::from_slice(body).map_err(|e| GError::BadRequest(format!("invalid JSON: {e}")))
}

async fn login(user: GUser, Path(path_user): Path<String>) -> Result<StatusCode, GError> {
    user.check(&path_user)?;
    Ok(StatusCode::OK)
}

async fn logout(user: GUser, Path(path_user): Path<String>) -> Result<StatusCode, GError> {
    user.check(&path_user)?;
    Ok(StatusCode::OK)
}

// ── Devices ──────────────────────────────────────────────────────────────

async fn list_devices(user: GUser, State(state): State<AppState>, Path(path_user): Path<String>) -> Result<axum::Json<Value>, GError> {
    user.check(&path_user)?;
    let devices = db::gpodder::list_devices(state.db(), user.user_id).await?;
    // Subscriptions are the account's, the same on every device.
    let count = db::gpodder::owned_feeds(state.db(), user.user_id).await?.len();
    Ok(axum::Json(Value::Array(
        devices
            .into_iter()
            .map(|d| json!({ "id": d.device_id, "caption": d.caption, "type": d.kind, "subscriptions": count }))
            .collect(),
    )))
}

#[derive(Deserialize)]
struct DeviceBody {
    caption: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
}

async fn update_device(user: GUser, State(state): State<AppState>, Path((path_user, device)): Path<(String, String)>, body: Bytes) -> Result<StatusCode, GError> {
    user.check(&path_user)?;
    let device = valid_device(&device)?;
    let body: DeviceBody = if body.is_empty() { DeviceBody { caption: None, kind: None } } else { parse_json(&body)? };
    let kind = body.kind.map(|k| k.to_ascii_lowercase()).map(|k| {
        if matches!(k.as_str(), "desktop" | "laptop" | "mobile" | "server" | "other") { k } else { "other".into() }
    });
    let caption = body.caption.map(|c| c.chars().take(200).collect::<String>());
    db::gpodder::upsert_device(state.db(), user.user_id, device, caption.as_deref(), kind.as_deref()).await?;
    Ok(StatusCode::OK)
}

// ── Subscriptions ────────────────────────────────────────────────────────

#[derive(Deserialize)]
struct SinceQuery {
    since: Option<String>,
}

fn parse_since(raw: Option<&str>) -> Result<i64, GError> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        None => Ok(0),
        Some(text) => text.parse::<i64>().map(|v| v.max(0)).map_err(|_| GError::BadRequest("since must be a number".into())),
    }
}

async fn get_subscriptions(
    user: GUser,
    State(state): State<AppState>,
    Path((path_user, device)): Path<(String, String)>,
    Query(query): Query<SinceQuery>,
) -> Result<axum::Json<Value>, GError> {
    user.check(&path_user)?;
    let device = valid_device(&device)?;
    let since = parse_since(query.since.as_deref())?;
    // An app that asks before registering its device has registered it.
    db::gpodder::upsert_device(state.db(), user.user_id, device, None, None).await?;
    let timestamp = db::gpodder::now_secs(state.db()).await?;
    let owned: std::collections::HashSet<String> =
        db::gpodder::owned_feeds(state.db(), user.user_id).await?.into_iter().map(|(_, url)| url).collect();
    let (add, remove) = if since == 0 {
        let mut all: Vec<String> = owned.into_iter().collect();
        all.sort();
        (all, Vec::new())
    } else {
        let mut add = Vec::new();
        let mut remove = Vec::new();
        // The log says what changed; what is owned now says how it ended, so
        // a show added and removed again in the window is a removal.
        for (url, _) in db::gpodder::subscription_changes(state.db(), user.user_id, since, device).await? {
            if owned.contains(&url) { add.push(url) } else { remove.push(url) }
        }
        (add, remove)
    };
    Ok(axum::Json(json!({ "add": add, "remove": remove, "timestamp": timestamp })))
}

#[derive(Deserialize)]
struct SubscriptionBody {
    #[serde(default)]
    add: Vec<String>,
    #[serde(default)]
    remove: Vec<String>,
}

/// A feed URL as it is kept: trimmed, with a scheme; `None` for what cannot be
/// a feed. `feed://`, `itpc://` and `pcast://` links mean HTTP.
pub fn sanitize_url(raw: &str) -> Option<String> {
    let text = raw.trim();
    let lowered = text.to_ascii_lowercase();
    let text = ["feed://", "itpc://", "pcast://", "podcast://"]
        .iter()
        .find(|p| lowered.starts_with(*p))
        .map(|p| format!("https://{}", &text[p.len()..]))
        .unwrap_or_else(|| text.to_string());
    let lowered = text.to_ascii_lowercase();
    if !(lowered.starts_with("http://") || lowered.starts_with("https://")) {
        return None;
    }
    let url = url::Url::parse(&text).ok()?;
    url.host_str()?;
    if text.len() > 2048 { None } else { Some(text) }
}

async fn upload_subscriptions(
    user: GUser,
    State(state): State<AppState>,
    Path((path_user, device)): Path<(String, String)>,
    body: Bytes,
) -> Result<axum::Json<Value>, GError> {
    user.check(&path_user)?;
    user.check_writable()?;
    let device = valid_device(&device)?;
    let body: SubscriptionBody = parse_json(&body)?;
    if body.add.len() + body.remove.len() > 5000 {
        return Err(GError::BadRequest("too many changes in one request".into()));
    }
    db::gpodder::upsert_device(state.db(), user.user_id, device, None, None).await?;

    let mut update_urls: Vec<[String; 2]> = Vec::new();
    let mut clean = |list: &[String]| -> Vec<String> {
        let mut out = Vec::new();
        for raw in list {
            let cleaned = sanitize_url(raw);
            if cleaned.as_deref() != Some(raw.as_str()) {
                update_urls.push([raw.clone(), cleaned.clone().unwrap_or_default()]);
            }
            if let Some(url) = cleaned {
                if !out.contains(&url) {
                    out.push(url);
                }
            }
        }
        out
    };
    let add = clean(&body.add);
    let remove = clean(&body.remove);
    if add.iter().any(|u| remove.contains(u)) {
        return Err(GError::BadRequest("the same feed is both added and removed".into()));
    }

    let owned: HashMap<String, Uuid> =
        db::gpodder::owned_feeds(state.db(), user.user_id).await?.into_iter().map(|(id, url)| (url, id)).collect();
    let mut new_feeds = Vec::new();
    for url in &add {
        // A show already here, the listener's or shared with them, is not copied.
        if !db::gpodder::visible_feed_exists(state.db(), user.viewer(), url).await? {
            new_feeds.push(url.clone());
        }
    }
    let gone: Vec<Uuid> = remove.iter().filter_map(|url| owned.get(url).copied()).collect();
    let created = db::gpodder::apply_subscriptions(state.db(), user.user_id, device, &new_feeds, &gone).await?;
    for feed_id in created {
        // The worker reads the feed: its title, its art, its episodes.
        db::jobs::enqueue(state.db(), "feed_refresh", Some(&json!({ "feed_id": feed_id.to_string() })), None).await?;
    }
    for feed_id in gone {
        let _ = db::sync::record_deletion(state.db(), db::access::PODCAST, feed_id, user.user_id, None).await;
    }
    let timestamp = db::gpodder::now_secs(state.db()).await?;
    Ok(axum::Json(json!({ "timestamp": timestamp, "update_urls": update_urls })))
}

// ── Episode actions ──────────────────────────────────────────────────────

#[derive(Deserialize)]
struct ActionsQuery {
    since: Option<String>,
    podcast: Option<String>,
    device: Option<String>,
    aggregated: Option<String>,
}

#[derive(Serialize)]
struct ActionOut {
    podcast: String,
    episode: String,
    action: String,
    timestamp: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    device: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    guid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    started: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    position: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    total: Option<i32>,
}

/// The API's own form: UTC, no zone, whole seconds.
fn format_time(at: DateTime<Utc>) -> String {
    at.format("%Y-%m-%dT%H:%M:%S").to_string()
}

fn parse_time(value: &Value) -> Option<DateTime<Utc>> {
    match value {
        Value::Number(n) => n.as_i64().and_then(|s| DateTime::from_timestamp(s, 0)),
        Value::String(text) => {
            let text = text.trim();
            if let Ok(at) = DateTime::parse_from_rfc3339(text) {
                return Some(at.with_timezone(&Utc));
            }
            for format in ["%Y-%m-%dT%H:%M:%S%.f", "%Y-%m-%dT%H:%M:%S", "%Y-%m-%d %H:%M:%S"] {
                if let Ok(naive) = NaiveDateTime::parse_from_str(text, format) {
                    return Some(naive.and_utc());
                }
            }
            None
        }
        _ => None,
    }
}

async fn get_actions(
    user: GUser,
    State(state): State<AppState>,
    Path(path_user): Path<String>,
    Query(query): Query<ActionsQuery>,
) -> Result<axum::Json<Value>, GError> {
    user.check(&path_user)?;
    let since = parse_since(query.since.as_deref())?;
    let timestamp = db::gpodder::now_secs(state.db()).await?;
    let podcast = query.podcast.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let device = query.device.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let mut actions = db::gpodder::list_actions(state.db(), user.user_id, since, podcast, device).await?;
    if query.aggregated.as_deref().is_some_and(|a| a.eq_ignore_ascii_case("true")) {
        // The latest action per episode, in the order they happened.
        let mut latest: HashMap<(String, String), usize> = HashMap::new();
        for (index, action) in actions.iter().enumerate() {
            latest.insert((action.podcast_url.clone(), action.episode_url.clone()), index);
        }
        let keep: std::collections::HashSet<usize> = latest.into_values().collect();
        actions = actions.into_iter().enumerate().filter(|(i, _)| keep.contains(i)).map(|(_, a)| a).collect();
    }
    let out: Vec<ActionOut> = actions
        .into_iter()
        .map(|a| ActionOut {
            podcast: a.podcast_url,
            episode: a.episode_url,
            timestamp: format_time(a.happened_at),
            device: a.device_id,
            guid: a.guid,
            started: if a.action == "play" { a.started } else { None },
            position: if a.action == "play" { a.position } else { None },
            total: if a.action == "play" { a.total } else { None },
            action: a.action,
        })
        .collect();
    Ok(axum::Json(json!({ "actions": out, "timestamp": timestamp })))
}

/// Within the last 30 seconds, or past 98%: heard, as the apps count it.
fn reached_end(position: i32, total: Option<i32>) -> bool {
    match total {
        Some(total) if total > 0 => total - position <= 30 || f64::from(position) / f64::from(total) >= 0.98,
        _ => false,
    }
}

async fn upload_actions(user: GUser, State(state): State<AppState>, Path(path_user): Path<String>, body: Bytes) -> Result<axum::Json<Value>, GError> {
    user.check(&path_user)?;
    user.check_writable()?;
    let raw: Vec<Value> = parse_json(&body)?;
    if raw.len() > 5000 {
        return Err(GError::BadRequest("too many actions in one request".into()));
    }
    let now = Utc::now();
    let mut update_urls: Vec<[String; 2]> = Vec::new();
    let mut parsed = Vec::with_capacity(raw.len());
    for (index, item) in raw.iter().enumerate() {
        let field = |name: &str| item.get(name).and_then(Value::as_str).map(str::to_string);
        let number = |name: &str| item.get(name).and_then(Value::as_i64).map(|n| n.clamp(0, i64::from(i32::MAX)) as i32);
        let action = field("action").map(|a| a.to_ascii_lowercase()).unwrap_or_default();
        if !matches!(action.as_str(), "download" | "play" | "delete" | "new") {
            return Err(GError::BadRequest(format!("action {index}: unknown action")));
        }
        let (Some(podcast_raw), Some(episode_raw)) = (field("podcast"), field("episode")) else {
            return Err(GError::BadRequest(format!("action {index}: podcast and episode are required")));
        };
        let (Some(podcast), Some(episode)) = (sanitize_url(&podcast_raw), sanitize_url(&episode_raw)) else {
            return Err(GError::BadRequest(format!("action {index}: podcast and episode must be http(s) URLs")));
        };
        for (old, new) in [(&podcast_raw, &podcast), (&episode_raw, &episode)] {
            if old != new && !update_urls.iter().any(|p| &p[0] == old) {
                update_urls.push([old.clone(), new.clone()]);
            }
        }
        let device = match field("device") {
            Some(d) => Some(valid_device(&d)?.to_string()),
            None => None,
        };
        // A time from the future is a wrong clock; it would win every comparison.
        let happened_at = item.get("timestamp").and_then(parse_time).map(|t| t.min(now)).unwrap_or(now);
        parsed.push(db::gpodder::Action {
            podcast_url: podcast,
            episode_url: episode,
            guid: field("guid").filter(|g| !g.is_empty()),
            started: if action == "play" { number("started") } else { None },
            position: if action == "play" { number("position") } else { None },
            total: if action == "play" { number("total") } else { None },
            action,
            device_id: device,
            happened_at,
        });
    }
    for action in &parsed {
        db::gpodder::insert_action(state.db(), user.user_id, action).await?;
        let progress = match action.action.as_str() {
            "play" => action.position.map(|p| (f64::from(p), reached_end(p, action.total))),
            "new" => Some((0.0, false)),
            _ => None,
        };
        if let Some((position, completed)) = progress {
            db::gpodder::apply_progress(
                state.db(),
                user.viewer(),
                &action.podcast_url,
                &action.episode_url,
                action.guid.as_deref(),
                position,
                completed,
                action.happened_at,
            )
            .await?;
        }
    }
    let timestamp = db::gpodder::now_secs(state.db()).await?;
    Ok(axum::Json(json!({ "timestamp": timestamp, "update_urls": update_urls })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_links_become_https_and_junk_is_refused() {
        assert_eq!(sanitize_url("  https://example.com/feed.xml "), Some("https://example.com/feed.xml".into()));
        assert_eq!(sanitize_url("feed://example.com/rss"), Some("https://example.com/rss".into()));
        assert_eq!(sanitize_url("itpc://example.com/rss"), Some("https://example.com/rss".into()));
        assert_eq!(sanitize_url("javascript:alert(1)"), None);
        assert_eq!(sanitize_url("not a url"), None);
        assert_eq!(sanitize_url("http://"), None);
    }

    #[test]
    fn the_apis_timestamps_read_in_every_form_apps_send() {
        let expected = DateTime::from_timestamp(1_792_224_000, 0).unwrap();
        assert_eq!(parse_time(&json!("2026-10-17T08:00:00")), Some(expected));
        assert_eq!(parse_time(&json!("2026-10-17T08:00:00.250")).map(|t| t.timestamp()), Some(1_792_224_000));
        assert_eq!(parse_time(&json!("2026-10-17T08:00:00Z")), Some(expected));
        assert_eq!(parse_time(&json!(1_792_224_000)), Some(expected));
        assert_eq!(parse_time(&json!("yesterday")), None);
        assert_eq!(format_time(expected), "2026-10-17T08:00:00");
    }

    #[test]
    fn the_end_of_an_episode_counts_as_heard() {
        assert!(reached_end(590, Some(600)));
        assert!(reached_end(3580, Some(3600)));
        assert!(!reached_end(100, Some(600)));
        assert!(!reached_end(100, None));
    }

    #[test]
    fn device_ids_follow_the_apis_rules() {
        assert_eq!(valid_device("phone-1.json").ok(), Some("phone-1"));
        assert!(valid_device("bad id").is_err());
        assert!(valid_device("").is_err());
    }

    #[test]
    fn the_path_user_matches_the_account_whatever_the_case() {
        let user = GUser { user_id: Uuid::nil(), family_id: Uuid::nil(), is_family_admin: false, email: "Jo@Example.com".into(), read_only: false };
        assert!(user.check("jo@example.com.json").is_ok());
        assert!(user.check("someone@example.com").is_err());
    }
}
