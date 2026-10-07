// SPDX-License-Identifier: AGPL-3.0-or-later
/// Companion files — docs/file-sync-plan.md §2 item 16, migration 0079.
///
/// Images, booklets, lyrics and cue sheets put next to music or into a book
/// folder are kept exactly as they are, at their path, and synced like the
/// audio. They are never changed; what the server does with them is read
/// them: an image can become the album's or the book's cover, a `.lrc` named
/// like a track becomes its lyrics.
use super::paths::{self, Kind};
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db::access;
use crate::families::FamilyContext;
use anyhow::Context;
use axum::extract::{Json, Path, State};
use axum::http::{HeaderMap, StatusCode};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use uuid::Uuid;

const STREAM_EXPIRY_SECS: u64 = 4 * 3600;
/// Lyrics files are text; anything bigger is not one.
const MAX_LYRICS_BYTES: i64 = 1024 * 1024;
/// File names that say "I am the cover", compared without extension.
const COVER_NAMES: &[&str] = &["cover", "folder", "front"];

#[derive(Deserialize, utoipa::ToSchema)]
pub struct NewCompanion {
    /// Key from `/uploads/presign` (kind `companion_file`), already PUT.
    pub object_key: String,
    /// Where it sits: under `Music/` or `Audiobooks/`.
    pub path: String,
    /// `private` (default) or `family`.
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Serialize, utoipa::ToSchema)]
pub struct CompanionResponse {
    pub id: Uuid,
    /// As asked, or with ` (2)` when the owner already has that path.
    pub path: String,
    pub visibility: &'static str,
    pub size_bytes: i64,
    /// What the server did with it: `cover` (became a cover), `lyrics`
    /// (became a track's lyrics), or nothing.
    pub used_as: Option<&'static str>,
}

/// POST /api/v1/sync/files
///
/// Register an image, booklet, lyrics file or cue sheet uploaded with
/// `/uploads/presign` (kind `companion_file`) at its place next to the audio.
#[utoipa::path(post, path = "/files", tag = "sync", security(("bearer" = [])),
    request_body = NewCompanion,
    responses((status = 201, body = CompanionResponse),
        (status = 400, description = "Bad path, not a companion file type, bad visibility, or nothing uploaded", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "The key is outside the caller's family", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "The caller may not upload", body = crate::http::openapi::ErrorBody)))]
pub async fn create(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<NewCompanion>,
) -> Result<(StatusCode, Json<CompanionResponse>), AuthError> {
    family.require_can_upload()?;
    crate::uploads::require_own_key(&family, &body.object_key)?;
    let path = paths::normalize(&body.path, Kind::CompanionFile).map_err(AuthError::BadRequest)?;
    if !paths::is_companion(&path) {
        return Err(AuthError::BadRequest(format!(
            "only {} files are kept next to the audio",
            paths::COMPANION_EXTENSIONS.join(", ")
        )));
    }
    let family_id = access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
        .map_err(|e| AuthError::BadRequest(e.to_string()))?;
    let media_kind = if path.starts_with(paths::AUDIOBOOKS) { access::AUDIOBOOK } else { access::MUSIC };

    let (size_bytes, content_type) = state
        .storage()
        .head_object(&body.object_key)
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("no object was uploaded under that key".to_string()))?;
    let pool = state.db();
    let object_id = crate::db::media::upsert_object(pool, state.storage().bucket(), &body.object_key, &content_type, Some(size_bytes))
        .await
        .map_err(AuthError::Internal)?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO companion_files (user_id, family_id, media_kind, object_id) VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(family.user_id)
    .bind(family_id)
    .bind(media_kind)
    .bind(object_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    let path = paths::claim(pool, family.user_id, Kind::CompanionFile, id, &path).await.map_err(AuthError::Internal)?;

    let mut used_as = None;
    if paths::is_image(&path) {
        if apply_default_cover(pool, family.user_id, paths::parent_dir(&path)).await.map_err(AuthError::Internal)? {
            used_as = Some("cover");
        }
    } else if paths::file_extension(&path) == "lrc"
        && lyrics_from(&state, family.user_id, &path, &body.object_key, size_bytes).await.map_err(AuthError::Internal)?
    {
        used_as = Some("lyrics");
    }

    Ok((
        StatusCode::CREATED,
        Json(CompanionResponse { id, path, visibility: access::visibility_of(family_id), size_bytes, used_as }),
    ))
}

/// A companion file the caller may see: `(owner, family_id, object key)`.
async fn find_visible(pool: &PgPool, family: &FamilyContext, id: Uuid) -> Result<(Uuid, Option<Uuid>, Uuid, String), AuthError> {
    let viewer = family.viewer();
    sqlx::query_as::<_, (Uuid, Option<Uuid>, Uuid, String)>(
        "SELECT t.user_id, t.family_id, t.object_id, m.object_key
           FROM companion_files t JOIN media_objects m ON m.id = t.object_id
          WHERE t.id = $4 AND audio2_can_access($1, $2, $3, t.media_kind, t.id, t.user_id, t.family_id)",
    )
    .bind(viewer.user_id)
    .bind(viewer.family_id)
    .bind(viewer.is_family_admin)
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    .ok_or(AuthError::ItemNotFound)
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = CompanionStreamResponse)]
pub struct StreamResponse {
    pub url: String,
    pub expires_in_secs: u64,
}

/// GET /api/v1/sync/files/{id}/stream — a presigned `GET` (honours `Range`).
#[utoipa::path(get, path = "/files/{id}/stream", tag = "sync", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Companion file id")),
    responses((status = 200, body = StreamResponse),
        (status = 404, body = crate::http::openapi::ErrorBody)))]
pub async fn stream(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<StreamResponse>, AuthError> {
    let (_, _, _, key) = find_visible(state.db(), &family, id).await?;
    let url = state.storage().presigned_get(&key, STREAM_EXPIRY_SECS).await.map_err(AuthError::Internal)?;
    Ok(Json(StreamResponse { url, expires_in_secs: STREAM_EXPIRY_SECS }))
}

#[derive(Deserialize, utoipa::ToSchema)]
#[schema(as = CompanionVisibility)]
pub struct SetVisibility {
    /// `private` or `family`.
    pub visibility: String,
}

/// PUT /api/v1/sync/files/{id}/visibility — the owner shares or unshares it.
#[utoipa::path(put, path = "/files/{id}/visibility", tag = "sync", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Companion file id")),
    request_body = SetVisibility,
    responses((status = 204, description = "Saved"),
        (status = 400, description = "Unknown visibility", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "Not the caller's file", body = crate::http::openapi::ErrorBody)))]
pub async fn set_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetVisibility>,
) -> Result<StatusCode, AuthError> {
    let family_id = match body.visibility.trim() {
        access::VIS_PRIVATE => None,
        access::VIS_FAMILY => Some(family.family_id),
        _ => return Err(AuthError::BadRequest("visibility must be 'private' or 'family'".to_string())),
    };
    let done = sqlx::query("UPDATE companion_files SET family_id = $3, updated_at = now() WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(family.user_id)
        .bind(family_id)
        .execute(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    if done.rows_affected() == 0 {
        return Err(AuthError::ItemNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/sync/files/{id} — to the trash, like every delete.
#[utoipa::path(delete, path = "/files/{id}", tag = "sync", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Companion file id"),
        ("x-trash-batch" = Option<Uuid>, Header, description = "Groups everything deleted in one gesture for restoring together")),
    responses((status = 204, description = "Moved to the trash"),
        (status = 403, description = "Visible to the caller but not theirs to delete", body = crate::http::openapi::ErrorBody),
        (status = 404, body = crate::http::openapi::ErrorBody)))]
pub async fn delete(
    family: FamilyContext,
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    crate::trash::move_to_trash(&state, family.viewer(), Some(&headers), crate::db::trash::Kind::CompanionFile, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, utoipa::ToSchema)]
#[schema(as = CompanionCoverResponse)]
pub struct CoverResponse {
    pub book: Option<Uuid>,
    pub tracks: u64,
}

/// POST /api/v1/sync/files/{id}/use-as-cover — the owner picks this image as
/// the cover of the book whose folder it is in, and of every track in its
/// folder, replacing what they had (§2 item 16: when a folder has several
/// images, the user chooses).
#[utoipa::path(post, path = "/files/{id}/use-as-cover", tag = "sync", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Companion file id")),
    responses((status = 200, body = CoverResponse, description = "`book`: the book that got the cover, if any; `tracks`: how many tracks did"),
        (status = 400, description = "Not an image", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "Not the caller's file", body = crate::http::openapi::ErrorBody)))]
pub async fn use_as_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CoverResponse>, AuthError> {
    let pool = state.db();
    let row: Option<(Uuid, String)> = sqlx::query_as(
        "SELECT t.object_id, p.path FROM companion_files t
           JOIN sync_paths p ON p.kind = 'companion_file' AND p.item_id = t.id
          WHERE t.id = $1 AND t.user_id = $2",
    )
    .bind(id)
    .bind(family.user_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    let (object_id, path) = row.ok_or(AuthError::ItemNotFound)?;
    if !paths::is_image(&path) {
        return Err(AuthError::BadRequest("only an image can be a cover".to_string()));
    }
    let (book, tracks) = set_cover(pool, family.user_id, paths::parent_dir(&path), object_id, true)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(CoverResponse { book, tracks }))
}

// ── Rules ─────────────────────────────────────────────────────────────────

/// Give the book and tracks in `dir` that have no cover the folder's cover
/// image: one called cover/folder/front, else the only image there. Art
/// embedded in the audio came first and is kept. True when it applied.
pub async fn apply_default_cover(pool: &PgPool, owner: Uuid, dir: &str) -> anyhow::Result<bool> {
    let images: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT t.object_id, p.path FROM companion_files t
           JOIN sync_paths p ON p.kind = 'companion_file' AND p.item_id = t.id
          WHERE t.user_id = $1 AND starts_with(lower(p.path), lower($2) || '/')",
    )
    .bind(owner)
    .bind(dir)
    .fetch_all(pool)
    .await
    .context("db: images in folder")?;
    let here: Vec<&(Uuid, String)> =
        images.iter().filter(|(_, p)| paths::is_image(p) && paths::parent_dir(p).eq_ignore_ascii_case(dir)).collect();
    let named = here.iter().find(|(_, p)| {
        let name = p.rsplit('/').next().unwrap_or("");
        let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_lowercase();
        COVER_NAMES.contains(&stem.as_str())
    });
    let chosen = match (named, here.as_slice()) {
        (Some(n), _) => n.0,
        (None, [only]) => only.0,
        _ => return Ok(false),
    };
    let (book, tracks) = set_cover(pool, owner, dir, chosen, false).await?;
    Ok(book.is_some() || tracks > 0)
}

/// Set `object` as the cover of the owner's book whose folder is `dir` and of
/// their tracks directly in `dir`; `replace` also overwrites existing covers.
async fn set_cover(pool: &PgPool, owner: Uuid, dir: &str, object: Uuid, replace: bool) -> anyhow::Result<(Option<Uuid>, u64)> {
    let book: Option<Uuid> = sqlx::query_scalar(
        "UPDATE audiobook_books SET cover_object_id = $3, updated_at = now()
          WHERE id = (SELECT item_id FROM sync_live_paths
                       WHERE user_id = $1 AND kind = 'audiobook' AND lower(path) = lower($2))
            AND ($4 OR cover_object_id IS NULL)
         RETURNING id",
    )
    .bind(owner)
    .bind(dir)
    .bind(object)
    .bind(replace)
    .fetch_optional(pool)
    .await
    .context("db: book cover from image")?;

    let candidates: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT item_id, path FROM sync_live_paths
          WHERE user_id = $1 AND kind = 'music_track' AND starts_with(lower(path), lower($2) || '/')",
    )
    .bind(owner)
    .bind(dir)
    .fetch_all(pool)
    .await
    .context("db: tracks in folder")?;
    let ids: Vec<Uuid> = candidates
        .into_iter()
        .filter(|(_, p)| paths::parent_dir(p).eq_ignore_ascii_case(dir))
        .map(|(id, _)| id)
        .collect();
    let tracks = sqlx::query(
        "UPDATE music_tracks SET cover_object_id = $2, updated_at = now()
          WHERE id = ANY($1) AND ($3 OR cover_object_id IS NULL)",
    )
    .bind(&ids)
    .bind(object)
    .bind(replace)
    .execute(pool)
    .await
    .context("db: track covers from image")?
    .rows_affected();
    Ok((book, tracks))
}

/// A `.lrc` next to a track of the same name becomes the track's lyrics.
async fn lyrics_from(state: &AppState, owner: Uuid, path: &str, key: &str, size: i64) -> anyhow::Result<bool> {
    if size > MAX_LYRICS_BYTES {
        return Ok(false);
    }
    let dir = paths::parent_dir(path);
    let name = path.rsplit('/').next().unwrap_or(path);
    let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(name).to_lowercase();
    let tracks: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT item_id, path FROM sync_live_paths
          WHERE user_id = $1 AND kind = 'music_track' AND starts_with(lower(path), lower($2) || '/')",
    )
    .bind(owner)
    .bind(dir)
    .fetch_all(state.db())
    .await
    .context("db: tracks for lyrics")?;
    let track = tracks.into_iter().find(|(_, p)| {
        let n = p.rsplit('/').next().unwrap_or(p);
        paths::parent_dir(p).eq_ignore_ascii_case(dir)
            && n.rsplit_once('.').map(|(s, _)| s).unwrap_or(n).to_lowercase() == stem
    });
    let Some((track, _)) = track else { return Ok(false) };
    let bytes = state.storage().get(key).await?;
    let text = String::from_utf8_lossy(&bytes);
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Ok(false);
    }
    crate::db::music::set_user_track_lyrics(state.db(), track, owner, text).await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    #[test]
    fn cover_names_are_lowercase() {
        assert!(super::COVER_NAMES.iter().all(|n| n.to_lowercase() == *n));
    }
}
