// SPDX-License-Identifier: AGPL-3.0-or-later
//! Corrections on artists and albums (docs/data-model-plan.md, E4). A wrong
//! match is fixed once, on the entity, never by retagging files, and holds at
//! the next scan (migration 0104).
//!
//! Renaming an artist or album, and grouping editions, is any member's: they
//! are the household's shared names, as with authors. Merging and splitting
//! rewrite what everyone's library is credited to, so they are a family
//! admin's, like deleting an author.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::db::entities::Renamed;
use crate::families::FamilyContext;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(rename_artist))
        .routes(routes!(merge_artist))
        .routes(routes!(split_artist))
        .routes(routes!(rename_album))
        .routes(routes!(set_album_release_group))
        .routes(routes!(merge_album))
}

#[derive(Serialize, ToSchema)]
pub struct ContributorRef {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RenameArtistRequest {
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct MergeRequest {
    /// The one that stays.
    pub into: Uuid,
}

#[derive(Deserialize, ToSchema)]
pub struct SplitArtistRequest {
    /// The contributors it really is, in credit order; at least two.
    pub names: Vec<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct RenameAlbumRequest {
    pub title: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ReleaseGroupRequest {
    /// Another album's `release_group_id`, to group the two as editions of one
    /// release; `null` for a group of its own.
    pub release_group_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct ReleaseGroupResponse {
    pub release_group_id: String,
}

/// An artist the caller can see.
async fn visible_artist(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<String, AuthError> {
    db::subsonic::find_artist(state.db(), family.viewer(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)
}

/// An album the caller can see.
async fn visible_album(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<(), AuthError> {
    db::subsonic::find_album(state.db(), family.viewer(), id)
        .await
        .map_err(AuthError::Internal)?
        .map(|_| ())
        .ok_or(AuthError::ItemNotFound)
}

/// Rename an artist everywhere it is credited. Its files keep their tags, and
/// the next scan still finds it by the old spelling.
#[utoipa::path(patch, path = "/artists/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Artist id, from `/music/artists`")),
    request_body = RenameArtistRequest,
    responses(
        (status = 200, body = ContributorRef),
        (status = 409, description = "Another artist already has this name: merge them instead", body = crate::http::openapi::ErrorBody)))]
async fn rename_artist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RenameArtistRequest>,
) -> Result<Json<ContributorRef>, AuthError> {
    let name = body.name.trim();
    if name.is_empty() {
        return Err(AuthError::BadRequest("name is required".into()));
    }
    visible_artist(&state, &family, id).await?;
    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    match db::entities::rename_contributor(&mut tx, family.family_id, id, name)
        .await
        .map_err(AuthError::Internal)?
    {
        Renamed::Done => {}
        Renamed::NotFound => return Err(AuthError::ItemNotFound),
        Renamed::Taken(_) => return Err(AuthError::Conflict(crate::audiobooks::authors::NAME_TAKEN.into())),
    }
    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(Json(ContributorRef { id: id.to_string(), name: name.to_string() }))
}

/// Merge an artist into another that is the same one: credits, the spellings
/// it is matched by, stars, ratings and photo go to `into`, and it is removed.
#[utoipa::path(post, path = "/artists/{id}/merge", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The artist that goes")),
    request_body = MergeRequest,
    responses(
        (status = 200, description = "The artist that stays", body = ContributorRef),
        (status = 403, description = "Not a family admin", body = crate::http::openapi::ErrorBody)))]
async fn merge_artist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<MergeRequest>,
) -> Result<Json<ContributorRef>, AuthError> {
    family.require_family_admin()?;
    if body.into == id {
        return Err(AuthError::BadRequest("an artist cannot be merged into itself".into()));
    }
    if !db::entities::merge_contributors(state.db(), family.family_id, id, body.into)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::ItemNotFound);
    }
    let name = db::authors::find_author(state.db(), family.family_id, body.into)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?
        .name;
    Ok(Json(ContributorRef { id: body.into.to_string(), name }))
}

/// Split an artist that is several ("A & B" read as one name) into the
/// artists it is: every credit becomes one per name, in order, and so does
/// every later scan of the same tags.
#[utoipa::path(post, path = "/artists/{id}/split", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The artist that goes")),
    request_body = SplitArtistRequest,
    responses(
        (status = 200, description = "The artists it became, in order", body = Vec<ContributorRef>),
        (status = 403, description = "Not a family admin", body = crate::http::openapi::ErrorBody)))]
async fn split_artist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SplitArtistRequest>,
) -> Result<Json<Vec<ContributorRef>>, AuthError> {
    family.require_family_admin()?;
    let mut names: Vec<String> = Vec::new();
    for name in body.names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if !names.iter().any(|seen| seen.to_lowercase() == name.to_lowercase()) {
            names.push(name.to_string());
        }
    }
    if names.len() < 2 {
        return Err(AuthError::BadRequest("a split needs at least two different names".into()));
    }
    let parts = db::entities::split_contributor(state.db(), family.family_id, id, &names)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    Ok(Json(
        parts
            .into_iter()
            .map(|(id, name)| ContributorRef { id: id.to_string(), name })
            .collect(),
    ))
}

/// Rename an album. Its tracks keep their tags and stay on it.
#[utoipa::path(patch, path = "/albums/{id}", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Album id, from `/music/albums`")),
    request_body = RenameAlbumRequest,
    responses((status = 204, description = "Renamed")))]
async fn rename_album(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<RenameAlbumRequest>,
) -> Result<StatusCode, AuthError> {
    if body.title.trim().is_empty() {
        return Err(AuthError::BadRequest("title is required".into()));
    }
    visible_album(&state, &family, id).await?;
    if !db::entities::rename_album(state.db(), family.family_id, id, &body.title)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::ItemNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Put an album in a release group: another edition's, so the two are one
/// release in different editions, or a group of its own. Tags no longer move it.
#[utoipa::path(put, path = "/albums/{id}/release-group", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Album id")),
    request_body = ReleaseGroupRequest,
    responses((status = 200, body = ReleaseGroupResponse)))]
async fn set_album_release_group(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReleaseGroupRequest>,
) -> Result<Json<ReleaseGroupResponse>, AuthError> {
    visible_album(&state, &family, id).await?;
    let group = db::entities::set_album_release_group(state.db(), family.family_id, id, body.release_group_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    Ok(Json(ReleaseGroupResponse { release_group_id: group.to_string() }))
}

/// Merge an album into another that is the same release tagged two ways:
/// its tracks, stars and ratings go to `into`, and so will its tracks' next scan.
#[utoipa::path(post, path = "/albums/{id}/merge", tag = "music", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The album that goes")),
    request_body = MergeRequest,
    responses(
        (status = 204, description = "Merged"),
        (status = 403, description = "Not a family admin", body = crate::http::openapi::ErrorBody)))]
async fn merge_album(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<MergeRequest>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    if body.into == id {
        return Err(AuthError::BadRequest("an album cannot be merged into itself".into()));
    }
    if !db::entities::merge_albums(state.db(), family.family_id, id, body.into)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::ItemNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}
