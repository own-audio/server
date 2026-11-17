// SPDX-License-Identifier: AGPL-3.0-or-later
//! Companion files on `/api/v2` (completion plan phase 11): the images, booklets, lyrics and
//! cue sheets kept next to the audio in the own.audio folder (migration 0079), kind `file`.
//!
//! Put one with `POST /uploads` (kind `companion`) and `POST /files`; it is synced like the
//! audio (`/sync/snapshot?kinds=file`, `/sync/changes`), shared or kept private like any item,
//! deleted into the trash, and downloaded through `POST /playback-info` (kind `file`, always
//! the original). An image may become a cover: `PUT /tracks/{id}/image` or
//! `PUT /books/{id}/image` with its `file_id`. What the server does on its own when a file
//! arrives is v1's: an image called cover, folder or front (or the only image there) becomes
//! the cover of what has none in that folder, and a `.lrc` named like a track its lyrics.

use crate::app::AppState;
use crate::db;
use crate::db::access::{family_id_for_visibility, visibility_of};
use crate::families::FamilyContext;
use crate::filesync::{companion, paths};
use crate::v2::household::Visibility;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
pub struct File {
    pub id: Uuid,
    pub owner_id: Uuid,
    /// `private` or `family`.
    pub visibility: String,
    /// Where it sits in its owner's own.audio folder (`Music/Artist/Album/cover.jpg`).
    pub path: Option<String>,
    /// The last part of `path`.
    pub name: Option<String>,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    /// Filled in shortly after the upload.
    pub sha256: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Only in `POST /files`'s answer: `cover` when it became a cover, `lyrics` when it became
    /// a track's lyrics.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used_as: Option<String>,
}

pub(crate) fn file(row: db::files::FileRow) -> File {
    let name = row.path.as_deref().map(|p| p.rsplit('/').next().unwrap_or(p).to_string());
    File {
        id: row.id,
        owner_id: row.user_id,
        visibility: visibility_of(row.family_id).to_string(),
        path: row.path,
        name,
        content_type: row.content_type,
        size_bytes: row.size_bytes,
        sha256: row.sha256,
        created_at: row.created_at,
        updated_at: row.updated_at,
        used_as: None,
    }
}

#[derive(Deserialize, ToSchema)]
pub struct AttachTo {
    /// `album`, `book` or `track`, of the caller's own.
    pub kind: String,
    pub id: Uuid,
}

#[derive(Deserialize, ToSchema)]
pub struct NewFile {
    /// From `POST /uploads` (kind `companion`): an image, PDF, `.lrc`, `.cue`, `.txt` or `.nfo`.
    pub upload_id: String,
    /// Its whole path in the own.audio folder, under `Music/` or `Audiobooks/` (what a sync
    /// client sends)…
    pub path: Option<String>,
    /// …or the item it goes with — a book's folder, a track's, an album's (its first track's) —
    /// and its name there, `relative_path`.
    pub attach_to: Option<AttachTo>,
    pub relative_path: Option<String>,
    /// `private` (default) or `family`.
    pub visibility: Option<String>,
}

/// The path the request names, checked.
pub(crate) fn wanted_path(path: Option<&str>, folder: Option<&str>, relative: Option<&str>) -> Result<String, Problem> {
    let raw = match (path.map(str::trim).filter(|p| !p.is_empty()), folder) {
        (Some(p), None) => p.to_string(),
        (None, Some(folder)) => {
            let rel = relative.map(str::trim).filter(|r| !r.is_empty())
                .ok_or_else(|| Problem::bad_request("invalid_body", "`relative_path` names the file next to the item").field("/relative_path", "required", ""))?;
            let rel = paths::normalize_relative(rel).map_err(|e| Problem::bad_request("invalid_path", e).field("/relative_path", "invalid", ""))?;
            format!("{folder}/{rel}")
        }
        _ => return Err(Problem::bad_request("invalid_body", "give `path`, or `attach_to` with `relative_path`").field("/path", "required", "")),
    };
    let normalized = paths::normalize(&raw, paths::Kind::CompanionFile).map_err(|e| Problem::bad_request("invalid_path", e).field("/path", "invalid", ""))?;
    if !paths::is_companion(&normalized) {
        return Err(Problem::bad_request("unsupported_type", format!("only {} files are kept next to the audio", paths::COMPANION_EXTENSIONS.join(", ")))
            .field("/path", "invalid", ""));
    }
    Ok(normalized)
}

/// Keep a file next to the audio: an image, a booklet, lyrics, a cue sheet. Send an
/// `Idempotency-Key`: a retry answers the first answer and keeps one file.
#[utoipa::path(post, path = "/files", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewFile,
    responses((status = 201, body = File),
        (status = 400, description = "`invalid_body`, `invalid_path` (outside `Music/` and `Audiobooks/`, `..`), `unsupported_type`, `invalid_visibility`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 402, description = "`storage_refused`: the household has no room for it", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "No such upload in this household, or `attach_to` is not the caller's", body = crate::v2::problem::ProblemBody)))]
pub async fn create(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewFile>) -> Result<(StatusCode, Json<File>), Problem> {
    family.require_can_upload()?;
    let folder = match &body.attach_to {
        None => None,
        Some(a) => {
            if !["album", "book", "track"].contains(&a.kind.as_str()) {
                return Err(Problem::bad_request("invalid_kind", "attach_to.kind is `album`, `book` or `track`").field("/attach_to/kind", "invalid", a.kind.clone()));
            }
            Some(db::files::folder_of(state.db(), family.user_id, &a.kind, a.id).await?.ok_or_else(|| Problem::not_found().detail("not an item of yours"))?)
        }
    };
    if body.path.is_some() && folder.is_some() {
        return Err(Problem::bad_request("invalid_body", "give `path` or `attach_to`, not both").field("/path", "conflict", ""));
    }
    let path = wanted_path(body.path.as_deref(), folder.as_deref(), body.relative_path.as_deref())?;
    let family_id = family_id_for_visibility(body.visibility.as_deref(), family.family_id)
        .map_err(|e| Problem::bad_request("invalid_visibility", e).field("/visibility", "invalid", ""))?;
    let adopted = crate::uploads::adopt(&state, &family, body.upload_id.trim()).await.map_err(|e| match e {
        crate::auth::error::AuthError::NotFound => Problem::not_found().detail("no such upload in this household"),
        crate::auth::error::AuthError::BadRequest(m) => Problem::bad_request("invalid_upload", m).field("/upload_id", "invalid", ""),
        other => other.into(),
    })?;
    let made = companion::register(&state, family.user_id, family_id, adopted.media_object_id, body.upload_id.trim(), &path, adopted.size_bytes).await?;
    let row = db::files::find_file(state.db(), family.viewer(), made.id).await?.ok_or_else(Problem::not_found)?;
    let mut answer = file(row);
    answer.used_as = made.used_as.map(str::to_string);
    Ok((StatusCode::CREATED, Json(answer)))
}

/// One file the caller can see.
#[utoipa::path(get, path = "/files/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = File), (status = 404, description = "Unknown, not visible, or in the trash", body = crate::v2::problem::ProblemBody)))]
pub async fn get(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<File>, Problem> {
    let row = db::files::find_file(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(file(row)))
}

/// Share a file with the household, or keep it to oneself. Its owner.
#[utoipa::path(put, path = "/files/{id}/visibility", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    request_body = Visibility,
    responses((status = 200, body = Visibility), (status = 400, description = "`invalid_body`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not the caller's", body = crate::v2::problem::ProblemBody)))]
pub async fn set_visibility(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<Visibility>) -> Result<Json<Visibility>, Problem> {
    let family_id = match body.visibility.as_str() {
        "private" => None,
        "family" => Some(family.family_id),
        other => return Err(Problem::bad_request("invalid_body", "visibility is `private` or `family`").field("/visibility", "invalid", other.to_string())),
    };
    if !db::files::set_visibility(state.db(), family.user_id, id, family_id).await? {
        return Err(Problem::not_found());
    }
    Ok(Json(body))
}

/// Delete a file: into the trash for 30 days (kind `file`), as a track. Its owner, or a
/// household admin for one shared with the household.
#[utoipa::path(delete, path = "/files/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "In the trash"),
        (status = 403, description = "Visible, but neither its owner nor, for a shared one, a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown, not visible, or already in the trash", body = crate::v2::problem::ProblemBody)))]
pub async fn delete(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    crate::trash::move_to_trash_in(&state, family.viewer(), Uuid::new_v4(), db::trash::Kind::CompanionFile, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_file_goes_where_the_request_says() {
        assert_eq!(wanted_path(Some("Music/A/B/cover.jpg"), None, None).unwrap(), "Music/A/B/cover.jpg");
        assert_eq!(wanted_path(None, Some("Audiobooks/W/T"), Some("booklet.pdf")).unwrap(), "Audiobooks/W/T/booklet.pdf");
        assert_eq!(wanted_path(None, Some("Music/A/B"), None).unwrap_err().code(), "invalid_body");
        assert_eq!(wanted_path(None, None, Some("x.jpg")).unwrap_err().code(), "invalid_body");
        assert_eq!(wanted_path(Some("Podcasts/S/x.jpg"), None, None).unwrap_err().code(), "invalid_path");
        assert_eq!(wanted_path(Some("Music/A/../x.jpg"), None, None).unwrap_err().code(), "invalid_path");
        assert_eq!(wanted_path(Some("Music/A/B/song.mp3"), None, None).unwrap_err().code(), "unsupported_type");
    }
}
