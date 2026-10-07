// SPDX-License-Identifier: AGPL-3.0-or-later
/// The server side of the own.audio folder — docs/file-sync-plan.md §5.
///
/// `GET /sync/tree` is what a sync client (the Mac's Finder extension, later
/// the Docker agent) builds the folder from; `/sync/tree/ids` is its periodic
/// full check; `/sync/shortcuts` decides which family items it shows;
/// `/sync/holdings` records which of the user's devices keep what offline;
/// `/sync/files` holds the images, booklets and lyrics kept next to the audio.
///
/// Downloading uses the existing stream routes, which return a presigned `GET`
/// that honours `Range` (§5.7): `/music/tracks/{id}/stream`,
/// `/audiobooks/{book}/files/{file}/stream`,
/// `/podcasts/{show}/episodes/{episode}/stream`, `/sync/files/{id}/stream`.
pub mod companion;
pub mod holdings;
pub mod organise;
pub mod paths;
pub mod shortcuts;
pub mod tree;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::families::FamilyContext;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_tree))
        .routes(routes!(get_tree_ids))
        .routes(routes!(list_shortcuts, add_shortcut))
        .routes(routes!(remove_shortcut))
        .routes(routes!(get_holdings, put_holdings))
        .routes(routes!(companion::create))
        .routes(routes!(companion::delete))
        .routes(routes!(companion::stream))
        .routes(routes!(companion::set_visibility))
        .routes(routes!(companion::use_as_cover))
        .routes(routes!(organise_paths))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct TreeQuery {
    /// The previous page's `cursor`; omitted for a full snapshot.
    #[serde(default)]
    cursor: Option<String>,
    /// Items per page, 1–2000, default 500.
    #[serde(default)]
    limit: Option<i64>,
}

/// GET /api/v1/sync/tree?cursor=&limit=
///
/// One page of the caller's own.audio folder: what changed since the cursor.
#[utoipa::path(get, path = "/tree", tag = "sync", security(("bearer" = [])),
    params(TreeQuery),
    responses((status = 200, body = tree::TreeResponse),
        (status = 400, description = "Invalid cursor: start again without one", body = crate::http::openapi::ErrorBody)))]
async fn get_tree(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<TreeQuery>,
) -> Result<Json<tree::TreeResponse>, AuthError> {
    let limit = q.limit.unwrap_or(tree::DEFAULT_LIMIT);
    match tree::read(state.db(), family.viewer(), q.cursor.as_deref(), limit)
        .await
        .map_err(AuthError::Internal)?
    {
        Ok(response) => Ok(Json(response)),
        Err(tree::CursorError::Invalid) => Err(AuthError::BadRequest(
            "invalid cursor — start again without one".to_string(),
        )),
    }
}

/// GET /api/v1/sync/tree/ids
///
/// Every item in the caller's tree, id only, for the periodic full check.
#[utoipa::path(get, path = "/tree/ids", tag = "sync", security(("bearer" = [])),
    responses((status = 200, body = Vec<tree::TreeId>, description = "Streamed JSON array")))]
async fn get_tree_ids(family: FamilyContext, State(state): State<AppState>) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (tx, rx) = tokio::sync::mpsc::channel(1024);
    tokio::spawn(tree::send_all_ids(state.db().clone(), family.viewer(), tx));
    let body = crate::http::json_stream::array(rx, Vec::new(), b"", "sync tree ids", |id| id);
    ([(axum::http::header::CONTENT_TYPE, "application/json")], body).into_response()
}

/// GET /api/v1/sync/shortcuts
///
/// The family items the caller has chosen to show in their folder.
#[utoipa::path(get, path = "/shortcuts", tag = "sync", security(("bearer" = [])),
    responses((status = 200, body = Vec<shortcuts::Shortcut>)))]
async fn list_shortcuts(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<shortcuts::Shortcut>>, AuthError> {
    shortcuts::list(state.db(), family.user_id).await.map(Json).map_err(AuthError::Internal)
}

/// POST /api/v1/sync/shortcuts — 201 when new, 200 when it already existed.
#[utoipa::path(post, path = "/shortcuts", tag = "sync", security(("bearer" = [])),
    request_body = shortcuts::NewShortcut,
    responses((status = 201, body = shortcuts::Shortcut, description = "Added"),
        (status = 200, body = shortcuts::Shortcut, description = "Already there"),
        (status = 400, body = crate::http::openapi::ErrorBody),
        (status = 404, description = "No such member or item visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn add_shortcut(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<shortcuts::NewShortcut>,
) -> Result<(StatusCode, Json<shortcuts::Shortcut>), AuthError> {
    match shortcuts::add(state.db(), family.viewer(), body).await.map_err(AuthError::Internal)? {
        Ok((s, true)) => Ok((StatusCode::CREATED, Json(s))),
        Ok((s, false)) => Ok((StatusCode::OK, Json(s))),
        Err(shortcuts::Refusal::Invalid(why)) => Err(AuthError::BadRequest(why.to_string())),
        Err(shortcuts::Refusal::NotFound) => Err(AuthError::ItemNotFound),
    }
}

/// POST /api/v1/sync/paths/organise — `{kind: "music"|"audiobook", preview, ids?}`: the
/// caller's own items moved to their default paths; with `preview` nothing changes.
#[utoipa::path(post, path = "/paths/organise", tag = "sync", security(("bearer" = [])),
    request_body = organise::Request,
    responses((status = 200, body = Vec<organise::Move>)))]
async fn organise_paths(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<organise::Request>,
) -> Result<Json<Vec<organise::Move>>, AuthError> {
    organise::organise(state.db(), family.user_id, &body).await.map(Json).map_err(AuthError::Internal)
}

/// DELETE /api/v1/sync/shortcuts/{id}
///
/// Stop showing a family item in the caller's folder.
#[utoipa::path(delete, path = "/shortcuts/{id}", tag = "sync", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Shortcut id")),
    responses((status = 204, description = "Removed"),
        (status = 404, body = crate::http::openapi::ErrorBody)))]
async fn remove_shortcut(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    if shortcuts::remove(state.db(), family.user_id, id).await.map_err(AuthError::Internal)? {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::ItemNotFound)
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
struct HoldingsQuery {
    /// `audiobook`, `music_track`, `podcast_episode` or `companion_file`; with `id`.
    #[serde(default)]
    kind: Option<String>,
    /// With `kind`.
    #[serde(default)]
    id: Option<Uuid>,
}

/// With `kind` and `id`: the devices holding that item. Without: what the
/// calling device holds.
#[derive(Serialize, ToSchema)]
#[serde(untagged)]
enum HoldingsResponse {
    Devices(Vec<holdings::HoldingDevice>),
    Items(Vec<holdings::Holding>),
}

/// GET /api/v1/sync/holdings?kind=&id= — the caller's devices holding that
/// item. Without a query: what the calling device has reported.
#[utoipa::path(get, path = "/holdings", tag = "sync", security(("bearer" = [])),
    params(HoldingsQuery),
    responses((status = 200, body = HoldingsResponse),
        (status = 400, description = "Unknown kind, only one of kind and id, or a session without a device", body = crate::http::openapi::ErrorBody)))]
async fn get_holdings(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<HoldingsQuery>,
) -> Result<Json<HoldingsResponse>, AuthError> {
    match (q.kind, q.id) {
        (Some(kind), Some(id)) => {
            if paths::Kind::parse(&kind).is_none() {
                return Err(AuthError::BadRequest(format!("unknown kind '{kind}'")));
            }
            let mut devices = holdings::devices_holding(state.db(), auth.user_id, &kind, id)
                .await
                .map_err(AuthError::Internal)?;
            for d in &mut devices {
                d.current = Some(d.chain_id) == auth.chain_id;
            }
            Ok(Json(HoldingsResponse::Devices(devices)))
        }
        (None, None) => {
            let chain = auth.chain_id.ok_or_else(no_device)?;
            holdings::held_by(state.db(), auth.user_id, chain)
                .await
                .map(|h| Json(HoldingsResponse::Items(h)))
                .map_err(AuthError::Internal)
        }
        _ => Err(AuthError::BadRequest("pass both kind and id, or neither".to_string())),
    }
}

/// PUT /api/v1/sync/holdings — `{items}` replaces the calling device's set,
/// `{added, removed}` changes it.
#[utoipa::path(put, path = "/holdings", tag = "sync", security(("bearer" = [])),
    request_body = holdings::HoldingsUpdate,
    responses((status = 204, description = "Saved"),
        (status = 400, description = "Invalid item, or a session without a device", body = crate::http::openapi::ErrorBody)))]
async fn put_holdings(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<holdings::HoldingsUpdate>,
) -> Result<StatusCode, AuthError> {
    let chain = auth.chain_id.ok_or_else(no_device)?;
    match holdings::update(state.db(), auth.user_id, chain, body).await.map_err(AuthError::Internal)? {
        Ok(()) => Ok(StatusCode::NO_CONTENT),
        Err(why) => Err(AuthError::BadRequest(why)),
    }
}

fn no_device() -> AuthError {
    AuthError::BadRequest("this session has no device; sign in again".to_string())
}
