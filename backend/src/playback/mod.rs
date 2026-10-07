// SPDX-License-Identifier: AGPL-3.0-or-later
/// Playback module — progress, bookmarks, continue listening, speed prefs, queue.
pub mod models;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use axum::extract::{Json, Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// ── DTOs ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
#[schema(as = PlaybackUpsertProgressRequest)]
pub struct UpsertProgressRequest {
    pub position_secs: f64,
    #[serde(default)]
    pub completed: bool,
    /// For audiobook progress: which file is playing.
    pub file_id: Option<Uuid>,
    /// `web`, `ios`, `android`, … — recorded on any session derived from
    /// this save, so stats can break down by platform.
    #[serde(default)]
    pub device_kind: Option<String>,
}

#[derive(Serialize, ToSchema)]
#[schema(as = PlaybackProgressResponse)]
pub struct ProgressResponse {
    pub media_id: String,
    pub position_secs: f64,
    pub completed: bool,
    pub updated_at: String,
    /// Only set for audiobook progress.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct BookmarkResponse {
    pub id: String,
    pub episode_id: Option<String>,
    pub book_id: Option<String>,
    pub file_id: Option<String>,
    pub position_secs: f64,
    pub label: Option<String>,
    pub audio_url: Option<String>,
    pub created_at: String,
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Episode progress
        .routes(routes!(get_episode_progress, upsert_episode_progress))
        // Audiobook progress
        .routes(routes!(list_book_progress))
        .routes(routes!(get_book_progress, upsert_book_progress, reset_book_progress))
        // Bookmarks
        .routes(routes!(list_bookmarks, create_bookmark_handler))
        .routes(routes!(update_bookmark_handler, delete_bookmark))
        // Book-specific bookmarks
        .routes(routes!(list_book_bookmarks))
        // User settings (includes audiobook defaults)
        .routes(routes!(get_settings))
        .routes(routes!(update_audiobook_defaults))
        .routes(routes!(get_eq_settings_handler, update_eq_settings_handler))
        // Listening history
        .routes(routes!(report_sessions))
        // Cross-device play queue
        .routes(routes!(get_queue, put_queue))
        // Bulk operations — multi-select in a mobile UI must not fan out
        // into N round trips over cellular.
        .routes(routes!(bulk_episode_progress))
}

// ── Bulk operations ───────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BulkEpisodeProgressRequest {
    pub episode_ids: Vec<Uuid>,
    /// Mark them played (true) or unplayed (false).
    pub completed: bool,
}

#[derive(Serialize, ToSchema)]
pub struct BulkResponse {
    pub updated: u64,
}

/// POST /api/v1/playback/episodes/progress/bulk
/// Mark many episodes played or unplayed in one request.
#[utoipa::path(post, path = "/episodes/progress/bulk", tag = "playback", security(("bearer" = [])),
    request_body = BulkEpisodeProgressRequest,
    responses((status = 200, body = BulkResponse), (status = 400, description = "More than 1000 episode ids", body = crate::http::openapi::ErrorBody)))]
async fn bulk_episode_progress(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<BulkEpisodeProgressRequest>,
) -> Result<Json<BulkResponse>, AuthError> {
    const MAX_IDS: usize = 1000;

    if body.episode_ids.is_empty() {
        return Ok(Json(BulkResponse { updated: 0 }));
    }
    if body.episode_ids.len() > MAX_IDS {
        return Err(AuthError::BadRequest(format!(
            "at most {MAX_IDS} episodes per request"
        )));
    }

    let updated = db::playback::bulk_set_episode_completed(
        state.db(),
        auth.user_id,
        &body.episode_ids,
        body.completed,
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(Json(BulkResponse { updated }))
}

// ── Play queue ────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, ToSchema)]
pub struct QueueItem {
    pub media_kind: String,
    pub item_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub part_id: Option<Uuid>,
}

#[derive(Deserialize, ToSchema)]
pub struct PutQueueRequest {
    pub items: Vec<QueueItem>,
    #[serde(default)]
    pub current_index: i32,
    #[serde(default)]
    pub position_secs: f64,
    /// `web`, `ios`, `android`, … — echoed back so another device can show
    /// "playing on your phone".
    #[serde(default)]
    pub device_kind: Option<String>,
    /// Optional human-readable device name (e.g. "Kornel's Pixel"), free-form
    /// and unvalidated — distinguishes two devices of the same `device_kind`
    /// on one account. Never send a raw model id here; omit instead.
    #[serde(default)]
    pub device_label: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct QueueResponse {
    pub items: Vec<QueueItem>,
    pub current_index: i32,
    pub position_secs: f64,
    pub updated_by_device: Option<String>,
    pub updated_by_device_label: Option<String>,
    /// Echo this back on the next write to detect a concurrent change.
    pub updated_at: String,
}

/// GET /api/v1/playback/queue — the caller's cross-device play queue.
/// An empty queue is returned rather than 404, so clients need no special case.
#[utoipa::path(get, path = "/queue", tag = "playback", security(("bearer" = [])),
    responses((status = 200, body = QueueResponse)))]
async fn get_queue(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<QueueResponse>, AuthError> {
    let queue = db::sync::get_queue(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(match queue {
        Some(q) => QueueResponse {
            items: serde_json::from_value(q.items).unwrap_or_default(),
            current_index: q.current_index,
            position_secs: q.position_secs,
            updated_by_device: q.updated_by_device,
            updated_by_device_label: q.updated_by_device_label,
            updated_at: q.updated_at.to_rfc3339(),
        },
        None => QueueResponse {
            items: Vec::new(),
            current_index: 0,
            position_secs: 0.0,
            updated_by_device: None,
            updated_by_device_label: None,
            updated_at: chrono::Utc::now().to_rfc3339(),
        },
    }))
}

/// PUT /api/v1/playback/queue — replace the queue wholesale.
///
/// Last write wins. A queue is small and always edited as a unit, so merging
/// concurrent edits would invent an order neither device asked for; the
/// returned `updated_at` lets a client notice it was overtaken.
#[utoipa::path(put, path = "/queue", tag = "playback", security(("bearer" = [])),
    request_body = PutQueueRequest,
    responses((status = 200, body = QueueResponse), (status = 400, description = "More than 1000 items, or an unknown media_kind", body = crate::http::openapi::ErrorBody)))]
async fn put_queue(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<PutQueueRequest>,
) -> Result<Json<QueueResponse>, AuthError> {
    const MAX_ITEMS: usize = 1000;

    if body.items.len() > MAX_ITEMS {
        return Err(AuthError::BadRequest(format!(
            "queue may hold at most {MAX_ITEMS} items"
        )));
    }
    for item in &body.items {
        if !db::access::is_valid_kind(&item.media_kind) {
            return Err(AuthError::BadRequest(
                "media_kind must be 'audiobook', 'podcast', or 'music'".into(),
            ));
        }
    }

    // Clamp rather than reject: a queue trimmed on another device can leave a
    // stale index, and failing the save would strand the client.
    let current_index = body
        .current_index
        .clamp(0, body.items.len().saturating_sub(1) as i32);

    let items = serde_json::to_value(&body.items)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("serialize queue: {e}")))?;

    let device_kind = normalize_device_kind(body.device_kind.as_deref());
    let device_label = body
        .device_label
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(100).collect::<String>());

    let saved = db::sync::put_queue(
        state.db(),
        auth.user_id,
        &items,
        current_index,
        body.position_secs.max(0.0),
        Some(&device_kind),
        device_label.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(Json(QueueResponse {
        items: serde_json::from_value(saved.items).unwrap_or_default(),
        current_index: saved.current_index,
        position_secs: saved.position_secs,
        updated_by_device: saved.updated_by_device,
        updated_by_device_label: saved.updated_by_device_label,
        updated_at: saved.updated_at.to_rfc3339(),
    }))
}

// ── Listening sessions ────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ReportSessionsRequest {
    pub sessions: Vec<SessionReport>,
}

#[derive(Deserialize, ToSchema)]
pub struct SessionReport {
    /// `audiobook`, `podcast`, or `music`.
    pub media_kind: String,
    pub item_id: Uuid,
    /// Audiobook file or podcast episode, when known.
    #[serde(default)]
    pub part_id: Option<Uuid>,
    pub started_at: chrono::DateTime<chrono::Utc>,
    pub ended_at: chrono::DateTime<chrono::Utc>,
    /// Audio actually consumed, which differs from the wall-clock span when
    /// the listener paused or played at a non-1.0 speed.
    pub seconds_listened: i32,
    #[serde(default)]
    pub playback_speed: Option<f64>,
    /// `web`, `ios`, `android`, or `other`.
    #[serde(default)]
    pub device_kind: Option<String>,
    /// Client-generated idempotency key. **Send one.** Android WorkManager
    /// and iOS background tasks both retry on failure, and this is what stops
    /// a retried batch from counting twice.
    #[serde(default)]
    pub client_session_id: Option<String>,
    /// Why playback stopped: `completed`, `skipped`, `stopped` or `replaced`.
    ///
    /// Send the fact, not a judgement — how much an early skip counts against
    /// a track is derived here from `seconds_listened`, so that curve can be
    /// retuned without an app update. Omit it when genuinely unknown; a wrong
    /// value teaches the preference model something false, a missing one only
    /// costs a little signal.
    #[serde(default)]
    pub ended_reason: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ReportSessionsResponse {
    /// Rows actually stored; duplicates from a retry are silently skipped.
    pub recorded: u64,
    pub received: usize,
}

/// POST /api/v1/playback/sessions
///
/// Batched so a device that was offline, backgrounded, or in Doze can flush
/// everything it accumulated in a single request.
#[utoipa::path(post, path = "/sessions", tag = "playback", security(("bearer" = [])),
    request_body = ReportSessionsRequest,
    responses((status = 200, body = ReportSessionsResponse), (status = 400, description = "More than 500 sessions, an unknown media_kind, ended_at before started_at, or negative seconds_listened", body = crate::http::openapi::ErrorBody)))]
async fn report_sessions(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ReportSessionsRequest>,
) -> Result<Json<ReportSessionsResponse>, AuthError> {
    const MAX_BATCH: usize = 500;

    if body.sessions.is_empty() {
        return Ok(Json(ReportSessionsResponse {
            recorded: 0,
            received: 0,
        }));
    }
    if body.sessions.len() > MAX_BATCH {
        return Err(AuthError::BadRequest(format!(
            "at most {MAX_BATCH} sessions per request"
        )));
    }

    let mut inputs = Vec::with_capacity(body.sessions.len());
    for s in &body.sessions {
        if !db::access::is_valid_kind(&s.media_kind) {
            return Err(AuthError::BadRequest(
                "media_kind must be 'audiobook', 'podcast', or 'music'".into(),
            ));
        }
        if s.ended_at < s.started_at {
            return Err(AuthError::BadRequest(
                "ended_at must not precede started_at".into(),
            ));
        }
        if s.seconds_listened < 0 {
            return Err(AuthError::BadRequest(
                "seconds_listened must not be negative".into(),
            ));
        }

        inputs.push(db::stats::SessionInput {
            media_kind: s.media_kind.clone(),
            item_id: s.item_id,
            part_id: s.part_id,
            started_at: s.started_at,
            ended_at: s.ended_at,
            seconds_listened: s.seconds_listened,
            playback_speed: s.playback_speed,
            device_kind: normalize_device_kind(s.device_kind.as_deref()),
            client_session_id: s.client_session_id.clone(),
            ended_reason: normalize_ended_reason(s.ended_reason.as_deref()),
        });
    }

    let recorded = db::stats::record_sessions(state.db(), auth.user_id, &inputs)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(ReportSessionsResponse {
        recorded,
        received: body.sessions.len(),
    }))
}

/// Unknown values collapse to `other` rather than erroring — a client sending a
/// kind we don't know about should still get its progress saved. Keep in sync
/// with the login allowlist in `auth::issue_tokens`, minus `subsonic`, which is
/// a playback source and never a login.
/// Unlike `normalize_device_kind`, an unrecognised value becomes `None`, not a
/// catch-all. Bucketing an unknown reason as `stopped` would be a silent lie to
/// the preference model; dropping it merely loses one data point.
fn normalize_ended_reason(value: Option<&str>) -> Option<String> {
    match value.map(str::trim) {
        Some(r @ ("completed" | "skipped" | "stopped" | "replaced")) => Some(r.to_string()),
        _ => None,
    }
}

fn normalize_device_kind(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some(k @ ("web" | "ios" | "android" | "macos" | "windows" | "tvos" | "subsonic")) => k.to_string(),
        _ => "other".to_string(),
    }
}

// ── Episode progress ──────────────────────────────────────────────────────

/// The caller's position in one podcast episode.
#[utoipa::path(get, path = "/episodes/{episode_id}/progress", tag = "playback", security(("bearer" = [])),
    params(("episode_id" = Uuid, Path, description = "Podcast episode id")),
    responses((status = 200, body = ProgressResponse), (status = 404, description = "The caller has no progress on this episode", body = crate::http::openapi::ErrorBody)))]
async fn get_episode_progress(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(episode_id): Path<Uuid>,
) -> Result<Json<ProgressResponse>, AuthError> {
    let progress = db::playback::get_podcast_progress(state.db(), auth.user_id, episode_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(ProgressResponse {
        media_id: progress.episode_id.to_string(),
        position_secs: progress.position_secs,
        completed: progress.completed,
        updated_at: progress.updated_at.to_rfc3339(),
        file_id: None,
    }))
}

/// Saves the caller's position in one podcast episode.
#[utoipa::path(put, path = "/episodes/{episode_id}/progress", tag = "playback", security(("bearer" = [])),
    params(("episode_id" = Uuid, Path, description = "Podcast episode id")),
    request_body = UpsertProgressRequest,
    responses((status = 200, body = ProgressResponse)))]
async fn upsert_episode_progress(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(episode_id): Path<Uuid>,
    Json(body): Json<UpsertProgressRequest>,
) -> Result<Json<ProgressResponse>, AuthError> {
    // Capture the previous position first: the jump between saves is what
    // becomes a derived listening session.
    let previous = db::playback::get_podcast_progress(state.db(), auth.user_id, episode_id)
        .await
        .map_err(AuthError::Internal)?
        .map(|p| p.position_secs)
        .unwrap_or(0.0);

    let progress = db::playback::upsert_podcast_progress(
        state.db(),
        auth.user_id,
        episode_id,
        body.position_secs,
        body.completed,
    )
    .await
    .map_err(AuthError::Internal)?;

    if let Some(feed_id) = db::playback::episode_feed_id(state.db(), episode_id)
        .await
        .map_err(AuthError::Internal)?
    {
        let _ = db::stats::derive_from_progress(
            state.db(),
            auth.user_id,
            db::access::PODCAST,
            feed_id,
            Some(episode_id),
            body.position_secs - previous,
            &normalize_device_kind(body.device_kind.as_deref()),
        )
        .await;
    }

    Ok(Json(ProgressResponse {
        media_id: progress.episode_id.to_string(),
        position_secs: progress.position_secs,
        completed: progress.completed,
        updated_at: progress.updated_at.to_rfc3339(),
        file_id: None,
    }))
}

// ── Audiobook progress ────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct BookProgressSummary {
    pub book_id: String,
    /// Counted across the whole book, so a client can divide it by the book's total duration.
    pub position_secs: f64,
    /// Which file the position below is in — what resuming has to seek to.
    pub file_id: String,
    /// The raw stored position inside `file_id`, which is what `PUT .../progress` writes and
    /// what a player seeks to. `position_secs` is the same moment measured from the start of
    /// the book; a client that only syncs the shelf has no file list to convert between them.
    pub file_position_secs: f64,
    pub completed: bool,
    /// Whoever wrote last wins. A device holding an unsent position of its own compares against
    /// this before letting the server's value replace it.
    pub updated_at: String,
}

/// GET /api/v1/playback/books/progress
///
/// Every book this user has started, in one request — what a shelf needs to mark the finished
/// ones and show how far into the rest they are. `GET /audiobooks` deliberately stays metadata
/// only; progress is per-listener, and the book list is shared across a family.
#[utoipa::path(get, path = "/books/progress", tag = "playback", security(("bearer" = [])),
    responses((status = 200, body = Vec<BookProgressSummary>)))]
async fn list_book_progress(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<BookProgressSummary>>, AuthError> {
    let rows = db::playback::list_audiobook_progress(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        rows.into_iter()
            .map(
                |(book_id, file_id, position_secs, file_position_secs, completed, updated_at)| {
                    BookProgressSummary {
                        book_id: book_id.to_string(),
                        position_secs,
                        file_id: file_id.to_string(),
                        file_position_secs,
                        completed,
                        updated_at: updated_at.to_rfc3339(),
                    }
                },
            )
            .collect(),
    ))
}

/// The caller's position in one audiobook.
#[utoipa::path(get, path = "/books/{book_id}/progress", tag = "playback", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Audiobook id")),
    responses((status = 200, description = "`file_id` is set", body = ProgressResponse), (status = 404, description = "The caller has no progress on this book", body = crate::http::openapi::ErrorBody)))]
async fn get_book_progress(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<ProgressResponse>, AuthError> {
    let progress = db::playback::get_audiobook_progress(state.db(), auth.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(ProgressResponse {
        media_id: progress.book_id.to_string(),
        position_secs: progress.position_secs,
        completed: progress.completed,
        updated_at: progress.updated_at.to_rfc3339(),
        file_id: Some(progress.file_id.to_string()),
    }))
}

/// Saves the caller's position in one audiobook; `file_id` is required.
#[utoipa::path(put, path = "/books/{book_id}/progress", tag = "playback", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Audiobook id")),
    request_body = UpsertProgressRequest,
    responses((status = 200, body = ProgressResponse), (status = 500, description = "No `file_id` in the body", body = crate::http::openapi::ErrorBody)))]
async fn upsert_book_progress(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    Json(body): Json<UpsertProgressRequest>,
) -> Result<Json<ProgressResponse>, AuthError> {
    let file_id = body
        .file_id
        .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("file_id required for audiobook progress")))?;

    let previous = db::playback::get_audiobook_progress(state.db(), auth.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?
        .filter(|p| p.file_id == file_id)
        .map(|p| p.position_secs)
        .unwrap_or(0.0);

    let progress = db::playback::upsert_audiobook_progress(
        state.db(),
        auth.user_id,
        book_id,
        file_id,
        body.position_secs,
        body.completed,
    )
    .await
    .map_err(AuthError::Internal)?;

    let _ = db::stats::derive_from_progress(
        state.db(),
        auth.user_id,
        db::access::AUDIOBOOK,
        book_id,
        Some(file_id),
        body.position_secs - previous,
        &normalize_device_kind(body.device_kind.as_deref()),
    )
    .await;

    Ok(Json(ProgressResponse {
        media_id: progress.book_id.to_string(),
        position_secs: progress.position_secs,
        completed: progress.completed,
        updated_at: progress.updated_at.to_rfc3339(),
        file_id: Some(progress.file_id.to_string()),
    }))
}

/// DELETE /api/v1/playback/books/:id/progress — "start over": no position or file survives, so
/// the next play begins at file one, position zero.
#[utoipa::path(delete, path = "/books/{book_id}/progress", tag = "playback", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Audiobook id")),
    responses((status = 204, description = "Reset")))]
async fn reset_book_progress(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::playback::delete_audiobook_progress(state.db(), auth.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

// ── Bookmarks ─────────────────────────────────────────────────────────────

/// All of the caller's bookmarks.
#[utoipa::path(get, path = "/bookmarks", tag = "playback", security(("bearer" = [])),
    responses((status = 200, body = Vec<BookmarkResponse>)))]
async fn list_bookmarks(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<BookmarkResponse>>, AuthError> {
    let bookmarks = db::playback::list_bookmarks_for_user(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::with_capacity(bookmarks.len());
    for b in bookmarks {
        responses.push(bookmark_to_response(b));
    }
    Ok(Json(responses))
}

/// Deletes one of the caller's bookmarks.
#[utoipa::path(delete, path = "/bookmarks/{id}", tag = "playback", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Bookmark id")),
    responses((status = 204, description = "Deleted")))]
async fn delete_bookmark(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::playback::delete_bookmark(state.db(), id, auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /bookmarks — create a text bookmark.
#[utoipa::path(post, path = "/bookmarks", tag = "playback", security(("bearer" = [])),
    request_body = CreateBookmarkRequest,
    responses((status = 200, body = BookmarkResponse)))]
async fn create_bookmark_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<CreateBookmarkRequest>,
) -> Result<Json<BookmarkResponse>, AuthError> {
    let bookmark = db::playback::create_bookmark(
        state.db(),
        auth.user_id,
        body.book_id,
        body.episode_id,
        body.file_id,
        body.position_secs,
        body.label.as_deref(),
        None, // audio_object_id — premium feature
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(Json(bookmark_to_response(bookmark)))
}

#[derive(Deserialize, ToSchema)]
pub struct CreateBookmarkRequest {
    pub book_id: Option<Uuid>,
    pub episode_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub position_secs: f64,
    pub label: Option<String>,
}

/// PUT /bookmarks/{id} — update a bookmark label.
#[utoipa::path(put, path = "/bookmarks/{id}", tag = "playback", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Bookmark id")),
    request_body = UpdateBookmarkRequest,
    responses((status = 204, description = "Updated")))]
async fn update_bookmark_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBookmarkRequest>,
) -> Result<StatusCode, AuthError> {
    db::playback::update_bookmark_label(state.db(), id, auth.user_id, body.label.as_deref())
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
struct UpdateBookmarkRequest {
    label: Option<String>,
}

/// GET /books/{book_id}/bookmarks — list bookmarks for a specific book.
#[utoipa::path(get, path = "/books/{book_id}/bookmarks", tag = "playback", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Audiobook id")),
    responses((status = 200, body = Vec<BookmarkResponse>)))]
async fn list_book_bookmarks(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<Vec<BookmarkResponse>>, AuthError> {
    let bookmarks = db::playback::list_bookmarks_for_book(state.db(), auth.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::with_capacity(bookmarks.len());
    for b in bookmarks {
        responses.push(bookmark_to_response(b));
    }
    Ok(Json(responses))
}

// ── User settings ─────────────────────────────────────────────────────────

/// The caller's playback settings, created with defaults on first read.
#[utoipa::path(get, path = "/settings", tag = "playback", security(("bearer" = [])),
    responses((status = 200, description = "`playback_speed`, `skip_intro_secs`, `skip_outro_secs`, `ab_skip_forward_secs`, `ab_skip_backward_secs`, `ab_playback_speed`, `updated_at`", body = Object)))]
async fn get_settings(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, AuthError> {
    let settings = db::playback::get_or_create_settings(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(serde_json::json!({
        "playback_speed": settings.playback_speed,
        "skip_intro_secs": settings.skip_intro_secs,
        "skip_outro_secs": settings.skip_outro_secs,
        "ab_skip_forward_secs": settings.ab_skip_forward_secs,
        "ab_skip_backward_secs": settings.ab_skip_backward_secs,
        "ab_playback_speed": settings.ab_playback_speed,
        "updated_at": settings.updated_at.to_rfc3339(),
    })))
}

// ── Audiobook player settings ─────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
struct UpdateAudiobookDefaultsRequest {
    ab_skip_forward_secs: i32,
    ab_skip_backward_secs: i32,
    ab_playback_speed: f64,
}

/// Sets the audiobook player's skip intervals and speed.
#[utoipa::path(put, path = "/settings/audiobook-defaults", tag = "playback", security(("bearer" = [])),
    request_body = UpdateAudiobookDefaultsRequest,
    responses((status = 200, description = "`ab_skip_forward_secs`, `ab_skip_backward_secs`, `ab_playback_speed`, as stored after clamping (skips 1–120 s, speed 0.5–3.0)", body = Object)))]
async fn update_audiobook_defaults(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateAudiobookDefaultsRequest>,
) -> Result<Json<serde_json::Value>, AuthError> {
    let settings = db::playback::update_audiobook_defaults(
        state.db(),
        auth.user_id,
        body.ab_skip_forward_secs.clamp(1, 120),
        body.ab_skip_backward_secs.clamp(1, 120),
        body.ab_playback_speed.clamp(0.5, 3.0),
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(Json(serde_json::json!({
        "ab_skip_forward_secs": settings.ab_skip_forward_secs,
        "ab_skip_backward_secs": settings.ab_skip_backward_secs,
        "ab_playback_speed": settings.ab_playback_speed,
    })))
}

// ── Equalizer settings ─────────────────────────────────────────────────────

/// Same idea as `normalize_device_kind` but excludes `subsonic` — the
/// equalizer belongs to a first-party client, not a generic Subsonic client,
/// and `eq_settings`' CHECK constraint doesn't allow that value.
fn normalize_eq_device_kind(value: Option<&str>) -> String {
    match value.map(str::trim) {
        Some(k @ ("web" | "ios" | "android" | "macos" | "windows" | "tvos")) => k.to_string(),
        _ => "other".to_string(),
    }
}

#[derive(Serialize, ToSchema)]
struct EqSettingsResponseDto {
    device_kind: String,
    enabled: bool,
    preamp_db: f64,
    band1_db: f64,
    band2_db: f64,
    band3_db: f64,
    band4_db: f64,
    band5_db: f64,
    band6_db: f64,
    preset_name: Option<String>,
    updated_at: String,
}

impl From<crate::playback::models::EqSettings> for EqSettingsResponseDto {
    fn from(s: crate::playback::models::EqSettings) -> Self {
        Self {
            device_kind: s.device_kind,
            enabled: s.enabled,
            preamp_db: s.preamp_db,
            band1_db: s.band1_db,
            band2_db: s.band2_db,
            band3_db: s.band3_db,
            band4_db: s.band4_db,
            band5_db: s.band5_db,
            band6_db: s.band6_db,
            preset_name: s.preset_name,
            updated_at: s.updated_at.to_rfc3339(),
        }
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct EqSettingsQuery {
    device_kind: Option<String>,
}

/// GET /playback/settings/eq?device_kind=macos — this device's saved curve,
/// or flat/off defaults if it has never saved one.
#[utoipa::path(get, path = "/settings/eq", tag = "playback", security(("bearer" = [])),
    params(EqSettingsQuery),
    responses((status = 200, body = EqSettingsResponseDto)))]
async fn get_eq_settings_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(query): Query<EqSettingsQuery>,
) -> Result<Json<EqSettingsResponseDto>, AuthError> {
    let device_kind = normalize_eq_device_kind(query.device_kind.as_deref());
    let settings = db::playback::get_eq_settings(state.db(), auth.user_id, &device_kind)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(settings.into()))
}

#[derive(Deserialize, ToSchema)]
struct UpdateEqSettingsRequest {
    device_kind: Option<String>,
    enabled: bool,
    preamp_db: f64,
    band1_db: f64,
    band2_db: f64,
    band3_db: f64,
    band4_db: f64,
    band5_db: f64,
    band6_db: f64,
    preset_name: Option<String>,
}

/// PUT /playback/settings/eq — upserts this device's curve. Gains are
/// clamped server-side (never trust the client), matching the pattern
/// `update_audiobook_defaults` already uses for its own ranges.
#[utoipa::path(put, path = "/settings/eq", tag = "playback", security(("bearer" = [])),
    request_body = UpdateEqSettingsRequest,
    responses((status = 200, description = "The stored curve; gains clamped to ±12 dB", body = EqSettingsResponseDto)))]
async fn update_eq_settings_handler(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateEqSettingsRequest>,
) -> Result<Json<EqSettingsResponseDto>, AuthError> {
    let device_kind = normalize_eq_device_kind(body.device_kind.as_deref());
    let clamp = |v: f64| v.clamp(-12.0, 12.0);

    let settings = db::playback::upsert_eq_settings(
        state.db(),
        auth.user_id,
        &device_kind,
        body.enabled,
        clamp(body.preamp_db),
        clamp(body.band1_db),
        clamp(body.band2_db),
        clamp(body.band3_db),
        clamp(body.band4_db),
        clamp(body.band5_db),
        clamp(body.band6_db),
        body.preset_name.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(Json(settings.into()))
}

fn bookmark_to_response(b: crate::playback::models::Bookmark) -> BookmarkResponse {
    BookmarkResponse {
        id: b.id.to_string(),
        episode_id: b.episode_id.map(|u| u.to_string()),
        book_id: b.book_id.map(|u| u.to_string()),
        file_id: b.file_id.map(|u| u.to_string()),
        position_secs: b.position_secs,
        label: b.label,
        audio_url: None,
        created_at: b.created_at.to_rfc3339(),
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_ended_reason;

    #[test]
    fn a_known_reason_survives() {
        for r in ["completed", "skipped", "stopped", "replaced"] {
            assert_eq!(normalize_ended_reason(Some(r)).as_deref(), Some(r));
        }
        assert_eq!(normalize_ended_reason(Some("  skipped ")).as_deref(), Some("skipped"));
    }

    /// The asymmetry with `normalize_device_kind` is deliberate: an unknown
    /// device is harmlessly 'other', but an unknown reason coerced into a real
    /// bucket would teach the preference model something false. It must drop.
    #[test]
    fn an_unknown_reason_drops_rather_than_bucketing() {
        for r in ["", "SKIPPED", "abandoned", "next", "completed!"] {
            assert_eq!(normalize_ended_reason(Some(r)), None, "{r:?} must not be kept");
        }
        assert_eq!(normalize_ended_reason(None), None);
    }
}
