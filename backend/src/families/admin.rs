// SPDX-License-Identifier: AGPL-3.0-or-later
/// Instance-admin, cross-family views — `users.role == "admin"`, not
/// `family_admin`. Nothing here touches membership (that stays the
/// family_admin's job, same boundary `crate::users`' People tab already
/// draws); credit is the one exception — an instance admin can post a manual
/// adjustment, since that's a support/billing action nobody but the server
/// operator can take today. Mounted at `/admin/families`, separate from
/// `/family` (singular, `FamilyContext`-scoped to the caller's own family).
use super::BillingStorage;
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_families))
        .routes(routes!(trash_stats))
        .routes(routes!(get_family_detail, delete_family))
        .routes(routes!(lock_family))
        .routes(routes!(unlock_family))
}

pub fn require_admin(auth: &AuthUser) -> Result<(), AuthError> {
    if auth.role != "admin" {
        Err(AuthError::SessionInvalid)
    } else {
        Ok(())
    }
}

#[derive(Serialize, ToSchema)]
pub struct AdminFamilySummaryResponse {
    pub id: String,
    pub name: String,
    pub member_count: i64,
    pub balance_micro: i64,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct AdminFamilyMemberResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    pub display_label: Option<String>,
    pub role: String,
    pub is_active: bool,
    pub joined_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct AdminFamilyDetailResponse {
    pub id: String,
    pub name: String,
    pub created_at: String,
    pub members: Vec<AdminFamilyMemberResponse>,
    pub storage: BillingStorage,
    pub balance_micro: i64,
    pub last_charge: serde_json::Value,
    pub entries: serde_json::Value,
}

/// GET /api/v1/admin/families/ — every family on the server. No storage
/// figure here: `db::storage_usage::family_storage` is 4 queries per family
/// (see its own doc comment), fine for one family on demand, too expensive
/// to run for every row of a list. Storage only appears on the detail route.
#[utoipa::path(get, path = "/", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = Vec<AdminFamilySummaryResponse>), (status = 401, description = "Not signed in, or not an instance admin", body = crate::http::openapi::ErrorBody)))]
async fn list_families(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminFamilySummaryResponse>>, AuthError> {
    require_admin(&auth)?;

    let families = db::families::list_all(state.db())
        .await
        .map_err(AuthError::Internal)?;
    let balances = state
        .hooks()
        .family_balances(state.db())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        families
            .into_iter()
            .map(|f| AdminFamilySummaryResponse {
                id: f.id.to_string(),
                name: f.name,
                member_count: f.member_count,
                balance_micro: balances.get(&f.id).copied().unwrap_or(0),
                created_at: f.created_at.to_rfc3339(),
            })
            .collect(),
    ))
}

/// GET /api/v1/admin/families/:id — one family's members, storage, and
/// credit history. Same underlying calls as `GET /family/billing`
/// (`families::get_billing`), just against a path param instead of the
/// caller's own `FamilyContext.family_id`.
#[utoipa::path(get, path = "/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The family's id")),
    responses((status = 200, body = AdminFamilyDetailResponse), (status = 401, description = "Not signed in, or not an instance admin", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such family", body = crate::http::openapi::ErrorBody)))]
async fn get_family_detail(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AdminFamilyDetailResponse>, AuthError> {
    require_admin(&auth)?;

    let family = db::families::find(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let members = db::families::list_members(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    let storage = db::storage_usage::family_storage(state.db(), state.storage().bucket(), id)
        .await
        .map_err(AuthError::Internal)?;
    // Credit figures are the edition's (`Hooks::family_extras`); without
    // billing they read 0 / null / [] so the field shapes never change.
    let extras = state
        .hooks()
        .family_extras(state.db(), state.storage(), id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(AdminFamilyDetailResponse {
        id: family.id.to_string(),
        name: family.name,
        created_at: family.created_at.to_rfc3339(),
        members: members
            .into_iter()
            .map(|m| AdminFamilyMemberResponse {
                user_id: m.user_id.to_string(),
                email: m.email,
                display_name: m.display_name,
                display_label: m.display_label,
                role: m.role,
                is_active: m.is_active,
                joined_at: m.joined_at.to_rfc3339(),
            })
            .collect(),
        storage: BillingStorage {
            total_bytes: storage.total_bytes,
            audiobooks_bytes: storage.audiobooks_bytes,
            podcasts_bytes: storage.podcasts_bytes,
            music_bytes: storage.music_bytes,
            other_bytes: storage.other_bytes,
            unsized_objects: storage.unsized_objects,
        },
        balance_micro: extras.get("balance_micro").and_then(|v| v.as_i64()).unwrap_or(0),
        last_charge: extras.get("last_charge").cloned().unwrap_or(serde_json::Value::Null),
        entries: extras.get("entries").cloned().unwrap_or_else(|| serde_json::Value::Array(Vec::new())),
    }))
}

/// DELETE /api/v1/admin/families/:id — only ever deletes an *empty* family
/// (no members). This is a cleanup tool for orphaned families (e.g. one left
/// behind by deleting its last member's account directly, see
/// `users::admin_delete_user`), not a way to remove a real household —
/// there is deliberately no way to delete a family that still has people in
/// it through this route.
#[utoipa::path(delete, path = "/{id}", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The family's id")),
    responses((status = 204, description = "Deleted"), (status = 400, description = "Family still has members", body = crate::http::openapi::ErrorBody), (status = 401, description = "Not signed in, or not an instance admin", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such family", body = crate::http::openapi::ErrorBody)))]
async fn delete_family(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    require_admin(&auth)?;

    db::families::find(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let deleted = db::families::delete_if_empty(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    if deleted {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::BadRequest("family still has members".into()))
    }
}

/// POST /api/v1/admin/families/:id/lock — instance-admin equivalent of a
/// family_admin blocking every member of their own family
/// (`families::block_member`), for a whole household at once: every account
/// that can currently sign in is deactivated and every existing
/// session/refresh token revoked, so an already-signed-in device is cut off
/// immediately too. There is no separate "locked" flag on `families` — a
/// family's lock state is just "none of its members can sign in", read back
/// from the same per-member `is_active` the detail view already returns.
#[utoipa::path(post, path = "/{id}/lock", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The family's id")),
    responses((status = 204, description = "Every member deactivated and signed out"), (status = 401, description = "Not signed in, or not an instance admin", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such family", body = crate::http::openapi::ErrorBody)))]
async fn lock_family(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    require_admin(&auth)?;

    db::families::find(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let members = db::families::list_members(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    for member in members.into_iter().filter(|m| m.is_active) {
        db::users::set_active(state.db(), member.user_id, false)
            .await
            .map_err(AuthError::Internal)?;
        db::sessions::revoke_all_for_user(state.db(), member.user_id)
            .await
            .map_err(AuthError::Internal)?;
        db::refresh_tokens::revoke_all_for_user(state.db(), member.user_id, None)
            .await
            .map_err(AuthError::Internal)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/admin/families/:id/unlock — reactivate every locked member.
/// They still need to sign in again, same as `families::unblock_member`.
#[utoipa::path(post, path = "/{id}/unlock", tag = "admin", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "The family's id")),
    responses((status = 204, description = "Every member reactivated"), (status = 401, description = "Not signed in, or not an instance admin", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such family", body = crate::http::openapi::ErrorBody)))]
async fn unlock_family(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    require_admin(&auth)?;

    db::families::find(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let members = db::families::list_members(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    for member in members.into_iter().filter(|m| !m.is_active) {
        db::users::set_active(state.db(), member.user_id, true)
            .await
            .map_err(AuthError::Internal)?;
    }

    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct AdminTrashStatsResponse {
    pub family_id: String,
    pub family_name: String,
    pub trashed_items: i64,
    pub trashed_bytes: i64,
    pub restores_30d: i64,
    pub restored_bytes_30d: i64,
    pub max_restores_one_item_30d: i64,
    pub library_bytes: Option<i64>,
    /// A pattern worth a look: more restored in 30 days than the whole
    /// library holds, or one item restored more than three times. Restores
    /// already pay for their days in the trash (docs/file-sync-plan.md §2
    /// item 12), so this is for noticing, not for blocking.
    pub flagged: bool,
}

/// GET /api/v1/admin/families/trash — trash size and restore patterns per
/// family.
#[utoipa::path(get, path = "/trash", tag = "admin", security(("bearer" = [])),
    responses((status = 200, body = Vec<AdminTrashStatsResponse>), (status = 401, description = "Not signed in, or not an instance admin", body = crate::http::openapi::ErrorBody)))]
async fn trash_stats(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<AdminTrashStatsResponse>>, AuthError> {
    require_admin(&auth)?;
    let stats = db::trash::family_stats(state.db()).await.map_err(AuthError::Internal)?;
    let mut out = Vec::with_capacity(stats.len());
    for s in stats {
        // family_storage is four queries; only worth it where there were restores.
        let library_bytes = if s.restores_30d > 0 {
            Some(
                db::storage_usage::family_storage(state.db(), state.storage().bucket(), s.family_id)
                    .await
                    .map_err(AuthError::Internal)?
                    .total_bytes,
            )
        } else {
            None
        };
        let flagged = s.max_restores_one_item_30d > 3
            || library_bytes.is_some_and(|lib| s.restored_bytes_30d > lib.max(1));
        out.push(AdminTrashStatsResponse {
            family_id: s.family_id.to_string(),
            family_name: s.family_name,
            trashed_items: s.trashed_items,
            trashed_bytes: s.trashed_bytes,
            restores_30d: s.restores_30d,
            restored_bytes_30d: s.restored_bytes_30d,
            max_restores_one_item_30d: s.max_restores_one_item_30d,
            library_bytes,
            flagged,
        });
    }
    Ok(Json(out))
}
