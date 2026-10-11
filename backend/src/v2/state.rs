// SPDX-License-Identifier: AGPL-3.0-or-later
//! The listener's own state (DESIGN §7.6; docs/api-v2-plan.md, V4): favourites,
//! ratings, progress with the last position on each device, plays, bookmarks,
//! and `POST /sync/push` for what a device changed while offline.
//!
//! Merge rules: the later change wins. A client's timestamp is clamped to the
//! server's clock on receipt (a phone set to next year cannot win every
//! argument), and progress older than what is stored is kept only as that
//! device's last position. Plays are idempotent by the client's id.

use crate::app::AppState;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::db::user_state::{self, Kind};
use crate::families::FamilyContext;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

fn kind(s: &str, allowed: fn(Kind) -> bool, what: &str) -> Result<Kind, Problem> {
    Kind::parse(s)
        .filter(|k| allowed(*k))
        .ok_or_else(|| Problem::bad_request("unsupported_kind", format!("`{s}` has no {what}")).field("kind", "invalid", s.to_string()))
}

async fn visible(state: &AppState, family: &FamilyContext, kind: Kind, id: Uuid) -> Result<(), Problem> {
    if user_state::is_visible(state.db(), family.viewer(), kind, id).await? { Ok(()) } else { Err(Problem::not_found()) }
}

/// A client's clock, never ahead of the server's.
fn clamp(at: Option<DateTime<Utc>>) -> DateTime<Utc> {
    let now = Utc::now();
    at.map_or(now, |at| at.min(now))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct KindPage {
    /// `track`, `album`, `contributor`, `book` or `episode`, as the list allows.
    pub kind: String,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

impl KindPage {
    fn page(&self) -> PageQuery {
        PageQuery { limit: self.limit, cursor: self.cursor.clone() }
    }
}

// ── Favourites ───────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Favourite {
    pub kind: String,
    pub id: Uuid,
    pub created_at: DateTime<Utc>,
}

/// The caller's favourites of one kind, newest first.
#[utoipa::path(get, path = "/me/favourites", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(KindPage),
    responses((status = 200, body = Page<Favourite>), (status = 400, description = "`unsupported_kind`", body = crate::v2::problem::ProblemBody)))]
pub async fn list_favourites(family: FamilyContext, State(state): State<AppState>, Query(q): Query<KindPage>) -> Result<Json<Page<Favourite>>, Problem> {
    let k = kind(&q.kind, Kind::can_favourite, "favourites")?;
    let page = q.page();
    let limit = page.limit()?;
    let rows = user_state::list_favourites(state.db(), family.viewer(), k, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |(id, at)| (*at, *id));
    Ok(Json(Page {
        items: page.items.into_iter().map(|(id, created_at)| Favourite { kind: k.as_str().into(), id, created_at }).collect(),
        next: page.next,
        included: None,
    }))
}

async fn apply_favourite(state: &AppState, family: &FamilyContext, k: &str, id: Uuid, on: bool) -> Result<(), Problem> {
    let k = kind(k, Kind::can_favourite, "favourites")?;
    if on {
        visible(state, family, k, id).await?;
    }
    user_state::set_favourite(state.db(), family.user_id, k, id, on).await?;
    Ok(())
}

/// Make it a favourite.
#[utoipa::path(put, path = "/me/favourites/{kind}/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("kind" = String, Path, description = "`track`, `album`, `contributor` or `book`"), ("id" = Uuid, Path)),
    responses((status = 204, description = "A favourite"), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn put_favourite(family: FamilyContext, State(state): State<AppState>, Path((k, id)): Path<(String, Uuid)>) -> Result<StatusCode, Problem> {
    apply_favourite(&state, &family, &k, id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Not a favourite any more.
#[utoipa::path(delete, path = "/me/favourites/{kind}/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("kind" = String, Path), ("id" = Uuid, Path)),
    responses((status = 204, description = "Not a favourite (also when it was not one)")))]
pub async fn delete_favourite(family: FamilyContext, State(state): State<AppState>, Path((k, id)): Path<(String, Uuid)>) -> Result<StatusCode, Problem> {
    apply_favourite(&state, &family, &k, id, false).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Ratings ──────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Rating {
    pub kind: String,
    pub id: Uuid,
    /// 1 to 5.
    pub rating: i16,
    pub updated_at: DateTime<Utc>,
}

#[derive(Deserialize, ToSchema)]
pub struct RatingRequest {
    /// 1 to 5; 0 takes the rating away.
    pub rating: i16,
}

/// The caller's ratings of one kind, most recently changed first.
#[utoipa::path(get, path = "/me/ratings", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(KindPage),
    responses((status = 200, body = Page<Rating>), (status = 400, description = "`unsupported_kind`", body = crate::v2::problem::ProblemBody)))]
pub async fn list_ratings(family: FamilyContext, State(state): State<AppState>, Query(q): Query<KindPage>) -> Result<Json<Page<Rating>>, Problem> {
    let k = kind(&q.kind, Kind::can_rate, "ratings")?;
    let page = q.page();
    let limit = page.limit()?;
    let rows = user_state::list_ratings(state.db(), family.viewer(), k, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |(id, _, at)| (*at, *id));
    Ok(Json(Page {
        items: page.items.into_iter().map(|(id, rating, updated_at)| Rating { kind: k.as_str().into(), id, rating, updated_at }).collect(),
        next: page.next,
        included: None,
    }))
}

async fn apply_rating(state: &AppState, family: &FamilyContext, k: &str, id: Uuid, rating: i16) -> Result<(), Problem> {
    let k = kind(k, Kind::can_rate, "ratings")?;
    if !(0..=5).contains(&rating) {
        return Err(Problem::bad_request("invalid_body", "a rating is 0 to 5").field("/rating", "out_of_range", "0 to 5"));
    }
    visible(state, family, k, id).await?;
    user_state::set_rating(state.db(), family.user_id, k, id, (rating > 0).then_some(rating)).await?;
    Ok(())
}

/// Rate it, or take the rating away with 0.
#[utoipa::path(put, path = "/me/ratings/{kind}/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("kind" = String, Path, description = "`track`, `album` or `contributor`"), ("id" = Uuid, Path)),
    request_body = RatingRequest,
    responses((status = 204, description = "Rated"), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn put_rating(family: FamilyContext, State(state): State<AppState>, Path((k, id)): Path<(String, Uuid)>, Json(body): Json<RatingRequest>) -> Result<StatusCode, Problem> {
    apply_rating(&state, &family, &k, id, body.rating).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Progress ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Progress {
    pub kind: String,
    pub id: Uuid,
    /// From the start; for a book, across all its files.
    pub position_ms: i64,
    pub finished: bool,
    pub updated_at: DateTime<Utc>,
    /// A book's: the file the position is in, and where in that file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_position_ms: Option<i64>,
    /// Where each device last was; only on `GET /me/progress/{kind}/{id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub devices: Option<Vec<DevicePosition>>,
}

#[derive(Serialize, ToSchema)]
pub struct DevicePosition {
    /// The device's session id, as in `GET /me/sessions`.
    pub device_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub position_ms: i64,
    pub finished: bool,
    pub at: DateTime<Utc>,
    /// The device asking.
    pub current: bool,
}

fn progress(k: Kind, p: user_state::Progress) -> Progress {
    Progress {
        kind: k.as_str().into(),
        id: p.item_id,
        position_ms: p.position_ms,
        finished: p.finished,
        updated_at: p.updated_at,
        file_id: p.file_id,
        file_position_ms: p.file_position_ms,
        devices: None,
    }
}

/// The caller's progress on one kind, most recently changed first.
#[utoipa::path(get, path = "/me/progress", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(KindPage),
    responses((status = 200, body = Page<Progress>), (status = 400, description = "`unsupported_kind`", body = crate::v2::problem::ProblemBody)))]
pub async fn list_progress(family: FamilyContext, State(state): State<AppState>, Query(q): Query<KindPage>) -> Result<Json<Page<Progress>>, Problem> {
    let k = kind(&q.kind, Kind::has_progress, "progress")?;
    let page = q.page();
    let limit = page.limit()?;
    let rows = user_state::list_progress(state.db(), family.viewer(), k, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |p| (p.updated_at, p.item_id));
    Ok(Json(Page { items: page.items.into_iter().map(|p| progress(k, p)).collect(), next: page.next, included: None }))
}

/// Where the caller is in one item, and where each of their devices last was.
#[utoipa::path(get, path = "/me/progress/{kind}/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("kind" = String, Path, description = "`track`, `book` or `episode`"), ("id" = Uuid, Path)),
    responses((status = 200, body = Progress), (status = 404, description = "Not started, unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_progress(auth: AuthUser, family: FamilyContext, State(state): State<AppState>, Path((k, id)): Path<(String, Uuid)>) -> Result<Json<Progress>, Problem> {
    let k = kind(&k, Kind::has_progress, "progress")?;
    visible(&state, &family, k, id).await?;
    let p = user_state::get_progress(state.db(), family.user_id, k, id).await?.ok_or_else(Problem::not_found)?;
    let mut answer = progress(k, p);
    let this = device_id(&auth);
    answer.devices = Some(
        user_state::device_positions(state.db(), family.user_id, k, id)
            .await?
            .into_iter()
            .map(|d| DevicePosition {
                current: d.device_id == this,
                device_id: d.device_id,
                device_name: d.device_name,
                device_kind: d.device_kind,
                position_ms: d.position_ms,
                finished: d.finished,
                at: d.at,
            })
            .collect(),
    );
    Ok(Json(answer))
}

#[derive(Deserialize, ToSchema)]
pub struct ProgressRequest {
    /// From the start; for a book, across its files. Or, for a book, give
    /// `file_id` and `file_position_ms` instead.
    pub position_ms: Option<i64>,
    pub file_id: Option<Uuid>,
    pub file_position_ms: Option<i64>,
    #[serde(default)]
    pub finished: bool,
    /// When the listener was there, by the device's clock; now when absent.
    pub at: Option<DateTime<Utc>>,
}

#[derive(Serialize, ToSchema)]
pub struct ProgressAnswer {
    /// What is stored now.
    pub progress: Progress,
    /// `false` when a newer position was already stored: this one is kept as
    /// the device's last position only.
    pub applied: bool,
}

/// This device's session (its refresh chain), or a personal token.
fn device_id(auth: &AuthUser) -> Uuid {
    auth.chain_id.unwrap_or(auth.session_id)
}

async fn apply_progress(
    state: &AppState,
    family: &FamilyContext,
    auth: &AuthUser,
    k: &str,
    id: Uuid,
    body: &ProgressRequest,
) -> Result<ProgressAnswer, Problem> {
    let k = kind(k, Kind::has_progress, "progress")?;
    visible(state, family, k, id).await?;
    let at = clamp(body.at);
    let (position_ms, file) = if k == Kind::Book {
        let timeline = user_state::book_timeline(state.db(), id).await?;
        match (body.file_id, body.file_position_ms, body.position_ms) {
            (Some(file), Some(in_file), _) => {
                let start = timeline.iter().find(|(f, _, _)| *f == file).map(|(_, s, _)| *s).ok_or_else(|| {
                    Problem::bad_request("invalid_body", "file_id is not a file of this book").field("/file_id", "invalid", file.to_string())
                })?;
                (start + in_file.max(0), Some((file, in_file.max(0))))
            }
            (_, _, Some(pos)) => {
                let located = user_state::locate(&timeline, pos.max(0))
                    .ok_or_else(|| Problem::new(StatusCode::CONFLICT, "conflict").detail("the book has no files"))?;
                (pos.max(0), Some(located))
            }
            _ => return Err(Problem::bad_request("invalid_body", "position_ms, or file_id with file_position_ms, is required")),
        }
    } else {
        let pos = body.position_ms.ok_or_else(|| Problem::bad_request("invalid_body", "position_ms is required").field("/position_ms", "required", "milliseconds"))?;
        (pos.max(0), None)
    };
    let (device_name, device_kind): (Option<String>, String) = match auth.chain_id {
        Some(chain) => sqlx::query_as(
            "SELECT device_name, device_kind FROM refresh_tokens WHERE chain_id = $1 AND user_id = $2 ORDER BY created_at DESC LIMIT 1",
        )
        .bind(chain)
        .bind(auth.user_id)
        .fetch_optional(state.db())
        .await
        .map_err(anyhow::Error::from)?
        .unwrap_or((None, "other".into())),
        None => (None, "other".into()),
    };
    user_state::put_device_position(
        state.db(),
        family.user_id,
        k,
        id,
        device_id(auth),
        device_name.as_deref(),
        &device_kind,
        position_ms,
        body.finished,
        at,
    )
    .await?;
    let (now, applied) = user_state::put_progress(state.db(), family.user_id, k, id, position_ms, body.finished, at, file).await?;
    Ok(ProgressAnswer { progress: progress(k, now), applied })
}

/// Save where the caller is. The later position wins; an older one is kept as
/// this device's last position.
#[utoipa::path(put, path = "/me/progress/{kind}/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    params(("kind" = String, Path, description = "`track`, `book` or `episode`"), ("id" = Uuid, Path)),
    request_body = ProgressRequest,
    responses((status = 200, body = ProgressAnswer), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn put_progress(
    auth: AuthUser,
    family: FamilyContext,
    State(state): State<AppState>,
    Path((k, id)): Path<(String, Uuid)>,
    Json(body): Json<ProgressRequest>,
) -> Result<Json<ProgressAnswer>, Problem> {
    Ok(Json(apply_progress(&state, &family, &auth, &k, id, &body).await?))
}

/// Start over: no position survives, on any device.
#[utoipa::path(delete, path = "/me/progress/{kind}/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    params(("kind" = String, Path), ("id" = Uuid, Path)),
    responses((status = 204, description = "Forgotten")))]
pub async fn delete_progress(family: FamilyContext, State(state): State<AppState>, Path((k, id)): Path<(String, Uuid)>) -> Result<StatusCode, Problem> {
    let k = kind(&k, Kind::has_progress, "progress")?;
    user_state::delete_progress(state.db(), family.user_id, k, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Plays ────────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct Play {
    /// The device's own id for this play: sending it again is harmless.
    pub client_id: String,
    /// `track`, `book` or `episode`.
    pub kind: String,
    pub id: Uuid,
    /// A book's file.
    pub file_id: Option<Uuid>,
    pub started_at: DateTime<Utc>,
    pub ended_at: DateTime<Utc>,
    /// Audio actually heard, which is not `ended_at - started_at` after a pause or at another speed.
    pub listened_ms: i64,
    pub speed: Option<f64>,
    /// `ios`, `android`, `web`, `macos`, `windows`, `tvos` or `other`.
    pub device_kind: Option<String>,
    /// Why it stopped, when known: `finished`, `skipped`, `paused`…
    pub ended_reason: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct PlaysRequest {
    /// Up to 500.
    pub plays: Vec<Play>,
}

#[derive(Serialize, ToSchema)]
pub struct ItemResult {
    pub client_id: String,
    /// `ok`, `duplicate` (already had it), or an error code: `not_found`, `invalid_body`, `unsupported_kind`.
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn result(client_id: &str, outcome: Result<&str, Problem>) -> ItemResult {
    match outcome {
        Ok(status) => ItemResult { client_id: client_id.to_string(), status: status.to_string(), detail: None },
        Err(p) => ItemResult { client_id: client_id.to_string(), status: p.code().to_string(), detail: None },
    }
}

async fn apply_play(state: &AppState, family: &FamilyContext, play: &Play) -> Result<&'static str, Problem> {
    let k = kind(&play.kind, Kind::has_progress, "plays")?;
    if play.client_id.trim().is_empty() || play.client_id.len() > 200 {
        return Err(Problem::bad_request("invalid_body", "client_id is 1 to 200 characters"));
    }
    if play.ended_at < play.started_at || play.listened_ms < 0 {
        return Err(Problem::bad_request("invalid_body", "a play ends after it starts and lasts zero or more"));
    }
    visible(state, family, k, play.id).await?;
    let (media_kind, item_id, part_id) = match k {
        Kind::Track => (db::access::MUSIC, play.id, None),
        Kind::Book => (db::access::AUDIOBOOK, play.id, play.file_id),
        _ => {
            let feed = db::playback::episode_feed_id(state.db(), play.id).await?.ok_or_else(Problem::not_found)?;
            (db::access::PODCAST, feed, Some(play.id))
        }
    };
    let device_kind = match play.device_kind.as_deref().map(str::trim) {
        Some(k @ ("web" | "ios" | "android" | "macos" | "windows" | "tvos")) => k.to_string(),
        _ => "other".to_string(),
    };
    let session = db::stats::SessionInput {
        media_kind: media_kind.to_string(),
        item_id,
        part_id,
        started_at: play.started_at,
        ended_at: play.ended_at,
        seconds_listened: i32::try_from(play.listened_ms / 1000).unwrap_or(i32::MAX),
        playback_speed: play.speed,
        device_kind,
        client_session_id: Some(play.client_id.clone()),
        ended_reason: play.ended_reason.clone(),
    };
    let new = db::stats::record_sessions(state.db(), family.user_id, std::slice::from_ref(&session)).await?;
    Ok(if new == 0 { "duplicate" } else { "ok" })
}

/// Report plays; each answers on its own, and a play sent twice counts once.
#[utoipa::path(post, path = "/me/plays", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    request_body = PlaysRequest,
    responses((status = 200, description = "One result per play, in order", body = Vec<ItemResult>),
        (status = 400, description = "More than 500 plays", body = crate::v2::problem::ProblemBody)))]
pub async fn post_plays(family: FamilyContext, State(state): State<AppState>, Json(body): Json<PlaysRequest>) -> Result<Json<Vec<ItemResult>>, Problem> {
    if body.plays.len() > 500 {
        return Err(Problem::bad_request("too_many", "at most 500 plays at a time"));
    }
    let mut out = Vec::with_capacity(body.plays.len());
    for play in &body.plays {
        out.push(result(&play.client_id, apply_play(&state, &family, play).await));
    }
    Ok(Json(out))
}

// ── Bookmarks ────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Bookmark {
    pub id: Uuid,
    pub kind: String,
    pub item_id: Uuid,
    /// From the start; for a book, across its files.
    pub position_ms: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<Uuid>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_position_ms: Option<i64>,
    pub label: Option<String>,
    pub has_audio_note: bool,
    pub created_at: DateTime<Utc>,
}

async fn bookmark(state: &AppState, row: user_state::BookmarkRow) -> Result<Bookmark, Problem> {
    let (k, item) = match (row.track_id, row.book_id, row.episode_id) {
        (Some(t), _, _) => (Kind::Track, t),
        (_, Some(b), _) => (Kind::Book, b),
        (_, _, Some(e)) => (Kind::Episode, e),
        _ => return Err(Problem::internal(anyhow::anyhow!("a bookmark without an item"))),
    };
    let in_file = (row.position_secs * 1000.0).round() as i64;
    let position_ms = match (k, row.file_id) {
        (Kind::Book, Some(file)) => {
            let timeline = user_state::book_timeline(state.db(), item).await?;
            timeline.iter().find(|(f, _, _)| *f == file).map_or(0, |(_, s, _)| *s) + in_file
        }
        _ => in_file,
    };
    Ok(Bookmark {
        id: row.id,
        kind: k.as_str().into(),
        item_id: item,
        position_ms,
        file_id: row.file_id,
        file_position_ms: row.file_id.map(|_| in_file),
        label: row.label,
        has_audio_note: row.has_audio_note,
        created_at: row.created_at,
    })
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BookmarksQuery {
    /// `track`, `book` or `episode`.
    pub kind: String,
    /// One item's bookmarks only.
    pub item_id: Option<Uuid>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// The caller's bookmarks, newest first.
#[utoipa::path(get, path = "/me/bookmarks", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(BookmarksQuery),
    responses((status = 200, body = Page<Bookmark>)))]
pub async fn list_bookmarks(family: FamilyContext, State(state): State<AppState>, Query(q): Query<BookmarksQuery>) -> Result<Json<Page<Bookmark>>, Problem> {
    let k = kind(&q.kind, Kind::has_progress, "bookmarks")?;
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let rows = user_state::list_bookmarks(state.db(), family.viewer(), k, q.item_id, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |b| (b.created_at, b.id));
    let mut items = Vec::with_capacity(page.items.len());
    for row in page.items {
        items.push(bookmark(&state, row).await?);
    }
    Ok(Json(Page { items, next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct BookmarkRequest {
    pub kind: String,
    pub item_id: Uuid,
    /// From the start; for a book, across its files. Or a book's `file_id` with `file_position_ms`.
    pub position_ms: Option<i64>,
    pub file_id: Option<Uuid>,
    pub file_position_ms: Option<i64>,
    pub label: Option<String>,
}

/// Where in the item, as stored: (file, seconds within it).
async fn place(state: &AppState, k: Kind, item: Uuid, position_ms: Option<i64>, file: Option<Uuid>, in_file: Option<i64>) -> Result<(Option<Uuid>, f64), Problem> {
    if k == Kind::Book {
        let timeline = user_state::book_timeline(state.db(), item).await?;
        let (file, ms) = match (file, in_file, position_ms) {
            (Some(f), Some(ms), _) if timeline.iter().any(|(id, _, _)| *id == f) => (f, ms.max(0)),
            (None, None, Some(pos)) => user_state::locate(&timeline, pos.max(0))
                .ok_or_else(|| Problem::new(StatusCode::CONFLICT, "conflict").detail("the book has no files"))?,
            _ => return Err(Problem::bad_request("invalid_body", "position_ms, or a file_id of this book with file_position_ms, is required")),
        };
        Ok((Some(file), ms as f64 / 1000.0))
    } else {
        let pos = position_ms.ok_or_else(|| Problem::bad_request("invalid_body", "position_ms is required"))?;
        Ok((None, pos.max(0) as f64 / 1000.0))
    }
}

/// Mark a place.
#[utoipa::path(post, path = "/me/bookmarks", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    request_body = BookmarkRequest,
    responses((status = 201, body = Bookmark), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn create_bookmark(family: FamilyContext, State(state): State<AppState>, Json(body): Json<BookmarkRequest>) -> Result<(StatusCode, Json<Bookmark>), Problem> {
    let k = kind(&body.kind, Kind::has_progress, "bookmarks")?;
    visible(&state, &family, k, body.item_id).await?;
    let (file, secs) = place(&state, k, body.item_id, body.position_ms, body.file_id, body.file_position_ms).await?;
    let label = body.label.as_deref().map(str::trim).filter(|l| !l.is_empty());
    let row = user_state::create_bookmark(state.db(), family.user_id, k, body.item_id, file, secs, label).await?;
    Ok((StatusCode::CREATED, Json(bookmark(&state, row).await?)))
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize, ToSchema)]
pub struct BookmarkPatch {
    /// `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub label: Option<Option<String>>,
    pub position_ms: Option<i64>,
    pub file_id: Option<Uuid>,
    pub file_position_ms: Option<i64>,
}

/// Rename or move a bookmark.
#[utoipa::path(patch, path = "/me/bookmarks/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    params(("id" = Uuid, Path)), request_body = BookmarkPatch,
    responses((status = 200, body = Bookmark), (status = 404, description = "Not the caller's", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_bookmark(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<BookmarkPatch>) -> Result<Json<Bookmark>, Problem> {
    let current = user_state::find_bookmark(state.db(), family.user_id, id).await?.ok_or_else(Problem::not_found)?;
    let (k, item) = match (current.track_id, current.book_id, current.episode_id) {
        (Some(t), _, _) => (Kind::Track, t),
        (_, Some(b), _) => (Kind::Book, b),
        (_, _, Some(e)) => (Kind::Episode, e),
        _ => return Err(Problem::not_found()),
    };
    let position = if body.position_ms.is_some() || body.file_id.is_some() {
        Some(place(&state, k, item, body.position_ms, body.file_id, body.file_position_ms).await?)
    } else {
        None
    };
    let label = body.label.as_ref().map(|l| l.as_deref().map(str::trim).filter(|l| !l.is_empty()));
    let row = user_state::update_bookmark(state.db(), family.user_id, id, label, position).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(bookmark(&state, row).await?))
}

/// Remove a bookmark.
#[utoipa::path(delete, path = "/me/bookmarks/{id}", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not the caller's", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_bookmark(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    if !user_state::delete_bookmark(state.db(), family.user_id, id).await? {
        return Err(Problem::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Offline changes ──────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct Change {
    /// The device's id for this change, echoed in its result.
    pub client_id: String,
    /// `favourite`, `unfavourite`, `rate`, `progress` or `play`.
    pub op: String,
    pub kind: String,
    pub id: Uuid,
    /// When it happened, by the device's clock.
    pub at: Option<DateTime<Utc>>,
    /// `rate`: 0 to 5.
    pub rating: Option<i16>,
    /// `progress`: as `PUT /me/progress`.
    pub position_ms: Option<i64>,
    pub file_id: Option<Uuid>,
    pub file_position_ms: Option<i64>,
    #[serde(default)]
    pub finished: bool,
    /// `play`: as `POST /me/plays`.
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub listened_ms: Option<i64>,
    pub speed: Option<f64>,
    pub device_kind: Option<String>,
    pub ended_reason: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct PushRequest {
    /// In the order they happened; up to 500.
    pub changes: Vec<Change>,
}

/// What a device changed while offline (DESIGN §8): favourites, ratings,
/// progress and plays, each applied in order and answered on its own.
#[utoipa::path(post, path = "/sync/push", tag = "sync", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("playback"))),
    request_body = PushRequest,
    responses((status = 200, description = "One result per change, in order: `ok`, `stale` (a newer one was already stored), `duplicate`, or an error code", body = Vec<ItemResult>),
        (status = 400, description = "More than 500 changes", body = crate::v2::problem::ProblemBody)))]
pub async fn push(auth: AuthUser, family: FamilyContext, State(state): State<AppState>, Json(body): Json<PushRequest>) -> Result<Json<Vec<ItemResult>>, Problem> {
    if body.changes.len() > 500 {
        return Err(Problem::bad_request("too_many", "at most 500 changes at a time"));
    }
    let mut out = Vec::with_capacity(body.changes.len());
    for c in &body.changes {
        let outcome: Result<&str, Problem> = match c.op.as_str() {
            "favourite" | "unfavourite" => apply_favourite(&state, &family, &c.kind, c.id, c.op == "favourite").await.map(|_| "ok"),
            "rate" => match c.rating {
                Some(r) => apply_rating(&state, &family, &c.kind, c.id, r).await.map(|_| "ok"),
                None => Err(Problem::bad_request("invalid_body", "rate needs a rating")),
            },
            "progress" => {
                let req = ProgressRequest { position_ms: c.position_ms, file_id: c.file_id, file_position_ms: c.file_position_ms, finished: c.finished, at: c.at };
                apply_progress(&state, &family, &auth, &c.kind, c.id, &req).await.map(|a| if a.applied { "ok" } else { "stale" })
            }
            "play" => match (c.started_at, c.ended_at, c.listened_ms) {
                (Some(started_at), Some(ended_at), Some(listened_ms)) => {
                    let play = Play {
                        client_id: c.client_id.clone(),
                        kind: c.kind.clone(),
                        id: c.id,
                        file_id: c.file_id,
                        started_at,
                        ended_at,
                        listened_ms,
                        speed: c.speed,
                        device_kind: c.device_kind.clone(),
                        ended_reason: c.ended_reason.clone(),
                    };
                    apply_play(&state, &family, &play).await
                }
                _ => Err(Problem::bad_request("invalid_body", "play needs started_at, ended_at and listened_ms")),
            },
            other => Err(Problem::bad_request("unsupported_op", format!("`{other}` is not an operation"))),
        };
        out.push(result(&c.client_id, outcome));
    }
    Ok(Json(out))
}
