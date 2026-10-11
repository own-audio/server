// SPDX-License-Identifier: AGPL-3.0-or-later
//! Adding to the library (docs/api-v2-plan.md, V9): `POST /uploads` hands out
//! a link to `PUT` one file straight to storage; `POST /tracks` and
//! `POST /books` make items of what was put there. The size is declared up
//! front (security hardening plan, C1), so the quota or credit refuses a file
//! that cannot fit before a byte is sent, and storage refuses one larger than
//! declared. The checks are v1's; send `Idempotency-Key` with the creates.

use crate::app::AppState;
use crate::db;
use crate::families::FamilyContext;
use crate::v2::problem::Problem;
use crate::v2::{music, spoken};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::json;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Deserialize, ToSchema)]
pub struct UploadRequest {
    /// `track`, `book_file`, `track_cover`, `book_cover` or `companion`.
    pub kind: String,
    pub filename: String,
    pub content_type: String,
    /// Required: the file's exact size in bytes.
    pub size_bytes: i64,
}

#[derive(Serialize, ToSchema)]
pub struct Upload {
    /// Name it in `POST /tracks` or `POST /books`.
    pub upload_id: String,
    /// `PUT` the file here…
    pub url: String,
    pub method: String,
    /// …with exactly these headers: they are part of the signature.
    #[schema(value_type = Object)]
    pub headers: serde_json::Value,
    pub expires_at: DateTime<Utc>,
}

/// A link to put one file straight into storage.
#[utoipa::path(post, path = "/uploads", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = UploadRequest,
    responses((status = 201, body = Upload),
        (status = 400, description = "Unknown kind, missing type, or a size that is not positive or over 5 GiB", body = crate::v2::problem::ProblemBody),
        (status = 402, description = "`storage_refused`: the household has no room for it", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_upload(family: FamilyContext, State(state): State<AppState>, Json(body): Json<UploadRequest>) -> Result<(StatusCode, Json<Upload>), Problem> {
    let kind = match body.kind.as_str() {
        "track" => "music_track",
        "book_file" => "audiobook_file",
        "track_cover" => "music_cover",
        "book_cover" => "audiobook_cover",
        "companion" => "companion_file",
        other => {
            return Err(Problem::bad_request("unsupported_kind", format!("`{other}`: one of track, book_file, track_cover, book_cover, companion"))
                .field("/kind", "invalid", other.to_string()));
        }
    };
    let request = crate::uploads::PresignRequest {
        kind: kind.into(),
        filename: body.filename,
        content_type: body.content_type,
        size_bytes: Some(body.size_bytes),
    };
    let Json(p) = crate::uploads::presign(family, State(state), Json(request)).await?;
    Ok((
        StatusCode::CREATED,
        Json(Upload {
            upload_id: p.object_key,
            url: p.url,
            method: p.method.to_string(),
            headers: json!({ "Content-Type": p.content_type }),
            expires_at: Utc::now() + chrono::Duration::seconds(p.expires_in_secs as i64),
        }),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct NewTrack {
    pub upload_id: String,
    /// The file's own name: the title when the tags have none.
    pub filename: String,
    /// `private` (default) or `family`.
    pub visibility: Option<String>,
    /// What the tags say otherwise, when they say nothing or something wrong.
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    /// Where it goes in the own.audio folder (`Music/…`).
    pub path: Option<String>,
}

/// A track from an uploaded file.
#[utoipa::path(post, path = "/tracks", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewTrack,
    responses((status = 201, body = music::Track),
        (status = 400, description = "Nothing was put under that upload", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_track(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewTrack>) -> Result<(StatusCode, Json<music::Track>), Problem> {
    let request: crate::music::FromUploadRequest = serde_json::from_value(json!({
        "object_key": body.upload_id,
        "original_filename": body.filename,
        "visibility": body.visibility,
        "title": body.title,
        "artist": body.artist,
        "album": body.album,
        "album_artist": body.album_artist,
        "genre": body.genre,
        "track_number": body.track_number,
        "path": body.path,
    }))
    .map_err(|e| Problem::bad_request("invalid_body", e.to_string()))?;
    let viewer = family.viewer();
    let (_, Json(made)) = crate::music::create_track_from_upload(family, State(state.clone()), Json(request)).await?;
    let id: Uuid = made.track.id.parse().map_err(|e: uuid::Error| Problem::internal(anyhow::anyhow!(e)))?;
    let row = db::catalog::find_track(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    let track = music::tracks(&state, vec![row]).await?.pop().ok_or_else(Problem::not_found)?;
    Ok((StatusCode::CREATED, Json(track)))
}

#[derive(Deserialize, ToSchema)]
pub struct NewBookFile {
    pub upload_id: String,
    /// Its path inside the book's folder; decides the order of the files.
    pub relative_path: Option<String>,
    pub title: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct NewBook {
    pub title: Option<String>,
    /// `;` or ` & ` between several.
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
    /// `private` (default) or `family`.
    pub visibility: Option<String>,
    pub cover_upload_id: Option<String>,
    /// Where it goes in the own.audio folder (`Audiobooks/…`).
    pub path: Option<String>,
    pub files: Vec<NewBookFile>,
}

/// A book from uploaded files.
#[utoipa::path(post, path = "/books", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewBook,
    responses((status = 201, body = spoken::Book),
        (status = 400, description = "No files, a duplicate path, or nothing put under an upload", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_book(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewBook>) -> Result<(StatusCode, Json<spoken::Book>), Problem> {
    let files: Vec<serde_json::Value> = body
        .files
        .into_iter()
        .map(|f| json!({ "object_key": f.upload_id, "relative_path": f.relative_path, "title": f.title }))
        .collect();
    let request: crate::audiobooks::FromUploadsRequest = serde_json::from_value(json!({
        "title": body.title,
        "author": body.author,
        "narrator": body.narrator,
        "description": body.description,
        "visibility": body.visibility,
        "cover_object_key": body.cover_upload_id,
        "path": body.path,
        "files": files,
    }))
    .map_err(|e| Problem::bad_request("invalid_body", e.to_string()))?;
    let viewer = family.viewer();
    let (_, Json(made)) = crate::audiobooks::create_book_from_uploads(family, State(state.clone()), Json(request)).await?;
    let id: Uuid = made.book.id.parse().map_err(|e: uuid::Error| Problem::internal(anyhow::anyhow!(e)))?;
    let row = db::catalog::find_book(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    Ok((StatusCode::CREATED, Json(spoken::full_book(&state, row).await?)))
}
