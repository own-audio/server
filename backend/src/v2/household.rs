// SPDX-License-Identifier: AGPL-3.0-or-later
//! The household (DESIGN §7.5; docs/api-v2-plan.md, V7): the family, its
//! members and their rights, invitations, and who sees an item — in two words,
//! `private` (its owner's) or `family` (the household's).
//!
//! The rules live in v1's handlers — who may change a role, what removing a
//! member does to their library, invitation mail, the security trail — and
//! are called from here, not copied; v2 gives them its own shapes, problem
//! details, and roles as `admin` / `member`.

use crate::app::AppState;
use crate::auth::audit::RequestMeta;
use crate::auth::middleware::AuthUser;
use crate::families::{self, FamilyAdmin, FamilyContext};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

fn v2_role(role: &str) -> String {
    if role == "family_admin" { "admin" } else { "member" }.to_string()
}

fn v1_role(role: &str) -> Result<String, Problem> {
    match role {
        "admin" => Ok("family_admin".into()),
        "member" => Ok("member".into()),
        other => Err(Problem::bad_request("invalid_body", format!("role is `admin` or `member`, not `{other}`")).field("/role", "invalid", other.to_string())),
    }
}

fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Serialize, ToSchema)]
pub struct Family {
    pub id: String,
    pub name: String,
    pub image: Option<String>,
    pub created_at: String,
    /// The caller in it.
    pub me: Membership,
}

#[derive(Serialize, ToSchema)]
pub struct Membership {
    pub user_id: String,
    /// `admin` or `member`.
    pub role: String,
    pub can_upload: bool,
    pub can_generate: bool,
}

fn family(f: families::FamilyResponse) -> Family {
    Family {
        id: f.id,
        name: f.name,
        image: f.avatar_url,
        created_at: f.created_at,
        me: Membership { user_id: f.my_user_id, role: v2_role(&f.my_role), can_upload: f.my_can_upload, can_generate: f.my_can_generate },
    }
}

/// The caller's household.
#[utoipa::path(get, path = "/family", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Family)))]
pub async fn get_family(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<Family>, Problem> {
    let Json(f) = families::get_family(family_ctx, State(state)).await?;
    Ok(Json(family(f)))
}

#[derive(Deserialize, ToSchema)]
pub struct FamilyPatch {
    pub name: String,
}

/// Rename the household.
#[utoipa::path(patch, path = "/family", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    request_body = FamilyPatch,
    responses((status = 200, body = Family)))]
pub async fn patch_family(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, Json(body): Json<FamilyPatch>) -> Result<Json<Family>, Problem> {
    families::update_family(family_ctx.clone(), State(state.clone()), Json(families::UpdateFamilyRequest { name: body.name })).await?;
    let Json(f) = families::get_family(family_ctx, State(state)).await?;
    Ok(Json(family(f)))
}

#[derive(Serialize, ToSchema)]
pub struct Member {
    pub user_id: String,
    pub display_name: String,
    /// What the household calls them, when it differs ("Mum").
    pub label: Option<String>,
    pub email: String,
    /// `admin` or `member`.
    pub role: String,
    pub active: bool,
    /// Made by an admin and not signed in yet.
    pub pending: bool,
    /// `adult`, `teen` or `child`.
    pub age_bracket: String,
    pub can_upload: bool,
    pub can_generate: bool,
    pub image: Option<String>,
    pub joined_at: String,
}

/// Everyone in the household.
#[utoipa::path(get, path = "/family/members", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Vec<Member>)))]
pub async fn list_members(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<Vec<Member>>, Problem> {
    let Json(members) = families::list_members(family_ctx, State(state)).await?;
    Ok(Json(
        members
            .into_iter()
            .map(|m| Member {
                user_id: m.user_id,
                display_name: m.display_name,
                label: m.display_label,
                email: m.email,
                role: v2_role(&m.role),
                active: m.is_active,
                pending: m.pending,
                age_bracket: m.age_bracket,
                can_upload: m.can_upload,
                can_generate: m.can_generate,
                image: m.avatar_url,
                joined_at: m.joined_at,
            })
            .collect(),
    ))
}

#[derive(Deserialize, ToSchema)]
pub struct MemberPatch {
    /// `admin` or `member`.
    pub role: Option<String>,
    /// `adult`, `teen` or `child`; a child never reaches paid work or uploads.
    pub age_bracket: Option<String>,
    pub can_upload: Option<bool>,
    pub can_generate: Option<bool>,
    /// `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub label: Option<Option<String>>,
}

/// Change a member's role, age bracket, rights or label.
#[utoipa::path(patch, path = "/family/members/{user_id}", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("user_id" = Uuid, Path)),
    request_body = MemberPatch,
    responses((status = 204, description = "Changed"), (status = 404, description = "Not a member", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_member(
    FamilyAdmin(family_ctx): FamilyAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(user_id): Path<Uuid>,
    Json(body): Json<MemberPatch>,
) -> Result<StatusCode, Problem> {
    let role = body.role.as_deref().map(v1_role).transpose()?;
    let request = families::UpdateMemberRequest {
        role,
        age_bracket: body.age_bracket,
        can_upload: body.can_upload,
        can_generate: body.can_generate,
        display_label: body.label,
    };
    families::update_member(family_ctx, State(state), meta, Path(user_id), Json(request)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a member out of the household.
#[utoipa::path(delete, path = "/family/members/{user_id}", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("user_id" = Uuid, Path)),
    responses((status = 204, description = "Removed"), (status = 404, description = "Not a member", body = crate::v2::problem::ProblemBody)))]
pub async fn remove_member(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, meta: RequestMeta, Path(user_id): Path<Uuid>) -> Result<StatusCode, Problem> {
    families::remove_member(family_ctx, State(state), meta, Path(user_id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Serialize, ToSchema)]
pub struct Invite {
    pub id: String,
    /// `email`, `link` or `claim`.
    pub kind: String,
    pub email: Option<String>,
    /// What the invitee redeems; deliver it yourself, or let the mail do it.
    pub code: String,
    /// `admin` or `member`.
    pub role: String,
    pub label: Option<String>,
    pub max_uses: i32,
    pub use_count: i32,
    pub created_at: String,
    pub expires_at: String,
    pub join_url: Option<String>,
}

fn invite(i: families::InviteResponse) -> Invite {
    Invite {
        id: i.id,
        kind: i.kind,
        email: i.email,
        code: i.code,
        role: v2_role(&i.role),
        label: i.label,
        max_uses: i.max_uses,
        use_count: i.use_count,
        created_at: i.created_at,
        expires_at: i.expires_at,
        join_url: i.join_url,
    }
}

/// Open invitations.
#[utoipa::path(get, path = "/family/invites", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    responses((status = 200, body = Vec<Invite>)))]
pub async fn list_invites(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>) -> Result<Json<Vec<Invite>>, Problem> {
    let Json(list) = families::list_invites(family_ctx, State(state)).await?;
    Ok(Json(list.into_iter().map(invite).collect()))
}

#[derive(Deserialize, ToSchema)]
pub struct InviteRequest {
    /// `email` (mailed to `email`) or `link` (a code to share).
    pub kind: Option<String>,
    pub email: Option<String>,
    /// `admin` or `member`; `member` when absent.
    pub role: Option<String>,
    /// A link: how many accounts may use it, 1 to 20.
    pub max_uses: Option<i32>,
    /// For the admins only ("fridge QR").
    pub label: Option<String>,
}

/// Invite someone.
#[utoipa::path(post, path = "/family/invites", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    request_body = InviteRequest,
    responses((status = 201, body = Invite), (status = 400, body = crate::v2::problem::ProblemBody)))]
pub async fn create_invite(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, meta: RequestMeta, Json(body): Json<InviteRequest>) -> Result<(StatusCode, Json<Invite>), Problem> {
    let request = families::CreateInviteRequest {
        kind: body.kind.unwrap_or_else(|| "email".into()),
        email: body.email,
        role: v1_role(body.role.as_deref().unwrap_or("member"))?,
        max_uses: body.max_uses,
        label: body.label,
    };
    let (_, Json(made)) = families::create_invite(family_ctx, State(state), meta, Json(request)).await?;
    Ok((StatusCode::CREATED, Json(invite(made))))
}

/// Withdraw an invitation.
#[utoipa::path(delete, path = "/family/invites/{id}", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Withdrawn"), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn delete_invite(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, meta: RequestMeta, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    families::delete_invite(family_ctx, State(state), meta, Path(id)).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct JoinRequest {
    pub code: String,
}

/// Join a household with an invitation's code.
#[utoipa::path(post, path = "/family/join", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body = JoinRequest,
    responses((status = 200, description = "The household joined", body = Family), (status = 400, body = crate::v2::problem::ProblemBody)))]
pub async fn join(auth: AuthUser, State(state): State<AppState>, Json(body): Json<JoinRequest>) -> Result<Json<Family>, Problem> {
    let Json(f) = families::accept_invite(auth, State(state), Json(families::AcceptInviteRequest { code: body.code })).await?;
    Ok(Json(family(f)))
}

/// What the household keeps, in bytes.
#[utoipa::path(get, path = "/family/storage", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = families::BillingStorage)))]
pub async fn storage(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<families::BillingStorage>, Problem> {
    Ok(families::get_family_storage(family_ctx, State(state)).await?)
}

// ── Visibility ───────────────────────────────────────────────────────────────

#[derive(Deserialize, Serialize, ToSchema)]
pub struct Visibility {
    /// `private` (the owner's) or `family` (the household's).
    pub visibility: String,
}

fn check(v: &Visibility) -> Result<(), Problem> {
    if v.visibility == "private" || v.visibility == "family" {
        Ok(())
    } else {
        Err(Problem::bad_request("invalid_body", "visibility is `private` or `family`").field("/visibility", "invalid", v.visibility.clone()))
    }
}

macro_rules! visibility {
    ($name:ident, $path:literal, $doc:literal, $module:ident, $call:ident) => {
        #[doc = $doc]
        #[utoipa::path(put, path = $path, tag = "household", security(("bearer" = [])),
            extensions(("x-permission" = json!("owner"))),
            params(("id" = Uuid, Path)),
            request_body = Visibility,
            responses((status = 200, body = Visibility), (status = 404, description = "Not the caller's", body = crate::v2::problem::ProblemBody)))]
        pub async fn $name(family_ctx: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<Visibility>) -> Result<Json<Visibility>, Problem> {
            check(&body)?;
            let _ = crate::$module::$call(family_ctx, State(state), Path(id), Json(crate::$module::SetVisibilityRequest { visibility: body.visibility.clone() })).await?;
            Ok(Json(body))
        }
    };
}

visibility!(set_track_visibility, "/tracks/{id}/visibility", "Share a track with the household, or keep it to yourself.",
    music, set_track_visibility);
visibility!(set_book_visibility, "/books/{id}/visibility", "Share a book with the household, or keep it to yourself.",
    audiobooks, set_book_visibility);
visibility!(set_podcast_visibility, "/podcasts/{id}/visibility", "Share a podcast with the household, or keep it to yourself.",
    podcasts, set_feed_visibility);
