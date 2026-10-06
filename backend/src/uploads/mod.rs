// SPDX-License-Identifier: AGPL-3.0-or-later
//! Direct-to-storage uploads.
//!
//! The multipart routes (`POST /audiobooks/upload` and friends) stream the
//! bytes through the API container. That works locally, but in production the
//! API is reached through Cloudflare, which caps a proxied request body at
//! 100 MB — an ordinary audiobook does not fit.
//!
//! These two endpoints move the bytes off that path entirely: the client asks
//! for a presigned `PUT`, uploads straight to object storage, then registers
//! what it uploaded. The API only ever sees the two small JSON requests.
//!
//! The object key is always derived server-side, under the caller's family
//! prefix, and `complete` re-checks that prefix — a client cannot presign or
//! register a key belonging to another family.

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::FamilyContext;
use crate::storage::ObjectStore;
use axum::Router;
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::routing::post;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// How long a presigned PUT stays valid. Long enough for a slow phone on a
/// bad connection to finish a large file, short enough that a leaked URL
/// stops working the same day.
const UPLOAD_URL_EXPIRY_SECS: u64 = 6 * 3600;

/// R2 (like S3) rejects a single `PutObject` above 5 GiB. Multipart upload
/// would lift this; nothing in the product needs it yet.
const MAX_UPLOAD_BYTES: i64 = 5 * 1024 * 1024 * 1024;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/presign", post(presign))
        .route("/complete", post(complete))
}

#[derive(Deserialize)]
pub struct PresignRequest {
    /// What the object will be used for — decides the key prefix. One of
    /// `audiobook_file`, `audiobook_cover`, `music_track`, `music_cover`,
    /// `companion_file`.
    pub kind: String,
    pub filename: String,
    pub content_type: String,
    /// Client-declared size, checked against the storage limit up front so a
    /// doomed 6 GB upload fails immediately instead of at the last byte. The
    /// real size is read back from storage in `complete`.
    pub size_bytes: Option<i64>,
}

#[derive(Serialize)]
pub struct PresignResponse {
    pub object_key: String,
    pub url: String,
    /// Always `PUT`.
    pub method: &'static str,
    /// Header the client MUST send with exactly this value: it is part of the
    /// signature, and anything else fails with a signature mismatch.
    pub content_type: String,
    pub expires_in_secs: u64,
}

#[derive(Deserialize)]
pub struct CompleteRequest {
    pub object_key: String,
}

#[derive(Serialize)]
pub struct CompleteResponse {
    pub media_object_id: String,
    pub object_key: String,
    pub content_type: String,
    pub size_bytes: i64,
}

/// POST /api/v1/uploads/presign
async fn presign(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<PresignRequest>,
) -> Result<Json<PresignResponse>, AuthError> {
    let content_type = body.content_type.trim();
    if content_type.is_empty() {
        return Err(AuthError::BadRequest("content_type is required".to_string()));
    }

    if let Some(size) = body.size_bytes {
        if size <= 0 {
            return Err(AuthError::BadRequest("size_bytes must be positive".to_string()));
        }
        if size > MAX_UPLOAD_BYTES {
            return Err(AuthError::BadRequest(format!(
                "file is larger than the {MAX_UPLOAD_BYTES}-byte single-upload limit"
            )));
        }
    }

    let folder = match body.kind.as_str() {
        "audiobook_file" => "audiobooks",
        "audiobook_cover" => "audiobook-covers",
        "music_track" => "music",
        "music_cover" => "music-covers",
        // Images, booklets, lyrics, cue sheets kept next to the audio.
        "companion_file" => "companions",
        other => {
            return Err(AuthError::BadRequest(format!(
                "unknown upload kind '{other}'"
            )));
        }
    };

    // Staged under `uploads/`, not under the eventual book/track id: the item
    // does not exist yet at upload time. Nothing recomputes these keys —
    // every read goes through `media_objects.object_key` — so the staging
    // layout is stable regardless of what the object is later attached to.
    let object_key = crate::storage::family_key(
        family.family_id,
        format!(
            "uploads/{folder}/{}/{}",
            Uuid::new_v4(),
            crate::audiobooks::sanitize_filename_component(&body.filename)
        ),
    );

    let url = state
        .storage()
        .presigned_put(&object_key, content_type, UPLOAD_URL_EXPIRY_SECS)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(PresignResponse {
        object_key,
        url,
        method: "PUT",
        content_type: content_type.to_string(),
        expires_in_secs: UPLOAD_URL_EXPIRY_SECS,
    }))
}

/// POST /api/v1/uploads/complete
async fn complete(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CompleteRequest>,
) -> Result<(StatusCode, Json<CompleteResponse>), AuthError> {
    require_own_key(&family, &body.object_key)?;

    let (size_bytes, content_type) = state
        .storage()
        .head_object(&body.object_key)
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| {
            AuthError::BadRequest("no object was uploaded under that key".to_string())
        })?;

    let media_object_id =
        register_object(state.db(), state.storage(), &body.object_key, &content_type, size_bytes)
            .await
            .map_err(AuthError::Internal)?;

    Ok((
        StatusCode::CREATED,
        Json(CompleteResponse {
            media_object_id: media_object_id.to_string(),
            object_key: body.object_key,
            content_type,
            size_bytes,
        }),
    ))
}

/// Reject a key outside the caller's family prefix.
///
/// Keys are minted server-side, so a mismatch means the client made one up —
/// answer with the generic not-found error rather than explaining the prefix
/// scheme back to whoever is probing. Note `AuthError::NotFound` renders as
/// 401 across this API, not 404.
pub fn require_own_key(family: &FamilyContext, object_key: &str) -> Result<(), AuthError> {
    let prefix = crate::storage::family_key(family.family_id, "");
    if object_key.starts_with(&prefix) {
        Ok(())
    } else {
        Err(AuthError::NotFound)
    }
}

/// Upsert the `media_objects` row for an already-uploaded object.
///
/// Same upsert the multipart path uses (`audiobooks::store_temp_upload`), so
/// re-registering a key is idempotent rather than a unique-violation.
pub async fn register_object(
    pool: &sqlx::PgPool,
    storage: &ObjectStore,
    object_key: &str,
    content_type: &str,
    size_bytes: i64,
) -> anyhow::Result<Uuid> {
    db::media::upsert_object(pool, storage.bucket(), object_key, content_type, Some(size_bytes)).await
}
