// SPDX-License-Identifier: AGPL-3.0-or-later
//! Running the server (docs/api-v2-completion-plan.md, phase 12): the first admin, then the
//! server administrator's accounts, households, totals and jobs. Everything under `/admin` is
//! `instance_admin` — a household's own admin manages the household, not the server.
//!
//! Two refusals v1 does not make, so that an administrator cannot shut themselves out: their own
//! account is not demoted, blocked or deleted here (`409 own_account`), and the household they are
//! in is not locked (`409 own_family`) — on a one-household server that would lock everyone out,
//! the administrator included, with nobody left to unlock it.

use crate::app::AppState;
use crate::auth::audit::{self, RequestMeta};
use crate::auth::middleware::InstanceAdmin;
use crate::db;
use crate::db::admin::{FamilyRow, FamilyState, UserRow};
use crate::families::BillingStorage;
use crate::v2::me::{self, Me};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// ── Setup ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SetupRequest {
    pub email: String,
    /// Twelve characters or more; a few words are fine.
    pub password: String,
    pub display_name: String,
}

/// An account's fields as typed, checked: the address folded to lower case, the name trimmed.
pub(crate) fn new_account(email: &str, password: &str, display_name: &str) -> Result<(String, String), Problem> {
    let email = email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(Problem::bad_request("invalid_email", "an email address is required").field("/email", "invalid", "not an email address"));
    }
    crate::v2::account::check_password(password, "/password")?;
    let display_name = display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(Problem::bad_request("invalid_display_name", "a name is required").field("/display_name", "required", "not blank"));
    }
    Ok((email, display_name))
}

fn setup_complete() -> Problem {
    Problem::new(StatusCode::CONFLICT, "setup_complete").detail("this server has its administrator; sign in instead")
}

/// The server's first account, its administrator, while `setup.complete` in discovery is
/// `false`; afterwards `409 setup_complete`. Answers the account; signing in is
/// `POST /auth/token`. The library folders are scanned straight away.
#[utoipa::path(post, path = "/setup", tag = "setup",
    extensions(("x-permission" = json!("public"))),
    request_body = SetupRequest,
    responses(
        (status = 201, description = "The administrator; sign in with `POST /auth/token`", body = Me),
        (status = 400, description = "`invalid_email`, `weak_password`, `invalid_display_name`", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`setup_complete`: the server has its administrator", body = crate::v2::problem::ProblemBody)))]
pub async fn setup(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<SetupRequest>) -> Result<(StatusCode, Json<Me>), Problem> {
    let pool = state.db();
    // Refused before the body is looked at: once set up, this route tells nothing more.
    if db::admin::totals(pool).await?.0 > 0 {
        return Err(setup_complete());
    }
    let (email, display_name) = new_account(&body.email, &body.password, &body.display_name)?;
    // Two first requests at once make one administrator: the second waits, then finds it.
    let mut lock = pool.acquire().await.map_err(|e| Problem::internal(anyhow::Error::from(e)))?;
    sqlx::query("SELECT pg_advisory_lock(hashtext('own-audio:setup'))")
        .execute(&mut *lock)
        .await
        .map_err(|e| Problem::internal(anyhow::Error::from(e)))?;
    let made = async {
        if db::admin::totals(pool).await?.0 > 0 {
            return Err(setup_complete());
        }
        Ok(crate::auth::create_vouched_account(&state, &email, &body.password, &display_name, "admin").await?)
    }
    .await;
    let _ = sqlx::query("SELECT pg_advisory_unlock(hashtext('own-audio:setup'))").execute(&mut *lock).await;
    let user = made?;
    tracing::info!(email = %user.email, "initial admin user created via setup (v2)");
    audit::record(&state, &meta, Some(user.id), None, "account.created", serde_json::json!({ "via": "setup" })).await;
    crate::library_folders::request_scan();
    Ok((StatusCode::CREATED, Json(me::build_for(&state, user.id).await?)))
}

// ── Accounts ────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct AdminUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: String,
    /// `admin` runs the server; `user` is everyone else.
    pub instance_role: String,
    /// May not sign in (blocked by an administrator, or their household locked).
    pub blocked: bool,
    pub email_verified: bool,
    /// Made by a household admin and not claimed yet: nobody can sign in as it.
    pub pending: bool,
    pub created_at: DateTime<Utc>,
    /// The household; absent for an account that has none.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub family: Option<AdminUserFamily>,
}

#[derive(Serialize, ToSchema)]
pub struct AdminUserFamily {
    pub id: Uuid,
    pub name: String,
    /// `admin` or `member` of the household.
    pub role: String,
}

fn family_role(role: &str) -> &'static str {
    if role == "family_admin" { "admin" } else { "member" }
}

fn instance_role(role: &str) -> &'static str {
    if role == "admin" { "admin" } else { "user" }
}

fn admin_user(row: UserRow) -> AdminUser {
    let family = match (row.family_id, row.family_name, row.family_role) {
        (Some(id), Some(name), Some(role)) => Some(AdminUserFamily { id, name, role: family_role(&role).to_string() }),
        _ => None,
    };
    AdminUser {
        id: row.id,
        email: row.email,
        display_name: row.display_name,
        instance_role: instance_role(&row.role).to_string(),
        blocked: !row.is_active,
        email_verified: row.email_verified_at.is_some(),
        pending: row.pending,
        created_at: row.created_at,
        family,
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct UsersQuery {
    /// A part of the address or the name, any case.
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Every account on the server, in the order they were made.
#[utoipa::path(get, path = "/admin/users", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(UsersQuery),
    responses((status = 200, body = Page<AdminUser>),
        (status = 400, description = "`invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody)))]
pub async fn list_users(_admin: InstanceAdmin, State(state): State<AppState>, Query(q): Query<UsersQuery>) -> Result<Json<Page<AdminUser>>, Problem> {
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let search = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty());
    let rows = db::admin::users_page(state.db(), search, page.after::<(DateTime<Utc>, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |r| (r.created_at, r.id));
    Ok(Json(Page { items: page.items.into_iter().map(admin_user).collect(), next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct NewUser {
    pub email: String,
    /// Twelve characters or more.
    pub password: String,
    pub display_name: String,
    /// `user` (the default) or `admin`, who runs the server.
    pub instance_role: Option<String>,
}

fn parse_role(role: Option<&str>) -> Result<Option<&'static str>, Problem> {
    match role.map(str::trim) {
        None => Ok(None),
        Some("user") => Ok(Some("user")),
        Some("admin") => Ok(Some("admin")),
        Some(other) => Err(Problem::bad_request("invalid_role", "instance_role is `user` or `admin`")
            .field("/instance_role", "invalid", other.to_string())),
    }
}

/// An account made by the administrator, whatever registration allows, with a password and a
/// place in a household (the server's one, on a one-household server). The address counts as
/// confirmed. Send an `Idempotency-Key` to retry safely.
#[utoipa::path(post, path = "/admin/users", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    request_body = NewUser,
    responses(
        (status = 201, body = AdminUser),
        (status = 400, description = "`invalid_email`, `weak_password`, `invalid_display_name`, `invalid_role`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`email_taken`", body = crate::v2::problem::ProblemBody)))]
pub async fn create_user(
    InstanceAdmin(auth): InstanceAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Json(body): Json<NewUser>,
) -> Result<(StatusCode, Json<AdminUser>), Problem> {
    let (email, display_name) = new_account(&body.email, &body.password, &body.display_name)?;
    let role = parse_role(body.instance_role.as_deref())?.unwrap_or("user");
    if db::users::find_by_email(state.db(), &email).await?.is_some() {
        return Err(Problem::new(StatusCode::CONFLICT, "email_taken").detail("an account with this email exists"));
    }
    let user = crate::auth::create_vouched_account(&state, &email, &body.password, &display_name, role).await?;
    audit::record(&state, &meta, Some(user.id), Some(auth.user_id), "account.created", serde_json::json!({ "role": role })).await;
    let row = db::admin::user(state.db(), user.id).await?.ok_or_else(Problem::not_found)?;
    Ok((StatusCode::CREATED, Json(admin_user(row))))
}

/// One account.
#[utoipa::path(get, path = "/admin/users/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AdminUser),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn get_user(_admin: InstanceAdmin, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<AdminUser>, Problem> {
    let row = db::admin::user(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(admin_user(row)))
}

#[derive(Deserialize, ToSchema)]
pub struct UserPatch {
    pub display_name: Option<String>,
    /// `user` or `admin`.
    pub instance_role: Option<String>,
    /// `true` stops the account signing in and signs it out everywhere; `false` lets it in again.
    pub blocked: Option<bool>,
}

fn own_account() -> Problem {
    Problem::new(StatusCode::CONFLICT, "own_account")
        .detail("an administrator does not demote, block or delete their own account here; use `/me`")
}

/// What a change asks of the account, checked before anything is written.
pub(crate) fn check_patch(caller: Uuid, target: Uuid, body: &UserPatch) -> Result<(Option<String>, Option<&'static str>), Problem> {
    let name = match body.display_name.as_deref().map(str::trim) {
        Some("") => {
            return Err(Problem::bad_request("invalid_display_name", "a name is required").field("/display_name", "required", "not blank"));
        }
        other => other.map(str::to_string),
    };
    let role = parse_role(body.instance_role.as_deref())?;
    if caller == target && (role == Some("user") || body.blocked == Some(true)) {
        return Err(own_account());
    }
    Ok((name, role))
}

/// Rename an account, make it the server's administrator or take that away, block it or let it
/// in again. Blocking it or changing its role signs it out on every device: the role is in its
/// access tokens.
#[utoipa::path(patch, path = "/admin/users/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    request_body = UserPatch,
    responses(
        (status = 200, body = AdminUser),
        (status = 400, description = "`invalid_display_name`, `invalid_role`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`own_account`: not one's own role or access", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_user(
    InstanceAdmin(auth): InstanceAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(id): Path<Uuid>,
    Json(body): Json<UserPatch>,
) -> Result<Json<AdminUser>, Problem> {
    let (name, role) = check_patch(auth.user_id, id, &body)?;
    let pool = state.db();
    if role.is_some() || body.blocked.is_some() {
        audit::record(&state, &meta, Some(id), Some(auth.user_id), "account.changed",
            serde_json::json!({ "role": role, "is_active": body.blocked.map(|b| !b) })).await;
    }
    if let Some(name) = &name {
        db::users::update_display_name(pool, id, name).await?;
    }
    let before = db::admin::user(pool, id).await?.ok_or_else(Problem::not_found)?;
    if let Some(role) = role {
        db::admin::set_role(pool, id, role).await?;
    }
    if let Some(blocked) = body.blocked {
        db::users::set_active(pool, id, !blocked).await?;
    }
    // The role travels in the access token: a changed one, like a block, takes effect when the
    // account signs in again, so it is signed out now rather than at the token's expiry.
    let role_changed = role.is_some_and(|r| r != instance_role(&before.role));
    if body.blocked == Some(true) || role_changed {
        db::sessions::revoke_all_for_user(pool, id).await?;
        db::refresh_tokens::revoke_all_for_user(pool, id, None).await?;
    }
    let row = db::admin::user(pool, id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(admin_user(row)))
}

/// Delete an account with everything it owns, its trash included, at once; a household left
/// empty goes with it.
#[utoipa::path(delete, path = "/admin/users/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`own_account`: delete one's own with `DELETE /me`", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_user(
    InstanceAdmin(auth): InstanceAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    if id == auth.user_id {
        return Err(own_account());
    }
    db::admin::user(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    audit::record(&state, &meta, Some(id), Some(auth.user_id), "account.deleted", serde_json::json!({})).await;
    crate::users::delete_account(&state, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Sign an account out on every device: its refresh tokens and access tokens stop working.
#[utoipa::path(delete, path = "/admin/users/{id}/sessions", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204, description = "Signed out everywhere"),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn delete_user_sessions(
    InstanceAdmin(auth): InstanceAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, Problem> {
    let pool = state.db();
    db::admin::user(pool, id).await?.ok_or_else(Problem::not_found)?;
    db::sessions::revoke_all_for_user(pool, id).await?;
    db::refresh_tokens::revoke_all_for_user(pool, id, None).await?;
    audit::record(&state, &meta, Some(id), Some(auth.user_id), "session.revoked_all", serde_json::json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Households ──────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct AdminFamily {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub member_count: i64,
    /// It has members and none of them may sign in.
    pub locked: bool,
    pub trash: FamilyTrash,
}

/// What the household has in its trash, and what it took back in the last 30 days.
#[derive(Serialize, ToSchema)]
pub struct FamilyTrash {
    pub items: i64,
    pub bytes: i64,
    pub restores_30d: i64,
    pub restored_bytes_30d: i64,
    /// The most times any one item was restored in 30 days.
    pub max_restores_one_item_30d: i64,
    /// Worth a look: more restored in 30 days than the library holds, or one item restored
    /// more than three times. Restores pay for their days in the trash; this is for noticing.
    pub flagged: bool,
}

#[derive(Serialize, ToSchema)]
pub struct AdminFamilyDetail {
    pub id: Uuid,
    pub name: String,
    pub created_at: DateTime<Utc>,
    pub member_count: i64,
    pub locked: bool,
    pub trash: FamilyTrash,
    pub members: Vec<AdminFamilyMember>,
    /// What the household keeps, in bytes.
    pub storage: BillingStorage,
}

#[derive(Serialize, ToSchema)]
pub struct AdminFamilyMember {
    pub user_id: Uuid,
    pub email: String,
    pub display_name: String,
    /// What the household calls them, when it calls them something else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    /// `admin` or `member` of the household.
    pub role: String,
    pub blocked: bool,
    /// Made by a household admin and not claimed yet.
    pub pending: bool,
    pub joined_at: DateTime<Utc>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FamiliesQuery {
    /// `active` (someone may sign in, or nobody is in it), `locked` (members, none of whom may
    /// sign in) or `trashed` (something in its trash); every household when absent.
    pub state: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

pub(crate) fn family_state(state: Option<&str>) -> Result<Option<FamilyState>, Problem> {
    match state.map(str::trim) {
        None | Some("") => Ok(None),
        Some("active") => Ok(Some(FamilyState::Active)),
        Some("locked") => Ok(Some(FamilyState::Locked)),
        Some("trashed") => Ok(Some(FamilyState::Trashed)),
        Some(other) => Err(Problem::bad_request("invalid_state", "state is `active`, `locked` or `trashed`")
            .field("state", "invalid", other.to_string())),
    }
}

fn is_locked(row: &FamilyRow) -> bool {
    row.member_count > 0 && row.active_members == 0
}

/// The trash figures of these households, as v1's trash report computes them.
async fn trash_of(state: &AppState, ids: &[Uuid]) -> Result<std::collections::HashMap<Uuid, FamilyTrash>, Problem> {
    let mut out = std::collections::HashMap::new();
    for s in db::trash::family_stats_for(state.db(), Some(ids)).await? {
        // The library's size is four queries; only worth it where something came back.
        let library_bytes = if s.restores_30d > 0 {
            Some(db::storage_usage::family_storage(state.db(), state.storage().bucket(), s.family_id).await?.total_bytes)
        } else {
            None
        };
        let flagged = s.max_restores_one_item_30d > 3 || library_bytes.is_some_and(|lib| s.restored_bytes_30d > lib.max(1));
        out.insert(s.family_id, FamilyTrash {
            items: s.trashed_items,
            bytes: s.trashed_bytes,
            restores_30d: s.restores_30d,
            restored_bytes_30d: s.restored_bytes_30d,
            max_restores_one_item_30d: s.max_restores_one_item_30d,
            flagged,
        });
    }
    Ok(out)
}

fn empty_trash() -> FamilyTrash {
    FamilyTrash { items: 0, bytes: 0, restores_30d: 0, restored_bytes_30d: 0, max_restores_one_item_30d: 0, flagged: false }
}

/// Every household on the server, in the order they were made.
#[utoipa::path(get, path = "/admin/families", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(FamiliesQuery),
    responses((status = 200, body = Page<AdminFamily>),
        (status = 400, description = "`invalid_state`, `invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody)))]
pub async fn list_families(_admin: InstanceAdmin, State(state): State<AppState>, Query(q): Query<FamiliesQuery>) -> Result<Json<Page<AdminFamily>>, Problem> {
    let filter = family_state(q.state.as_deref())?;
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let rows = db::admin::families_page(state.db(), filter, page.after::<(DateTime<Utc>, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |r| (r.created_at, r.id));
    let ids: Vec<Uuid> = page.items.iter().map(|r| r.id).collect();
    let mut trash = trash_of(&state, &ids).await?;
    let items = page
        .items
        .into_iter()
        .map(|r| AdminFamily {
            locked: is_locked(&r),
            trash: trash.remove(&r.id).unwrap_or_else(empty_trash),
            id: r.id,
            name: r.name,
            created_at: r.created_at,
            member_count: r.member_count,
        })
        .collect();
    Ok(Json(Page { items, next: page.next, included: None }))
}

async fn family_detail(state: &AppState, id: Uuid) -> Result<AdminFamilyDetail, Problem> {
    let pool = state.db();
    let row = db::admin::family(pool, id).await?.ok_or_else(Problem::not_found)?;
    let members = db::families::list_members(pool, id).await?;
    let s = db::storage_usage::family_storage(pool, state.storage().bucket(), id).await?;
    let trash = trash_of(state, &[id]).await?.remove(&id).unwrap_or_else(empty_trash);
    Ok(AdminFamilyDetail {
        locked: is_locked(&row),
        id: row.id,
        name: row.name,
        created_at: row.created_at,
        member_count: row.member_count,
        trash,
        members: members
            .into_iter()
            .map(|m| AdminFamilyMember {
                user_id: m.user_id,
                email: m.email,
                display_name: m.display_name,
                label: m.display_label,
                role: family_role(&m.role).to_string(),
                blocked: !m.is_active,
                pending: m.pending,
                joined_at: m.joined_at,
            })
            .collect(),
        storage: BillingStorage {
            total_bytes: s.total_bytes,
            audiobooks_bytes: s.audiobooks_bytes,
            podcasts_bytes: s.podcasts_bytes,
            music_bytes: s.music_bytes,
            other_bytes: s.other_bytes,
            unsized_objects: s.unsized_objects,
        },
    })
}

/// One household: its people, what it keeps and its trash.
#[utoipa::path(get, path = "/admin/families/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = AdminFamilyDetail),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn get_family(_admin: InstanceAdmin, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<AdminFamilyDetail>, Problem> {
    Ok(Json(family_detail(&state, id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct FamilyPatch {
    /// `true` stops every member signing in and signs them out everywhere; `false` lets every
    /// member in again (they sign in anew).
    pub locked: bool,
}

/// Lock a household or let it in again. Not the household the administrator is in
/// (`409 own_family`).
#[utoipa::path(patch, path = "/admin/families/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    request_body = FamilyPatch,
    responses(
        (status = 200, body = AdminFamilyDetail),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`own_family`: locking it would lock the administrator out", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_family(
    InstanceAdmin(auth): InstanceAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(id): Path<Uuid>,
    Json(body): Json<FamilyPatch>,
) -> Result<Json<AdminFamilyDetail>, Problem> {
    let pool = state.db();
    db::admin::family(pool, id).await?.ok_or_else(Problem::not_found)?;
    let members = db::families::list_members(pool, id).await?;
    if body.locked {
        if members.iter().any(|m| m.user_id == auth.user_id) {
            return Err(Problem::new(StatusCode::CONFLICT, "own_family")
                .detail("this is the household you are in: locking it would lock you out"));
        }
        for m in members.iter().filter(|m| m.is_active) {
            db::users::set_active(pool, m.user_id, false).await?;
            db::sessions::revoke_all_for_user(pool, m.user_id).await?;
            db::refresh_tokens::revoke_all_for_user(pool, m.user_id, None).await?;
        }
    } else {
        for m in members.iter().filter(|m| !m.is_active) {
            db::users::set_active(pool, m.user_id, true).await?;
        }
    }
    audit::record(&state, &meta, None, Some(auth.user_id), if body.locked { "family.locked" } else { "family.unlocked" },
        serde_json::json!({ "family_id": id })).await;
    Ok(Json(family_detail(&state, id).await?))
}

/// Delete a household nobody is in any more (one left behind by deleting accounts). A household
/// with people is not deleted: delete or move its accounts first (`409 family_not_empty`).
#[utoipa::path(delete, path = "/admin/families/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    responses(
        (status = 204, description = "Deleted"),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`family_not_empty`", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_family(_admin: InstanceAdmin, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    let pool = state.db();
    db::admin::family(pool, id).await?.ok_or_else(Problem::not_found)?;
    if !db::families::delete_if_empty(pool, id).await? {
        return Err(Problem::new(StatusCode::CONFLICT, "family_not_empty").detail("people are still in this household"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── The server's figures ────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct AdminStats {
    pub users: i64,
    pub families: i64,
    /// Accounts made in the last day, week and 30 days (not ones made for someone who has not
    /// claimed them yet).
    pub registrations: crate::dashboard::WindowCounts,
    pub sign_ins: crate::dashboard::WindowCounts,
    pub storage: StorageTotals,
    /// Everything in every household's trash.
    pub trash_bytes: i64,
    pub listening: ListeningWindows,
}

#[derive(Serialize, ToSchema)]
pub struct StorageTotals {
    pub audio_bytes: i64,
    pub other_bytes: i64,
    pub total_bytes: i64,
}

#[derive(Serialize, ToSchema)]
pub struct ListeningWindows {
    pub d1: Listening,
    pub d7: Listening,
    pub d30: Listening,
}

#[derive(Serialize, ToSchema)]
pub struct Listening {
    pub listened_ms: i64,
    pub sessions: i64,
    /// People who listened.
    pub listeners: i64,
}

fn listening((seconds, sessions, listeners): (i64, i64, i64)) -> Listening {
    Listening { listened_ms: seconds.saturating_mul(1000), sessions, listeners }
}

/// The server at a glance: accounts, households, storage, the trash, sign-ins and listening.
#[utoipa::path(get, path = "/admin/stats", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    responses((status = 200, body = AdminStats),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody)))]
pub async fn stats(_admin: InstanceAdmin, State(state): State<AppState>) -> Result<Json<AdminStats>, Problem> {
    let pool = state.db();
    let (users, families) = db::admin::totals(pool).await?;
    let (audio_bytes, other_bytes) = db::storage_usage::instance_storage_by_type(pool, state.storage().bucket()).await?;
    let trash_bytes = db::trash::family_stats(pool).await?.iter().map(|t| t.trashed_bytes).sum();
    let registrations = db::users::registration_counts(pool).await?;
    let sign_ins = db::sessions::login_counts(pool).await?;
    let (d1, d7, d30) = db::stats::streaming_totals(pool).await?;
    Ok(Json(AdminStats {
        users,
        families,
        registrations: registrations.into(),
        sign_ins: sign_ins.into(),
        storage: StorageTotals { audio_bytes, other_bytes, total_bytes: audio_bytes + other_bytes },
        trash_bytes,
        listening: ListeningWindows { d1: listening(d1), d7: listening(d7), d30: listening(d30) },
    }))
}

// ── Jobs ────────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Job {
    pub id: Uuid,
    /// What it does: `feed_refresh`, `media_checksum`, `disc_backfill`, `trash_purge`…; a server
    /// may add kinds.
    pub kind: String,
    /// `pending`, `running`, `completed` or `failed`.
    pub state: String,
    pub attempts: i32,
    pub max_attempts: i32,
    /// Why it failed, the last time it did.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What a job reports when done, for those that do (the disc back-fill's `{checked, left}`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result: Option<serde_json::Value>,
    pub scheduled_at: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

fn job(j: crate::jobs::models::Job) -> Job {
    Job {
        id: j.id,
        kind: j.job_type,
        state: j.status,
        attempts: j.attempts,
        max_attempts: j.max_attempts,
        error: j.error,
        result: j.result,
        scheduled_at: j.scheduled_at,
        started_at: j.started_at,
        completed_at: j.completed_at,
        created_at: j.created_at,
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct JobsQuery {
    /// Only jobs of this kind.
    pub kind: Option<String>,
    /// `pending`, `running`, `completed` or `failed`.
    pub state: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

pub(crate) fn job_filters(q: &JobsQuery) -> Result<(Option<&str>, Option<&str>), Problem> {
    let state = match q.state.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(s @ ("pending" | "running" | "completed" | "failed")) => Some(s),
        Some(other) => {
            return Err(Problem::bad_request("invalid_state", "state is `pending`, `running`, `completed` or `failed`")
                .field("state", "invalid", other.to_string()));
        }
    };
    let kind = match q.kind.as_deref().map(str::trim) {
        None | Some("") => None,
        Some(k) if k.len() <= 64 && k.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') => Some(k),
        Some(other) => {
            return Err(Problem::bad_request("invalid_kind", "kind is a job kind such as `feed_refresh`")
                .field("kind", "invalid", other.chars().take(64).collect::<String>()));
        }
    };
    Ok((kind, state))
}

/// The server's background jobs, newest first.
#[utoipa::path(get, path = "/admin/jobs", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(JobsQuery),
    responses((status = 200, body = Page<Job>),
        (status = 400, description = "`invalid_state`, `invalid_kind`, `invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody)))]
pub async fn list_jobs(_admin: InstanceAdmin, State(state): State<AppState>, Query(q): Query<JobsQuery>) -> Result<Json<Page<Job>>, Problem> {
    let (kind, job_state) = job_filters(&q)?;
    let page = PageQuery { limit: q.limit, cursor: q.cursor.clone() };
    let limit = page.limit()?;
    let rows = db::admin::jobs_page(state.db(), kind, job_state, page.after::<(DateTime<Utc>, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |j| (j.created_at, j.id));
    Ok(Json(Page { items: page.items.into_iter().map(job).collect(), next: page.next, included: None }))
}

/// One background job.
#[utoipa::path(get, path = "/admin/jobs/{id}", tag = "admin", security(("bearer" = [])),
    extensions(("x-permission" = json!("instance_admin"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Job),
        (status = 403, description = "Not the server's administrator", body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn get_job(_admin: InstanceAdmin, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Job>, Problem> {
    let j = db::jobs::find_by_id(state.db(), id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(job(j)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_account_is_checked_field_by_field() {
        assert_eq!(new_account(" Ann@Example.COM ", "correct horse battery staple", " Ann ").unwrap(),
            ("ann@example.com".to_string(), "Ann".to_string()));
        assert_eq!(new_account("nobody", "correct horse battery staple", "Ann").unwrap_err().code(), "invalid_email");
        assert_eq!(new_account("a@b.c", "short", "Ann").unwrap_err().code(), "weak_password");
        assert_eq!(new_account("a@b.c", "correct horse battery staple", "  ").unwrap_err().code(), "invalid_display_name");
    }

    #[test]
    fn an_administrator_keeps_their_own_rights() {
        let me = Uuid::new_v4();
        let other = Uuid::new_v4();
        let patch = |role: Option<&str>, blocked: Option<bool>| UserPatch {
            display_name: None,
            instance_role: role.map(str::to_string),
            blocked,
        };
        assert_eq!(check_patch(me, me, &patch(Some("user"), None)).unwrap_err().code(), "own_account");
        assert_eq!(check_patch(me, me, &patch(None, Some(true))).unwrap_err().code(), "own_account");
        assert!(check_patch(me, me, &patch(Some("admin"), Some(false))).is_ok(), "nothing taken away");
        assert_eq!(check_patch(me, other, &patch(Some("user"), Some(true))).unwrap().1, Some("user"));
        assert_eq!(check_patch(me, other, &patch(Some("root"), None)).unwrap_err().code(), "invalid_role");
        let blank = UserPatch { display_name: Some(" ".into()), instance_role: None, blocked: None };
        assert_eq!(check_patch(me, other, &blank).unwrap_err().code(), "invalid_display_name");
    }

    #[test]
    fn the_lists_take_their_own_words() {
        assert_eq!(family_state(None).unwrap(), None);
        assert_eq!(family_state(Some("locked")).unwrap(), Some(FamilyState::Locked));
        assert_eq!(family_state(Some("trashed")).unwrap(), Some(FamilyState::Trashed));
        assert_eq!(family_state(Some("deleted")).unwrap_err().code(), "invalid_state");
        let q = |kind: Option<&str>, state: Option<&str>| JobsQuery {
            kind: kind.map(str::to_string),
            state: state.map(str::to_string),
            limit: None,
            cursor: None,
        };
        assert_eq!(job_filters(&q(Some("disc_backfill"), Some("failed"))).unwrap(), (Some("disc_backfill"), Some("failed")));
        assert_eq!(job_filters(&q(None, Some("done"))).unwrap_err().code(), "invalid_state");
        assert_eq!(job_filters(&q(Some("DROP TABLE"), None)).unwrap_err().code(), "invalid_kind");
    }

    #[test]
    fn listening_is_in_milliseconds() {
        let l = listening((90, 2, 1));
        assert_eq!((l.listened_ms, l.sessions, l.listeners), (90_000, 2, 1));
    }
}
