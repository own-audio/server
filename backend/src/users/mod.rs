// SPDX-License-Identifier: AGPL-3.0-or-later
/// Users module — accounts, roles, profile, admin functions.
pub mod models;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::families::FamilyContext;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Json, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use std::path::Path as StdPath;
use uuid::Uuid;

/// Above the app-level `MAX_AVATAR_BYTES` check, so a body under that check's cap is never
/// rejected purely by multipart-framing overhead — see `upload_avatar`.
const AVATAR_BODY_LIMIT_BYTES: usize = 6 * 1024 * 1024;
const MAX_AVATAR_BYTES: usize = 5 * 1024 * 1024;

// ── DTOs ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct UserResponse {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    /// `/api/v1/users/{id}/avatar` when set, else `null` — same relative-URL
    /// convention every other cover URL in this API uses.
    pub avatar_url: Option<String>,
    /// False until the user switches it on. Clients must compute and show
    /// nothing listening-derived while this is false — not compute it and
    /// hide it. There is no server-side derived state either way, which is
    /// what makes the switch honest rather than cosmetic.
    pub recommendations_enabled: bool,
    /// Base subtags (`en`, `cs`). Empty means discovery answers in every
    /// language — see migration 0063.
    pub discovery_languages: Vec<String>,
    pub created_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateUserRequest {
    pub display_name: Option<String>,
    pub role: Option<String>,
    pub is_active: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateSelfRequest {
    pub display_name: Option<String>,
    /// Deliberately only on `/users/me`, and deliberately absent from
    /// `UpdateUserRequest`: an admin can deactivate an account or change its
    /// role, but cannot decide for someone else that their listening may be
    /// used to suggest things.
    pub recommendations_enabled: Option<bool>,
    /// Which languages podcast discovery should answer in. Sent whole, not
    /// added to one at a time: it is a list the user edits, and a PATCH that
    /// appends has no way to express "remove the last one". `[]` means every
    /// language, which is the state everyone starts in.
    pub discovery_languages: Option<Vec<String>>,
}

#[derive(Serialize, ToSchema)]
pub struct SubsonicKeyResponse {
    pub username: String,
    pub api_key: String,
}

#[derive(Serialize, ToSchema)]
pub struct AdminUserResponse {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    pub is_active: bool,
    pub created_at: String,
    pub family_id: Option<String>,
    pub family_name: Option<String>,
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_users))
        .routes(routes!(get_self, update_self, delete_self))
        .routes(routes!(get_subsonic_key))
        .routes(routes!(regenerate_subsonic_key))
        .routes(crate::http::openapi::map(routes!(upload_avatar), |m| {
            m.layer(DefaultBodyLimit::max(AVATAR_BODY_LIMIT_BYTES))
        }))
        .routes(routes!(delete_avatar))
        .routes(routes!(get_user, update_user, admin_delete_user))
        .routes(routes!(admin_revoke_sessions))
        // Family-scoped, not the "self or global admin" rule `GET /{id}` uses above — a
        // profile picture is exactly the kind of thing fellow family members should see.
        .routes(routes!(get_avatar))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/users/  — admin only. Includes each user's family (name
/// visible to an instance admin only here — nowhere a family_admin can see
/// another family's name).
#[utoipa::path(get, path = "/", tag = "users", security(("bearer" = [])),
    responses((status = 200, body = Vec<AdminUserResponse>), (status = 401, description = "Invalid access token, or the caller is not an admin (answered 401, not 403)", body = crate::http::openapi::ErrorBody)))]
async fn list_users(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminUserResponse>>, AuthError> {
    require_admin(&auth)?;
    let users = db::users::list_all_with_family(state.db())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        users
            .into_iter()
            .map(|u| AdminUserResponse {
                id: u.id.to_string(),
                email: u.email,
                display_name: u.display_name,
                role: u.role,
                is_active: u.is_active,
                created_at: u.created_at.to_rfc3339(),
                family_id: u.family_id.map(|id| id.to_string()),
                family_name: u.family_name,
            })
            .collect(),
    ))
}

/// GET /api/v1/users/:id
#[utoipa::path(get, path = "/{id}", tag = "users", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, body = UserResponse),
        (status = 401, description = "Invalid access token, someone else's account asked for by a non-admin, or no such account", body = crate::http::openapi::ErrorBody)))]
async fn get_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<UserResponse>, AuthError> {
    // Users can only see themselves; admins can see anyone.
    if auth.user_id != id && auth.role != "admin" {
        return Err(AuthError::SessionInvalid);
    }

    let user = db::users::find_by_id(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    Ok(Json(user_to_response(user)))
}

/// PATCH /api/v1/users/:id  — admin only
#[utoipa::path(patch, path = "/{id}", tag = "users", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    request_body = UpdateUserRequest,
    responses((status = 204, description = "Updated; deactivating also signs the account out everywhere"), (status = 401, description = "Invalid access token, or the caller is not an admin (answered 401, not 403)", body = crate::http::openapi::ErrorBody)))]
async fn update_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateUserRequest>,
) -> Result<StatusCode, AuthError> {
    require_admin(&auth)?;

    let pool = state.db();

    if let Some(ref display_name) = body.display_name {
        sqlx::query("UPDATE users SET display_name = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(display_name)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
    }

    if let Some(ref role) = body.role {
        sqlx::query("UPDATE users SET role = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(role)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
    }

    if let Some(is_active) = body.is_active {
        sqlx::query("UPDATE users SET is_active = $1, updated_at = CURRENT_TIMESTAMP WHERE id = $2")
            .bind(is_active)
            .bind(id)
            .execute(pool)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;

        // Mirrors families::block_member: is_active alone doesn't cut off an
        // already-issued JWT (AuthUser only checks session revocation, not
        // this flag), so a "locked" account would stay signed in until its
        // token expired without this.
        if !is_active {
            db::sessions::revoke_all_for_user(pool, id)
                .await
                .map_err(AuthError::Internal)?;
            db::refresh_tokens::revoke_all_for_user(pool, id, None)
                .await
                .map_err(AuthError::Internal)?;
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/users/me — get own profile
#[utoipa::path(get, path = "/me", tag = "users", security(("bearer" = [])),
    responses((status = 200, body = UserResponse), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn get_self(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<UserResponse>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;
    Ok(Json(user_to_response(user)))
}

/// PATCH /api/v1/users/me — update own profile (display_name only)
#[utoipa::path(patch, path = "/me", tag = "users", security(("bearer" = [])),
    request_body = UpdateSelfRequest,
    responses(
        (status = 204, description = "Updated"),
        (status = 400, description = "Empty display name, or more than 20 discovery languages", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn update_self(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<UpdateSelfRequest>,
) -> Result<StatusCode, AuthError> {
    if let Some(ref name) = body.display_name {
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(AuthError::BadRequest("display name cannot be empty".into()));
        }
        db::users::update_display_name(state.db(), auth.user_id, trimmed)
            .await
            .map_err(AuthError::Internal)?;
    }
    if let Some(enabled) = body.recommendations_enabled {
        db::users::set_recommendations_enabled(state.db(), auth.user_id, enabled)
            .await
            .map_err(AuthError::Internal)?;
    }
    if let Some(ref languages) = body.discovery_languages {
        // Folded here rather than trusted: a client sending `en-US` would
        // otherwise store a value that matches nothing, since every filter in
        // the catalogue reads the folded `language_base`.
        let mut folded: Vec<String> = Vec::new();
        for tag in languages {
            if let Some(base) = crate::podcasts::base_language(tag) {
                if !folded.contains(&base) {
                    folded.push(base);
                }
            }
        }
        if folded.len() > 20 {
            return Err(AuthError::BadRequest("too many discovery languages".into()));
        }
        db::users::set_discovery_languages(state.db(), auth.user_id, &folded)
            .await
            .map_err(AuthError::Internal)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/users/me/subsonic-key — fetch (creating if needed) the
/// caller's OpenSubsonic API key, used to configure external music clients.
#[utoipa::path(get, path = "/me/subsonic-key", tag = "users", security(("bearer" = [])),
    responses((status = 200, body = SubsonicKeyResponse), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn get_subsonic_key(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<SubsonicKeyResponse>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    let api_key = db::subsonic::get_or_create_key(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(SubsonicKeyResponse { username: user.email, api_key }))
}

/// POST /api/v1/users/me/subsonic-key/regenerate — rotate the caller's
/// OpenSubsonic API key, invalidating any previously configured client.
#[utoipa::path(post, path = "/me/subsonic-key/regenerate", tag = "users", security(("bearer" = [])),
    responses((status = 200, body = SubsonicKeyResponse), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn regenerate_subsonic_key(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<SubsonicKeyResponse>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    let api_key = db::subsonic::regenerate_key(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(SubsonicKeyResponse { username: user.email, api_key }))
}

/// POST /api/v1/users/me/avatar — set the caller's own profile picture. Self-serve only:
/// there is no "set someone else's avatar" admin action, matching how a real photo works.
#[utoipa::path(post, path = "/me/avatar", tag = "users", security(("bearer" = [])),
    request_body(content_type = "multipart/form-data", description = "fields: `avatar` (an image file, at most 5 MB)"),
    responses(
        (status = 204, description = "Set"),
        (status = 400, description = "Missing `avatar` field, not an image, or too large", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody),
        (status = 413, description = "Request body over 6 MB")))]
async fn upload_avatar(
    auth: AuthUser,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    // The field is fully drained (`.bytes().await`) inside the loop body, not stored for later
    // use outside it — `Field<'_>` borrows `multipart`, and holding one past a `break` while the
    // loop's own scrutinee keeps calling `multipart.next_field()` on other iterations doesn't
    // satisfy the borrow checker even though only one field is ever kept.
    let mut avatar: Option<(String, String, axum::body::Bytes)> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        if field.name() != Some("avatar") {
            continue;
        }
        let file_name = field.file_name().map(ToOwned::to_owned).unwrap_or_else(|| "avatar.jpg".to_string());
        let content_type = field
            .content_type()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let bytes = field
            .bytes()
            .await
            .map_err(|e| AuthError::BadRequest(format!("invalid upload: {e}")))?;
        avatar = Some((file_name, content_type, bytes));
        break;
    }
    let (file_name, content_type, bytes) =
        avatar.ok_or_else(|| AuthError::BadRequest("missing avatar payload".into()))?;

    if !content_type.starts_with("image/") {
        return Err(AuthError::BadRequest("avatar must be an image".into()));
    }
    if bytes.len() > MAX_AVATAR_BYTES {
        return Err(AuthError::BadRequest(format!(
            "avatar must be under {}MB",
            MAX_AVATAR_BYTES / 1_000_000
        )));
    }

    let ext = avatar_extension(&file_name).unwrap_or("jpg");
    // Deterministic key (not content-addressed): a re-upload with the same extension
    // overwrites in place via `upsert_object`'s `ON CONFLICT`, so there's nothing to garbage
    // collect on the common "change my photo" path. Outside the `f/{family_id}/…` prefix
    // (`storage::family_key`), so avatars are never scooped into family storage billing —
    // `db::storage_usage::family_storage` filters strictly on that prefix.
    let object_key = format!("u/{}/avatar.{ext}", auth.user_id);
    let size_bytes = bytes.len() as i64;

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    state
        .storage()
        .put(&object_key, bytes, &content_type)
        .await
        .map_err(AuthError::Internal)?;
    let media_id = db::media::upsert_object(
        &mut *tx,
        state.storage().bucket(),
        &object_key,
        &content_type,
        Some(size_bytes),
    )
    .await
    .map_err(AuthError::Internal)?;

    db::users::set_avatar(&mut *tx, auth.user_id, Some(media_id))
        .await
        .map_err(AuthError::Internal)?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/users/me/avatar — remove the caller's own profile picture. The stored object
/// is left in place (same as every other cover-replace path in this API); nothing here scans
/// for orphaned `media_objects` rows.
#[utoipa::path(delete, path = "/me/avatar", tag = "users", security(("bearer" = [])),
    responses((status = 204, description = "Removed"), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn delete_avatar(auth: AuthUser, State(state): State<AppState>) -> Result<StatusCode, AuthError> {
    db::users::set_avatar(state.db(), auth.user_id, None)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/users/:id/avatar — streams the raw image. Visible to the user themselves or any
/// fellow member of their family; unlike `GET /users/:id`, an instance admin gets no special
/// case here (they'd need to actually share a family to see it, same as anyone else).
#[utoipa::path(get, path = "/{id}/avatar", tag = "users", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 200, description = "The image", content_type = "image/*"),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "No avatar, or not the caller or a fellow family member", body = crate::http::openapi::ErrorBody)))]
async fn get_avatar(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AuthError> {
    if id != family.user_id {
        let is_fellow_member = db::families::list_members(state.db(), family.family_id)
            .await
            .map_err(AuthError::Internal)?
            .iter()
            .any(|m| m.user_id == id);
        if !is_fellow_member {
            return Err(AuthError::ItemNotFound);
        }
    }

    let (object_key, content_type) = db::users::find_avatar(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;
    Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response())
}

fn avatar_extension(file_name: &str) -> Option<&str> {
    StdPath::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim_matches('.'))
        .filter(|ext| !ext.is_empty())
}

/// DELETE /api/v1/users/me — delete own account
#[utoipa::path(delete, path = "/me", tag = "users", security(("bearer" = [])),
    responses(
        (status = 204, description = "The account and everything it owns are deleted"),
        (status = 400, description = "The caller is the last admin", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn delete_self(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<StatusCode, AuthError> {
    if auth.role == "admin" {
        // Prevent admin from self-deleting if they are the last admin
        let users = db::users::list_all(state.db())
            .await
            .map_err(AuthError::Internal)?;
        let admin_count = users.iter().filter(|u| u.role == "admin" && u.is_active).count();
        if admin_count <= 1 {
            return Err(AuthError::BadRequest(
                "cannot delete the last admin account".into(),
            ));
        }
    }
    delete_account(&state, auth.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/users/:id — admin only: hard-delete user and all their data
#[utoipa::path(delete, path = "/{id}", tag = "users", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses(
        (status = 204, description = "The account and everything it owns are deleted"),
        (status = 400, description = "The id is the caller's own; use `DELETE /users/me`", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Invalid access token, or the caller is not an admin (answered 401, not 403)", body = crate::http::openapi::ErrorBody)))]
async fn admin_delete_user(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    require_admin(&auth)?;

    // Prevent deleting yourself via this route
    if id == auth.user_id {
        return Err(AuthError::BadRequest("use DELETE /users/me to delete your own account".into()));
    }

    delete_account(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Delete an account with everything it owns — its trash included, which
/// skips the 30 days: an account deletion is immediate (GDPR) — and the stored
/// objects that leaves unreferenced.
///
/// Objects are found by snapshot: what under the user's own and their
/// family's key prefixes is referenced *before* the delete, and is no longer
/// referenced after it. Another member's upload still in flight is
/// unreferenced before too, so it is never in the snapshot.
async fn delete_account(state: &AppState, user_id: Uuid) -> Result<(), AuthError> {
    let pool = state.db();
    let membership = db::families::find_membership(pool, user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut prefixes = vec![format!("music/{user_id}/")];
    if let Some(m) = &membership {
        prefixes.push(crate::storage::family_key(m.family_id, ""));
    }
    let candidates = db::media::referenced_under_prefixes(pool, &prefixes)
        .await
        .map_err(AuthError::Internal)?;

    db::users::delete_user(pool, user_id)
        .await
        .map_err(AuthError::Internal)?;

    // family_members.user_id cascades on user delete, but nothing else
    // prunes the family that leaves behind — unlike move_to_family /
    // move_to_personal_family, which already do this for every other way a
    // family can lose its last member. Without this, deleting an account
    // leaks an empty family forever.
    if let Some(m) = membership {
        db::families::delete_if_empty(pool, m.family_id)
            .await
            .map_err(AuthError::Internal)?;
    }

    crate::trash::delete_orphans(pool, state.storage(), &candidates).await;
    Ok(())
}

/// POST /api/v1/users/:id/revoke-sessions — admin only: force-logout user
#[utoipa::path(post, path = "/{id}/revoke-sessions", tag = "users", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "User id")),
    responses((status = 204, description = "Every session and device of the account is signed out"), (status = 401, description = "Invalid access token, or the caller is not an admin (answered 401, not 403)", body = crate::http::openapi::ErrorBody)))]
async fn admin_revoke_sessions(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    require_admin(&auth)?;
    db::sessions::revoke_all_for_user(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    db::refresh_tokens::revoke_all_for_user(state.db(), id, None)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn require_admin(auth: &AuthUser) -> Result<(), AuthError> {
    if auth.role != "admin" {
        Err(AuthError::SessionInvalid)
    } else {
        Ok(())
    }
}

fn user_to_response(u: crate::users::models::User) -> UserResponse {
    UserResponse {
        id: u.id.to_string(),
        email: u.email,
        display_name: u.display_name,
        role: u.role,
        is_active: u.is_active,
        avatar_url: u.avatar_object_id.map(|_| format!("/api/v1/users/{}/avatar", u.id)),
        recommendations_enabled: u.recommendations_enabled,
        discovery_languages: u.discovery_languages.clone(),
        created_at: u.created_at.to_rfc3339(),
    }
}
