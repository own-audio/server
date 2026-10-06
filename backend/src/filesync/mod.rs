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
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/tree", get(get_tree))
        .route("/tree/ids", get(get_tree_ids))
        .route("/shortcuts", get(list_shortcuts).post(add_shortcut))
        .route("/shortcuts/{id}", axum::routing::delete(remove_shortcut))
        .route("/holdings", get(get_holdings).put(put_holdings))
        .route("/files", axum::routing::post(companion::create))
        .route("/files/{id}", axum::routing::delete(companion::delete))
        .route("/files/{id}/stream", get(companion::stream))
        .route("/files/{id}/visibility", axum::routing::put(companion::set_visibility))
        .route("/files/{id}/use-as-cover", axum::routing::post(companion::use_as_cover))
        .route("/paths/organise", axum::routing::post(organise_paths))
}

#[derive(Deserialize)]
struct TreeQuery {
    #[serde(default)]
    cursor: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

/// GET /api/v1/sync/tree?cursor=&limit=
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
async fn get_tree_ids(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<tree::TreeId>>, AuthError> {
    tree::all_ids(state.db(), family.viewer()).await.map(Json).map_err(AuthError::Internal)
}

/// GET /api/v1/sync/shortcuts
async fn list_shortcuts(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<shortcuts::Shortcut>>, AuthError> {
    shortcuts::list(state.db(), family.user_id).await.map(Json).map_err(AuthError::Internal)
}

/// POST /api/v1/sync/shortcuts — 201 when new, 200 when it already existed.
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
async fn organise_paths(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<organise::Request>,
) -> Result<Json<Vec<organise::Move>>, AuthError> {
    organise::organise(state.db(), family.user_id, &body).await.map(Json).map_err(AuthError::Internal)
}

/// DELETE /api/v1/sync/shortcuts/{id}
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

#[derive(Deserialize)]
struct HoldingsQuery {
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    id: Option<Uuid>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum HoldingsResponse {
    Devices(Vec<holdings::HoldingDevice>),
    Items(Vec<holdings::Holding>),
}

/// GET /api/v1/sync/holdings?kind=&id= — the caller's devices holding that
/// item. Without a query: what the calling device has reported.
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
