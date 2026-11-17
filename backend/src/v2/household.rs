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
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::families::{self, FamilyAdmin, FamilyContext};
use crate::v2::account::{Picture, picture};
use crate::v2::me::Me;
use crate::v2::problem::Problem;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
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
    /// The household's photo: a signed link to `GET /images/{id}`, good for one to two weeks.
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

pub(crate) fn family(state: &AppState, f: families::FamilyResponse) -> Family {
    Family {
        id: f.id,
        name: f.name,
        image: crate::v2::images::maybe_expiring(state, f.avatar_object_id),
        created_at: f.created_at,
        me: Membership { user_id: f.my_user_id, role: v2_role(&f.my_role), can_upload: f.my_can_upload, can_generate: f.my_can_generate },
    }
}

/// The caller's household.
#[utoipa::path(get, path = "/family", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Family)))]
pub async fn get_family(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<Family>, Problem> {
    let Json(f) = families::get_family(family_ctx, State(state.clone())).await?;
    Ok(Json(family(&state, f)))
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
    let Json(f) = families::get_family(family_ctx, State(state.clone())).await?;
    Ok(Json(family(&state, f)))
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
    /// Their picture: a signed link to `GET /images/{id}`, good for one to two weeks.
    pub image: Option<String>,
    pub joined_at: String,
}

fn member(state: &AppState, m: db::families::FamilyMember) -> Member {
    Member {
        user_id: m.user_id.to_string(),
        display_name: m.display_name,
        label: m.display_label,
        email: m.email,
        role: v2_role(&m.role),
        active: m.is_active,
        pending: m.pending,
        age_bracket: m.age_bracket,
        can_upload: m.can_upload,
        can_generate: m.can_generate,
        image: crate::v2::images::maybe_expiring(state, m.avatar_object_id),
        joined_at: m.joined_at.to_rfc3339(),
    }
}

/// One person of the caller's household; a household is small, so it is read whole.
async fn find_member(state: &AppState, family_id: Uuid, user_id: Uuid) -> Result<Option<db::families::FamilyMember>, Problem> {
    Ok(db::families::list_members(state.db(), family_id).await?.into_iter().find(|m| m.user_id == user_id))
}

/// v1's member rules answer `NotFound` for someone not in the household, which v1 sends as `401`.
fn member_problem(e: AuthError) -> Problem {
    match e {
        AuthError::NotFound => Problem::not_found(),
        other => other.into(),
    }
}

/// Everyone in the household.
#[utoipa::path(get, path = "/family/members", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Vec<Member>)))]
pub async fn list_members(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<Vec<Member>>, Problem> {
    let members = db::families::list_members(state.db(), family_ctx.family_id).await?;
    Ok(Json(members.into_iter().map(|m| member(&state, m)).collect()))
}

/// Someone of the household as the change feed carries them (kind `member`): enough to
/// name an item's owner. Everyone in the household, and everyone who left it, so an owner
/// id always resolves. The rest of a member is on `GET /family/members`.
#[derive(Serialize, ToSchema)]
pub struct MemberRecord {
    /// The account's id, as `owner_id` on items.
    pub user_id: Uuid,
    /// Absent for an account that was deleted.
    pub display_name: Option<String>,
    /// What the household calls them, when it differs ("Mum").
    pub label: Option<String>,
    /// Their picture: a signed link to `GET /images/{id}`, good for one to two weeks
    /// (`GET /family/members` hands out a fresh one). None for someone who left.
    pub image: Option<String>,
    /// When they left the household; absent while they are in it.
    pub left_at: Option<DateTime<Utc>>,
    /// A household admin blocked them: they cannot sign in. Always `false` for someone who left.
    pub blocked: bool,
}

pub(crate) fn member_record(state: &AppState, row: crate::db::families::MemberRecordRow) -> MemberRecord {
    MemberRecord {
        user_id: row.user_id,
        display_name: row.display_name,
        label: row.label,
        image: crate::v2::images::maybe_expiring(state, row.avatar_object_id),
        left_at: row.left_at,
        blocked: row.blocked,
    }
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
    /// `true` stops the account: it cannot sign in, and every device signed in as it is signed
    /// out at once. `false` lets it sign in again. Their library and membership stay as they are.
    pub blocked: Option<bool>,
    /// Whether the household's admins see the member's listening statistics. An admin may set it
    /// only for an account they manage (one with a restriction, as on v1) or for themselves;
    /// everyone else decides it in their own `/me/settings`.
    pub stats_visible_to_admin: Option<bool>,
}

/// Change a member's role, age bracket, rights or label, or block them.
#[utoipa::path(patch, path = "/family/members/{user_id}", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("user_id" = Uuid, Path)),
    request_body = MemberPatch,
    responses((status = 204, description = "Changed"),
        (status = 400, description = "`cannot_block_self`; an age bracket, role or right that is not one, or one's own rights", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`cannot_block_instance_admin`: the server's administrators are not blocked from a household; `member_not_restricted`: an adult decides who sees their statistics", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not a member", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_member(
    FamilyAdmin(family_ctx): FamilyAdmin,
    State(state): State<AppState>,
    meta: RequestMeta,
    Path(user_id): Path<Uuid>,
    Json(body): Json<MemberPatch>,
) -> Result<StatusCode, Problem> {
    let role = body.role.as_deref().map(v1_role).transpose()?;
    find_member(&state, family_ctx.family_id, user_id).await?.ok_or_else(Problem::not_found)?;
    if body.blocked == Some(true) {
        if user_id == family_ctx.user_id {
            return Err(Problem::bad_request("cannot_block_self", "you cannot block your own account").field("/blocked", "invalid", "not yourself"));
        }
        let account = db::users::find_by_id(state.db(), user_id).await?.ok_or_else(Problem::not_found)?;
        if account.role == "admin" {
            return Err(Problem::new(StatusCode::FORBIDDEN, "cannot_block_instance_admin")
                .detail("an administrator of this server cannot be blocked from a household"));
        }
    }
    if body.stats_visible_to_admin.is_some()
        && user_id != family_ctx.user_id
        && !db::stats::is_restricted_member(state.db(), family_ctx.family_id, user_id).await?
    {
        return Err(Problem::new(StatusCode::FORBIDDEN, "member_not_restricted")
            .detail("only an account you manage: a member decides who sees their statistics"));
    }
    let changes_member = role.is_some() || body.age_bracket.is_some() || body.can_upload.is_some() || body.can_generate.is_some() || body.label.is_some();
    if changes_member {
        let request = families::UpdateMemberRequest {
            role,
            age_bracket: body.age_bracket,
            can_upload: body.can_upload,
            can_generate: body.can_generate,
            display_label: body.label,
        };
        families::update_member(family_ctx.clone(), State(state.clone()), meta.clone(), Path(user_id), Json(request)).await.map_err(member_problem)?;
    }
    match body.blocked {
        Some(true) => families::block(&state, user_id).await?,
        Some(false) => db::users::set_active(state.db(), user_id, true).await?,
        None => {}
    }
    if let Some(shared) = body.stats_visible_to_admin {
        db::stats::set_stats_visibility(state.db(), family_ctx.family_id, user_id, if shared { "family_admin" } else { "private" }).await?;
    }
    if let Some(blocked) = body.blocked {
        crate::auth::audit::record(&state, &meta, Some(user_id), Some(family_ctx.user_id), "family.member_changed", serde_json::json!({ "blocked": blocked })).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Take a member out of the household, or leave it: anyone may remove themselves, a household
/// admin anyone. The last admin stays while others are left (`409 last_admin`): make someone
/// else an admin first. Someone who leaves keeps their library in a household of their own; what
/// they shared becomes private again. A member made with `POST /family/members` who never
/// claimed the account is deleted with it. On a server with one household nobody can leave it
/// (`409 one_household`): block the member or delete the account instead.
#[utoipa::path(delete, path = "/family/members/{user_id}", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("family:admin"))),
    params(("user_id" = Uuid, Path)),
    responses((status = 204, description = "Removed, or left"),
        (status = 403, description = "Someone else, and the caller is not a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not a member", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`last_admin`, `one_household`", body = crate::v2::problem::ProblemBody)))]
pub async fn remove_member(family_ctx: FamilyContext, State(state): State<AppState>, meta: RequestMeta, Path(user_id): Path<Uuid>) -> Result<StatusCode, Problem> {
    match families::leave_or_remove(&family_ctx, &state, &meta, user_id).await.map_err(member_problem)? {
        families::Removal::Done => Ok(StatusCode::NO_CONTENT),
        families::Removal::LastAdmin => Err(Problem::new(StatusCode::CONFLICT, "last_admin")
            .detail("the household's last admin stays while others are in it: make someone else an admin first")),
        families::Removal::OneFamily => Err(Problem::new(StatusCode::CONFLICT, "one_household")
            .detail("this server has one household: block the member or delete the account instead")),
    }
}

// ── Pictures ─────────────────────────────────────────────────────────────────

/// The household's photo: `{upload_id}` of an image put with `POST /uploads`, or the image itself
/// as the body (`image/jpeg`, `image/png`, `image/webp`…, at most 5 MB). A new photo gets a new
/// link.
#[utoipa::path(put, path = "/family/image", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    request_body(content(
        (crate::v2::account::ImageRequest = "application/json"),
        (crate::v2::account::ImageBytes = "image/*"))),
    responses(
        (status = 200, body = Family),
        (status = 400, description = "`not_an_image`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "No such upload in this household", body = crate::v2::problem::ProblemBody),
        (status = 413, description = "`too_large`: over 5 MB", body = crate::v2::problem::ProblemBody),
        (status = 415, description = "`unsupported_media_type`: neither JSON nor an image", body = crate::v2::problem::ProblemBody)))]
pub async fn put_family_image(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Result<Json<Family>, Problem> {
    match picture(&state, &family_ctx, &headers, body).await? {
        Picture::Stored(object) => db::families::set_avatar(state.db(), family_ctx.family_id, Some(object)).await?,
        Picture::Bytes { bytes, content_type, ext } => families::store_avatar(&state, family_ctx.family_id, bytes, &content_type, ext).await?,
    }
    let Json(f) = families::get_family(family_ctx, State(state.clone())).await?;
    Ok(Json(family(&state, f)))
}

/// Takes the household's photo away.
#[utoipa::path(delete, path = "/family/image", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    responses((status = 204, description = "Removed, or there was none")))]
pub async fn delete_family_image(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>) -> Result<StatusCode, Problem> {
    db::families::set_avatar(state.db(), family_ctx.family_id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Someone the caller may give a picture: themselves, or anyone of the household for an admin.
async fn pictured(state: &AppState, family_ctx: &FamilyContext, user_id: Uuid) -> Result<(), Problem> {
    find_member(state, family_ctx.family_id, user_id).await?.ok_or_else(Problem::not_found)?;
    if user_id != family_ctx.user_id && !family_ctx.is_family_admin() {
        return Err(Problem::forbidden().detail("only a household admin sets someone else's picture"));
    }
    Ok(())
}

/// A member's picture, set by themselves or, for someone who cannot (a child, a grandparent), by a
/// household admin: `{upload_id}` of an image put with `POST /uploads`, or the image itself as the
/// body (at most 5 MB). The same picture as `PUT /me/image`.
#[utoipa::path(put, path = "/family/members/{user_id}/image", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("user:write"))),
    params(("user_id" = Uuid, Path)),
    request_body(content(
        (crate::v2::account::ImageRequest = "application/json"),
        (crate::v2::account::ImageBytes = "image/*"))),
    responses(
        (status = 200, body = Member),
        (status = 400, description = "`not_an_image`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Someone else's, and the caller is not a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not a member, or no such upload in this household", body = crate::v2::problem::ProblemBody),
        (status = 413, description = "`too_large`: over 5 MB", body = crate::v2::problem::ProblemBody),
        (status = 415, description = "`unsupported_media_type`: neither JSON nor an image", body = crate::v2::problem::ProblemBody)))]
pub async fn put_member_image(
    family_ctx: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<Member>, Problem> {
    pictured(&state, &family_ctx, user_id).await?;
    match picture(&state, &family_ctx, &headers, body).await? {
        Picture::Stored(object) => db::users::set_avatar(state.db(), user_id, Some(object)).await?,
        Picture::Bytes { bytes, content_type, ext } => crate::users::store_avatar(&state, user_id, bytes, &content_type, ext).await?,
    }
    let m = find_member(&state, family_ctx.family_id, user_id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(member(&state, m)))
}

/// Takes a member's picture away: themselves, or a household admin.
#[utoipa::path(delete, path = "/family/members/{user_id}/image", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("user:write"))),
    params(("user_id" = Uuid, Path)),
    responses((status = 204, description = "Removed, or there was none"),
        (status = 403, description = "Someone else's, and the caller is not a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not a member", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_member_image(family_ctx: FamilyContext, State(state): State<AppState>, Path(user_id): Path<Uuid>) -> Result<StatusCode, Problem> {
    pictured(&state, &family_ctx, user_id).await?;
    db::users::set_avatar(state.db(), user_id, None).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Members made by an admin ─────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct NewMember {
    pub display_name: String,
    /// What they will sign in with, shaped like an email address (`lena@family.example`); no mail
    /// is ever sent to it, so it need not be a real mailbox.
    pub login_email: String,
    /// What the household calls them ("Grandma").
    pub label: Option<String>,
    /// `adult`, `teen` or `child`; `adult` when absent. A child may not upload or generate.
    pub age_bracket: Option<String>,
    pub can_upload: Option<bool>,
    pub can_generate: Option<bool>,
}

#[derive(Serialize, ToSchema)]
pub struct ProvisionedMember {
    /// `pending` until they claim the account.
    pub member: Member,
    /// The `claim` invitation: show its code (or `join_url` as a QR code) to the person, who
    /// sets a password with `POST /auth/claim`. `POST /family/invites/{id}/regenerate` makes a
    /// new code.
    pub invite: Invite,
}

/// An account for someone without a mailbox of their own, a child or a grandparent: made in the
/// household, `pending` until they claim it with the code and set their own password. Send an
/// `Idempotency-Key` to retry safely.
#[utoipa::path(post, path = "/family/members", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    request_body = NewMember,
    responses(
        (status = 201, body = ProvisionedMember),
        (status = 400, description = "`invalid_display_name`, `invalid_email`, `invalid_body` (an age bracket that is not one, or rights a child may not have)", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`email_taken`: an account signs in with that already", body = crate::v2::problem::ProblemBody)))]
pub async fn create_member(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, Json(body): Json<NewMember>) -> Result<(StatusCode, Json<ProvisionedMember>), Problem> {
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(Problem::bad_request("invalid_display_name", "a name is required").field("/display_name", "required", "not blank"));
    }
    let login_email = body.login_email.trim().to_lowercase();
    if login_email.is_empty() || !login_email.contains('@') {
        return Err(Problem::bad_request("invalid_email", "the login looks like an email address, e.g. name@family.example")
            .field("/login_email", "invalid", "not an email address"));
    }
    let (bracket, can_upload, can_generate) =
        families::resolve_member_permissions(body.age_bracket.as_deref(), body.can_upload, body.can_generate, "adult", true, true).map_err(|e| match e {
            AuthError::BadRequest(m) => Problem::bad_request("invalid_body", m.clone()).field("/age_bracket", "invalid", m),
            other => other.into(),
        })?;
    if db::users::find_by_email(state.db(), &login_email).await?.is_some() {
        return Err(Problem::new(StatusCode::CONFLICT, "email_taken").detail("an account signs in with that already"));
    }
    let (made, invite_row) = families::provision(&state, &family_ctx, &display_name, &login_email, body.label.as_deref()).await?;
    let made = if (bracket.as_str(), can_upload, can_generate) != (made.age_bracket.as_str(), made.can_upload, made.can_generate) {
        db::families::set_member_permissions(state.db(), family_ctx.family_id, made.user_id, &bracket, can_upload, can_generate).await?;
        find_member(&state, family_ctx.family_id, made.user_id).await?.ok_or_else(Problem::not_found)?
    } else {
        made
    };
    Ok((
        StatusCode::CREATED,
        Json(ProvisionedMember { member: member(&state, made), invite: invite(families::invite_to_response(&state, invite_row)) }),
    ))
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
    /// A `claim` invitation: the pending member it lets in.
    pub member_user_id: Option<String>,
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
        member_user_id: i.member_user_id,
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

/// A new code for an invitation that is not used up, and a new expiry: the old code stops
/// working. An `email` invitation is mailed again.
#[utoipa::path(post, path = "/family/invites/{id}/regenerate", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Invite),
        (status = 404, body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`invite_used`: every use is spent", body = crate::v2::problem::ProblemBody)))]
pub async fn regenerate_invite(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Invite>, Problem> {
    let renewed = families::regenerate(&state, &family_ctx, id)
        .await
        .map_err(member_problem)?
        .ok_or_else(|| Problem::new(StatusCode::CONFLICT, "invite_used").detail("every use of this invitation is spent"))?;
    Ok(Json(invite(families::invite_to_response(&state, renewed))))
}

#[derive(Deserialize, ToSchema)]
pub struct CodeRequest {
    pub code: String,
}

#[derive(Serialize, ToSchema)]
pub struct JoinPreview {
    /// `invite` (sign up or sign in, then join with `POST /auth/register` or `POST /family/join`)
    /// or `claim` (an account made for the holder: `POST /auth/claim`).
    pub kind: String,
    pub family: JoinFamily,
    /// Who made the code.
    pub inviter_name: Option<String>,
    /// What joining makes the holder: `admin` or `member`.
    pub role: String,
    pub member_count: i64,
    pub expires_at: DateTime<Utc>,
    pub uses_left: i32,
    /// A `claim` code: the account it is for.
    pub claim: Option<families::ClaimPreview>,
}

#[derive(Serialize, ToSchema)]
pub struct JoinFamily {
    pub name: String,
    /// The household's photo: a signed link to `GET /images/{id}`, good for one to two weeks.
    pub image: Option<String>,
}

fn dead_code(state: families::Code) -> Problem {
    match state {
        families::Code::Exhausted => Problem::new(StatusCode::GONE, "invite_used").detail("this code has been used"),
        families::Code::Expired => Problem::new(StatusCode::GONE, "invite_expired").detail("this code has expired: ask a household admin for a new one"),
        _ => Problem::not_found(),
    }
}

/// What a join or claim code leads to, before signing in: the household, who invited, and for a
/// claim code the account. Only a valid code tells anything; a used or expired one says just that
/// (`410`), an unknown one is `404`. The code goes in the body, never in the address.
#[utoipa::path(post, path = "/family/join/preview", tag = "household",
    extensions(("x-permission" = json!("public"))),
    request_body = CodeRequest,
    responses((status = 200, body = JoinPreview),
        (status = 404, description = "Unknown code", body = crate::v2::problem::ProblemBody),
        (status = 410, description = "`invite_used`, `invite_expired`", body = crate::v2::problem::ProblemBody)))]
pub async fn join_preview(State(state): State<AppState>, Json(body): Json<CodeRequest>) -> Result<Json<JoinPreview>, Problem> {
    let valid = match families::look_up_code(&state, &body.code).await? {
        families::Code::Valid(valid) => valid,
        other => return Err(dead_code(other)),
    };
    let families::ValidCode { invite, family, member_count, inviter_name, claim } = *valid;
    Ok(Json(JoinPreview {
        kind: if invite.kind == "claim" { "claim" } else { "invite" }.to_string(),
        family: JoinFamily { name: family.name, image: crate::v2::images::maybe_expiring(&state, family.avatar_object_id) },
        inviter_name,
        role: v2_role(&invite.role),
        member_count,
        expires_at: invite.expires_at,
        uses_left: invite.max_uses - invite.use_count,
        claim,
    }))
}

#[derive(Deserialize, ToSchema)]
pub struct ClaimRequest {
    /// The claim code a household admin was given with `POST /family/members`.
    pub code: String,
    /// Twelve characters or more; a few words are fine.
    pub password: String,
}

/// Claims an account a household admin made (`POST /family/members`): sets its first password.
/// Answers the account; signing in is `POST /auth/token` with the login the preview shows. A code
/// works once.
#[utoipa::path(post, path = "/auth/claim", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body = ClaimRequest,
    responses(
        (status = 201, description = "The account; sign in with `POST /auth/token`", body = Me),
        (status = 400, description = "`weak_password`, `not_a_claim_code`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown code", body = crate::v2::problem::ProblemBody),
        (status = 410, description = "`invite_used` (claimed already), `invite_expired`", body = crate::v2::problem::ProblemBody)))]
pub async fn claim(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<ClaimRequest>) -> Result<(StatusCode, Json<Me>), Problem> {
    crate::v2::account::check_password(&body.password, "/password")?;
    let user = match families::claim_account(&state, &body.code, &body.password).await? {
        Ok(user) => user,
        Err(families::ClaimRefusal::Unknown) => return Err(Problem::not_found()),
        Err(families::ClaimRefusal::NotAClaim) => {
            return Err(Problem::bad_request("not_a_claim_code", "this code invites to a household; sign up or sign in, then join")
                .field("/code", "invalid", "not a claim code"));
        }
        Err(families::ClaimRefusal::Used | families::ClaimRefusal::AlreadyClaimed) => return Err(dead_code(families::Code::Exhausted)),
        Err(families::ClaimRefusal::Expired) => return Err(dead_code(families::Code::Expired)),
    };
    crate::auth::audit::record(&state, &meta, Some(user.id), None, "account.claimed", serde_json::json!({})).await;
    Ok((StatusCode::CREATED, Json(crate::v2::me::build_for(&state, user.id).await?)))
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
    let Json(f) = families::accept_invite(auth, State(state.clone()), Json(families::AcceptInviteRequest { code: body.code })).await?;
    Ok(Json(family(&state, f)))
}

/// What the household keeps, in bytes.
#[utoipa::path(get, path = "/family/storage", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = families::BillingStorage)))]
pub async fn storage(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<families::BillingStorage>, Problem> {
    Ok(families::get_family_storage(family_ctx, State(state)).await?)
}

// ── Settings ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct FamilySettings {
    /// Measuring the household's music (tempo, loudness…) for smart playlists: whether it is
    /// on, and how far it has got.
    pub audio_analysis: families::AudioAnalysisResponse,
    /// Where identifying an audiobook looks.
    pub book_source: families::book_source::BookSourceResponse,
}

#[derive(Deserialize, ToSchema)]
pub struct FamilySettingsPatch {
    pub audio_analysis: Option<AudioAnalysisPatch>,
    pub book_source: Option<BookSourcePatch>,
}

#[derive(Deserialize, ToSchema)]
pub struct AudioAnalysisPatch {
    /// On queues what is not measured yet; off stops new work and keeps every measurement.
    pub enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct BookSourcePatch {
    /// The household's choice; unchanged when absent.
    pub source: Option<crate::metadata::books::BookSource>,
    /// A Google Books key of the household's own; `null` removes it, absent keeps it. It is never
    /// sent back.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub google_books_api_key: Option<Option<String>>,
}

async fn settings(state: &AppState, family_ctx: &FamilyContext) -> Result<FamilySettings, Problem> {
    Ok(FamilySettings {
        audio_analysis: families::audio_analysis_status(state, family_ctx).await?,
        book_source: families::book_source::status(state, family_ctx).await?,
    })
}

/// The household's settings. Anyone in it may read them; the Google key itself is never sent.
#[utoipa::path(get, path = "/family/settings", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = FamilySettings)))]
pub async fn get_settings(family_ctx: FamilyContext, State(state): State<AppState>) -> Result<Json<FamilySettings>, Problem> {
    Ok(Json(settings(&state, &family_ctx).await?))
}

/// Change the household's settings; what the body leaves out stays. A book source of the
/// household's own needs a server that offers the choice (`501 feature_unavailable`,
/// `book_source_choice`), and Google needs a key, the household's or the server's.
#[utoipa::path(patch, path = "/family/settings", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    request_body = FamilySettingsPatch,
    responses((status = 200, body = FamilySettings),
        (status = 400, description = "`invalid_key`, `key_required`", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable`: this server does not let households choose a book source", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_settings(FamilyAdmin(family_ctx): FamilyAdmin, State(state): State<AppState>, Json(body): Json<FamilySettingsPatch>) -> Result<Json<FamilySettings>, Problem> {
    // The book source first: it is the part that may be refused, and then nothing has changed.
    if let Some(choice) = body.book_source {
        let source = match choice.source {
            Some(source) => source,
            None => families::book_source::status(&state, &family_ctx).await?.source,
        };
        match families::book_source::choose(&state, &family_ctx, source, choice.google_books_api_key).await? {
            Ok(_) => {}
            Err(families::book_source::Refusal::NotOffered) => {
                return Err(crate::v2::account::unavailable("book_source_choice", "this server does not let households choose where books are identified"));
            }
            Err(families::book_source::Refusal::NotAKey) => {
                return Err(Problem::bad_request("invalid_key", "that is not a Google API key").field("/book_source/google_books_api_key", "invalid", ""));
            }
            Err(families::book_source::Refusal::KeyNeeded) => {
                return Err(Problem::bad_request("key_required", "Google Books needs an API key: without one it turns this server away")
                    .field("/book_source/google_books_api_key", "required", ""));
            }
        }
    }
    if let Some(analysis) = body.audio_analysis {
        families::switch_audio_analysis(&state, &family_ctx, analysis.enabled).await?;
    }
    Ok(Json(settings(&state, &family_ctx).await?))
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
