// SPDX-License-Identifier: AGPL-3.0-or-later
/// Families module — user groups, membership roles, invites.
///
/// A family owns a shared library; its members each keep their own playback
/// memory. Family admins manage membership and (from Phase 2) per-member
/// access to individual media items.
pub mod admin;
pub mod context;

pub use context::FamilyContext;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use axum::Router;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, Json, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use serde::{Deserialize, Serialize};
use std::path::Path as StdPath;
use uuid::Uuid;

use argon2::password_hash::rand_core::{OsRng, RngCore};
use argon2::password_hash::SaltString;
use argon2::{Argon2, PasswordHasher};

/// How long a fresh invite stays redeemable — one TTL per kind (see
/// migration 0042 / docs/family-join-qr-plan.md D5).
const INVITE_TTL_DAYS: i64 = 14;
const LINK_INVITE_TTL_DAYS: i64 = 7;
const CLAIM_INVITE_TTL_DAYS: i64 = 30;

/// Above `MAX_AVATAR_BYTES` (`crate::users`'s constant of the same name — not shared, since a
/// two-constant duplication isn't worth a cross-module dependency for) so a body under that
/// check's cap is never rejected purely by multipart-framing overhead.
const FAMILY_AVATAR_BODY_LIMIT_BYTES: usize = 6 * 1024 * 1024;
const MAX_FAMILY_AVATAR_BYTES: usize = 5 * 1024 * 1024;

/// Same limits again for `POST /family/members/{id}/avatar` — an admin setting a *member's*
/// personal photo on their behalf, not the shared family one above.
/// Free text on a report. Long enough to describe what was wrong, short
/// enough that the column is not an essay store.
const MAX_REPORT_NOTE_CHARS: usize = 1000;
const MEMBER_AVATAR_BODY_LIMIT_BYTES: usize = 6 * 1024 * 1024;
const MAX_MEMBER_AVATAR_BYTES: usize = 5 * 1024 * 1024;

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct FamilyResponse {
    pub id: String,
    pub name: String,
    /// The calling user's role in this family.
    pub my_role: String,
    /// The calling user's own id — lets clients find "myself" in `members`
    /// without threading a separate profile fetch through just for that
    /// (e.g. a "Leave family" button targeting `DELETE /members/{my_user_id}`).
    pub my_user_id: String,
    /// The caller's own permissions, so a client need not locate itself in
    /// `members` to decide whether to show an upload affordance.
    pub my_can_upload: bool,
    pub my_can_generate: bool,
    /// `/api/v1/family/avatar` when the family has set a photo, else `null`.
    /// family_admin-managed, like the name — see `families::upload_avatar`.
    pub avatar_url: Option<String>,
    pub members: Vec<MemberResponse>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct MemberResponse {
    pub user_id: String,
    pub email: String,
    pub display_name: String,
    /// Optional in-family label ("Dad"); falls back to display_name in UIs.
    pub display_label: Option<String>,
    pub role: String,
    pub is_active: bool,
    /// True for a provisioned account nobody has claimed yet.
    pub pending: bool,
    /// `/api/v1/users/{user_id}/avatar` when the member has set one, else
    /// `null`. A relative path, same convention as every other cover URL in
    /// this API (e.g. `audiobooks::book_to_response`'s `cover_url`).
    pub avatar_url: Option<String>,
    pub joined_at: String,
    /// `adult` | `teen` | `child`. Admin-set; see docs/family-permissions-plan.md.
    pub age_bracket: String,
    pub can_upload: bool,
    pub can_generate: bool,
}

#[derive(Deserialize)]
pub struct CreateReportRequest {
    /// `audiobook` | `podcast` | `music`.
    pub media_kind: String,
    pub item_id: Uuid,
    /// `offensive` | `inaccurate` | `not_for_children` | `other`.
    pub reason: String,
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct ContentReportResponse {
    pub id: String,
    /// `null` when the reporter has since been deleted — the report stands.
    pub reporter_name: Option<String>,
    pub media_kind: String,
    pub item_id: String,
    pub reason: String,
    pub note: Option<String>,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct UpdateFamilyRequest {
    pub name: String,
}

#[derive(Deserialize)]
pub struct UpdateMemberRequest {
    pub role: Option<String>,
    /// `adult` | `teen` | `child`.
    pub age_bracket: Option<String>,
    pub can_upload: Option<bool>,
    pub can_generate: Option<bool>,
    /// Present-but-null clears the label.
    #[serde(default, deserialize_with = "double_option")]
    pub display_label: Option<Option<String>>,
}

#[derive(Deserialize)]
pub struct CreateInviteRequest {
    /// `email` (default) or `link`. `claim` invites are created only via
    /// `POST /family/members/provision`, never directly.
    #[serde(default = "default_invite_kind")]
    pub kind: String,
    /// Required when `kind = "email"`; ignored otherwise.
    pub email: Option<String>,
    #[serde(default = "default_member_role")]
    pub role: String,
    /// `link` kind only: how many separate accounts may redeem this code
    /// (1–20, default 1). A fridge QR wants more than one.
    pub max_uses: Option<i32>,
    /// Admin-facing note ("fridge QR"); never shown to the invitee.
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct InviteResponse {
    pub id: String,
    /// `email` | `link` | `claim`.
    pub kind: String,
    pub email: Option<String>,
    /// The bearer code the invitee redeems. Deliver it out-of-band.
    pub code: String,
    pub role: String,
    pub label: Option<String>,
    pub max_uses: i32,
    pub use_count: i32,
    pub created_at: String,
    pub expires_at: String,
    /// `{base_url}/join/{code}` when `server.base_url` is configured, else
    /// `null` — clients fall back to building it from their connected origin.
    pub join_url: Option<String>,
    /// `kind: "claim"` only — the provisioned account this code activates, so
    /// a client can match a pending `MemberResponse` back to its invite (to
    /// re-show the code / offer "regenerate") without a second lookup.
    pub member_user_id: Option<String>,
}

#[derive(Deserialize)]
pub struct AcceptInviteRequest {
    pub code: String,
}

#[derive(Deserialize)]
pub struct ProvisionMemberRequest {
    pub display_name: String,
    /// An email-shaped login identifier the admin picks for a member with no
    /// real mailbox (e.g. `lena@maraz.family`). Stored in `users.email` and
    /// used to sign in; no mail is ever sent to it.
    pub login_email: String,
    #[serde(default)]
    pub display_label: Option<String>,
}

#[derive(Serialize)]
pub struct ProvisionMemberResponse {
    pub member: MemberResponse,
    /// A `claim` invite for the new account — show its QR/link/code so the
    /// member can set their own password.
    pub invite: InviteResponse,
}

#[derive(Serialize)]
pub struct JoinPreviewResponse {
    /// `valid` | `expired` | `exhausted`. Unknown codes are a plain 404
    /// instead — see decision D6 on what dead codes may reveal.
    pub status: &'static str,
    pub kind: Option<String>,
    pub family_name: Option<String>,
    pub inviter_name: Option<String>,
    pub role: Option<String>,
    pub member_count: Option<i64>,
    pub expires_at: Option<String>,
    pub uses_left: Option<i32>,
    /// `kind = "claim"` only: who this code signs the claimant in as.
    pub claim: Option<ClaimPreview>,
}

#[derive(Serialize)]
pub struct ClaimPreview {
    pub display_name: String,
    pub login_email: String,
}

#[derive(Deserialize)]
pub struct ClaimInviteRequest {
    pub password: String,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_kind: Option<String>,
}

fn default_member_role() -> String {
    "member".to_string()
}

fn default_invite_kind() -> String {
    "email".to_string()
}

/// Distinguish "field absent" from "field explicitly null" so a PATCH-style
/// body can clear `display_label` without clearing it on every request.
fn double_option<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: serde::Deserialize<'de>,
{
    serde::Deserialize::deserialize(deserializer).map(Some)
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(get_family))
        .route("/", put(update_family))
        .route("/storage", get(get_family_storage))
        .route(
            "/avatar",
            post(upload_family_avatar).layer(DefaultBodyLimit::max(FAMILY_AVATAR_BODY_LIMIT_BYTES)),
        )
        .route("/avatar", delete(delete_family_avatar))
        .route("/avatar", get(get_family_avatar))
        .route("/settings/audio-analysis", get(get_audio_analysis))
        .route("/settings/audio-analysis", put(set_audio_analysis))
        .route("/members", get(list_members))
        .route("/members/provision", post(provision_member))
        .route("/members/{user_id}", put(update_member))
        .route("/members/{user_id}", delete(remove_member))
        .route("/members/{user_id}/block", post(block_member))
        .route("/members/{user_id}/unblock", post(unblock_member))
        .route(
            "/members/{user_id}/avatar",
            post(upload_member_avatar).layer(DefaultBodyLimit::max(MEMBER_AVATAR_BODY_LIMIT_BYTES)),
        )
        .route("/members/{user_id}/avatar", delete(delete_member_avatar))
        .route("/invites", get(list_invites))
        .route("/invites", post(create_invite))
        .route("/invites/{id}", delete(delete_invite))
        .route("/invites/{id}/regenerate", post(regenerate_invite))
        .route("/invites/accept", post(accept_invite))
        // Parental controls over shared content
        .route("/members/{user_id}/access", get(get_member_access))
        .route("/members/{user_id}/policy", put(set_member_policy))
        .route("/members/{user_id}/grants", put(replace_member_grants))
        .route("/content/{kind}/{item_id}/audience", get(content_audience))
        .route("/content/{kind}/{item_id}/audience", put(set_content_audience))
        .route("/content/{kind}/{item_id}/request-access", post(request_content_access))
        // Content reports — required in-app by Play for AI output and for
        // content shared between accounts; see audio2-android-book/PLAY_COMPLIANCE.md.
        .route("/content-reports", post(create_content_report))
        .route("/content-reports", get(list_content_reports))
        .route("/content-reports/{id}/resolve", post(resolve_content_report))
}

/// Public join router — nested at `/api/v1/join`, no auth required. The
/// bearer code is the credential; see decision D6 on what dead codes reveal.
pub fn join_router(limits: &crate::http::rate_limit::Limiters) -> Router<AppState> {
    Router::new()
        .route("/{code}", limits.join.apply(get(join_preview)))
        .route("/{code}/claim", limits.join.apply(post(claim_provisioned_account)))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/family — the caller's family, members, and own role.
async fn get_family(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<FamilyResponse>, AuthError> {
    let record = db::families::find(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(FamilyResponse {
        id: record.id.to_string(),
        name: record.name,
        my_role: family.family_role.clone(),
        my_user_id: family.user_id.to_string(),
        my_can_upload: family.effective_can_upload(),
        my_can_generate: family.effective_can_generate(),
        avatar_url: record.avatar_object_id.map(|_| "/api/v1/family/avatar".to_string()),
        members: members.into_iter().map(member_to_response).collect(),
        created_at: record.created_at.to_rfc3339(),
    }))
}

/// PUT /api/v1/family — rename the family (family_admin only).
async fn update_family(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<UpdateFamilyRequest>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;

    let name = body.name.trim();
    if name.is_empty() {
        return Err(AuthError::BadRequest("family name is required".into()));
    }

    db::families::rename(state.db(), family.family_id, name)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/family/avatar — set the family's own photo (family_admin only, same gate as
/// renaming). Distinct from a member's personal avatar (`crate::users::upload_avatar`); this one
/// is shared, shown at the top of every member's Family tab.
async fn upload_family_avatar(
    family: FamilyContext,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;

    // Field fully drained inside the loop body — see `crate::users::upload_avatar`'s own comment
    // on why a `Field<'_>` can't be held past a `break` here.
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
    if bytes.len() > MAX_FAMILY_AVATAR_BYTES {
        return Err(AuthError::BadRequest(format!(
            "avatar must be under {}MB",
            MAX_FAMILY_AVATAR_BYTES / 1_000_000
        )));
    }

    let ext = family_avatar_extension(&file_name).unwrap_or("jpg");
    // Deliberately *outside* the `f/{family_id}/…` prefix (`storage::family_key`) despite this
    // being family-owned content: every object under that prefix must also be registered in
    // `db::storage_usage`'s per-kind `*_REFS` lists or it reads as orphaned/unsized — not worth wiring
    // a small profile photo into that actively-changing accounting for.
    let object_key = format!("fam/{}/avatar.{ext}", family.family_id);
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

    db::families::set_avatar(&mut *tx, family.family_id, Some(media_id))
        .await
        .map_err(AuthError::Internal)?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/family/avatar — remove the family photo (family_admin only). Stored object is
/// left in place, same as every other cover-replace path in this API.
async fn delete_family_avatar(family: FamilyContext, State(state): State<AppState>) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    db::families::set_avatar(state.db(), family.family_id, None)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/family/avatar — any family member (not just admins) can view it.
async fn get_family_avatar(family: FamilyContext, State(state): State<AppState>) -> Result<Response, AuthError> {
    let (object_key, content_type) = db::families::find_avatar(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;
    Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response())
}

fn family_avatar_extension(file_name: &str) -> Option<&str> {
    StdPath::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim_matches('.'))
        .filter(|ext| !ext.is_empty())
}

/// GET /api/v1/family/members
// ── Library audio analysis opt-in ─────────────────────────────────────────
//
// Measuring a library means fetching every object back out of storage and
// decoding it. It happens because someone asked, not by default. See
// docs/music-signals-and-smart-playlists-plan.md §3.4 and migration 0073.

#[derive(Serialize)]
pub struct AudioAnalysisResponse {
    pub enabled: bool,
    pub enabled_at: Option<String>,
    /// Tracks measured at the current extractor version.
    pub measured: i64,
    /// Tracks in this family's library worth measuring.
    pub total: i64,
    /// True while there is queued work — what a client shows a bar for.
    pub running: bool,
}

#[derive(Deserialize)]
pub struct SetAudioAnalysisRequest {
    pub enabled: bool,
}

/// GET /family/settings/audio-analysis
async fn get_audio_analysis(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<AudioAnalysisResponse>, AuthError> {
    audio_analysis_status(&state, &family).await.map(Json)
}

/// PUT /family/settings/audio-analysis
///
/// Enabling queues the family's unmeasured library, most-played first.
/// Disabling stops new work and **keeps every measurement already taken** —
/// deleting them would mean re-fetching the library if the user changed their
/// mind, which is the one expensive thing in this feature.
async fn set_audio_analysis(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<SetAudioAnalysisRequest>,
) -> Result<Json<AudioAnalysisResponse>, AuthError> {
    // A family-wide switch that spends real work on shared content: admins
    // only, like every other family-level setting.
    family.require_family_admin()?;

    db::music::set_audio_analysis_optin(
        state.db(),
        family.family_id,
        body.enabled,
        family.user_id,
    )
    .await
    .map_err(AuthError::Internal)?;

    if body.enabled {
        let queued = db::music::enqueue_family_analysis(
            state.db(),
            family.family_id,
            crate::music::analysis::ANALYSIS_VERSION,
            ANALYSIS_ENQUEUE_BATCH,
        )
        .await
        .map_err(AuthError::Internal)?;
        tracing::info!(family_id = %family.family_id, queued, "library analysis requested");
    }

    audio_analysis_status(&state, &family).await.map(Json)
}

/// One batch of the library per request. The worker's periodic sweep continues
/// from here, so a library larger than this does not need a bigger burst — it
/// needs the next tick.
const ANALYSIS_ENQUEUE_BATCH: i64 = 500;

async fn audio_analysis_status(
    state: &AppState,
    family: &FamilyContext,
) -> Result<AudioAnalysisResponse, AuthError> {
    let (enabled, enabled_at) = db::music::audio_analysis_optin(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let (measured, total) = db::music::audio_analysis_progress(
        state.db(),
        family.family_id,
        crate::music::analysis::ANALYSIS_VERSION,
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(AudioAnalysisResponse {
        enabled,
        enabled_at: enabled_at.map(|t| t.to_rfc3339()),
        measured,
        total,
        running: enabled && measured < total,
    })
}

async fn list_members(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<MemberResponse>>, AuthError> {
    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(members.into_iter().map(member_to_response).collect()))
}

/// `POST /family/content-reports` — any member, including one who may not
/// upload. Reporting is the counterweight to a family library nobody outside
/// the household moderates, so it must not be a privilege.
async fn create_content_report(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateReportRequest>,
) -> Result<StatusCode, AuthError> {
    if !db::access::is_valid_kind(&body.media_kind) {
        return Err(AuthError::BadRequest(
            "media_kind must be 'audiobook', 'podcast', or 'music'".into(),
        ));
    }
    if !db::access::is_valid_report_reason(&body.reason) {
        return Err(AuthError::BadRequest(
            "reason must be 'offensive', 'inaccurate', 'not_for_children' or 'other'".into(),
        ));
    }
    let note = body
        .note
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(|n| n.chars().take(MAX_REPORT_NOTE_CHARS).collect::<String>());

    db::access::insert_report(
        state.db(),
        family.family_id,
        family.user_id,
        &body.media_kind,
        body.item_id,
        &body.reason,
        note.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(StatusCode::CREATED)
}

/// `GET /family/content-reports` — the admin's moderation queue.
async fn list_content_reports(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<ContentReportResponse>>, AuthError> {
    family.require_family_admin()?;
    let reports = db::access::list_open_reports(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(
        reports
            .into_iter()
            .map(|r| ContentReportResponse {
                id: r.id.to_string(),
                reporter_name: r.reporter_name,
                media_kind: r.media_kind,
                item_id: r.item_id.to_string(),
                reason: r.reason,
                note: r.note,
                created_at: r.created_at.to_rfc3339(),
            })
            .collect(),
    ))
}

async fn resolve_content_report(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(report_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    let resolved = db::access::resolve_report(
        state.db(),
        family.family_id,
        report_id,
        family.user_id,
    )
    .await
    .map_err(AuthError::Internal)?;
    if resolved {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(AuthError::NotFound)
    }
}

/// What a `PUT /family/members/{id}` should persist, given what the admin sent
/// and what the member is today. Pure: the child invariant is the compliance
/// rule (see docs/family-permissions-plan.md), so it is worth testing without
/// standing up a database.
fn resolve_member_permissions(
    requested_bracket: Option<&str>,
    requested_upload: Option<bool>,
    requested_generate: Option<bool>,
    current_bracket: &str,
    current_upload: bool,
    current_generate: bool,
) -> Result<(String, bool, bool), AuthError> {
    let bracket = requested_bracket.unwrap_or(current_bracket).to_string();
    if !matches!(bracket.as_str(), "adult" | "teen" | "child") {
        return Err(AuthError::BadRequest(
            "age_bracket must be 'adult', 'teen' or 'child'".into(),
        ));
    }

    let can_upload = requested_upload.unwrap_or(current_upload);
    let can_generate = requested_generate.unwrap_or(current_generate);

    // A child never produces content; migration 0056 enforces it in the schema
    // too. Moving someone to `child` clears both flags rather than erroring, so
    // an admin need not send three fields to restrict one member — but asking
    // outright for a child to upload is refused, not silently ignored.
    if bracket == "child" {
        if requested_upload == Some(true) || requested_generate == Some(true) {
            return Err(AuthError::BadRequest(
                "a child member cannot be given upload or generation rights".into(),
            ));
        }
        return Ok((bracket, false, false));
    }

    Ok((bracket, can_upload, can_generate))
}

/// PUT /api/v1/family/members/{user_id} — change role and/or label
/// (family_admin only).
async fn update_member(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<UpdateMemberRequest>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;

    // Nobody edits their own permissions, admin or not — a restriction you can
    // lift from yourself is advisory, not a control. Role and label keep their
    // existing behaviour.
    let touches_permissions =
        body.age_bracket.is_some() || body.can_upload.is_some() || body.can_generate.is_some();
    if touches_permissions && user_id == family.user_id {
        return Err(AuthError::BadRequest(
            "cannot change your own permissions".into(),
        ));
    }

    if touches_permissions {
        let target = db::families::list_members(state.db(), family.family_id)
            .await
            .map_err(AuthError::Internal)?
            .into_iter()
            .find(|m| m.user_id == user_id)
            .ok_or(AuthError::NotFound)?;

        let (bracket, can_upload, can_generate) = resolve_member_permissions(
            body.age_bracket.as_deref(),
            body.can_upload,
            body.can_generate,
            &target.age_bracket,
            target.can_upload,
            target.can_generate,
        )?;

        let updated = db::families::set_member_permissions(
            state.db(),
            family.family_id,
            user_id,
            &bracket,
            can_upload,
            can_generate,
        )
        .await
        .map_err(AuthError::Internal)?;
        if !updated {
            return Err(AuthError::NotFound);
        }
    }

    if let Some(role) = body.role.as_deref() {
        if !matches!(role, "family_admin" | "member") {
            return Err(AuthError::BadRequest(
                "role must be 'family_admin' or 'member'".into(),
            ));
        }

        // Never let the last admin demote themselves out of the role — the
        // family would be left with nobody who can manage it.
        if role == "member" {
            let admins = db::families::count_admins(state.db(), family.family_id)
                .await
                .map_err(AuthError::Internal)?;
            let target_is_admin = db::families::list_members(state.db(), family.family_id)
                .await
                .map_err(AuthError::Internal)?
                .into_iter()
                .any(|m| m.user_id == user_id && m.role == "family_admin");

            if target_is_admin && admins <= 1 {
                return Err(AuthError::BadRequest(
                    "cannot demote the last family admin".into(),
                ));
            }
        }

        let updated = db::families::set_role(state.db(), family.family_id, user_id, role)
            .await
            .map_err(AuthError::Internal)?;
        if !updated {
            return Err(AuthError::NotFound);
        }
    }

    if let Some(label) = body.display_label {
        let trimmed = label.as_deref().map(str::trim).filter(|v| !v.is_empty());
        let updated =
            db::families::set_display_label(state.db(), family.family_id, user_id, trimmed)
                .await
                .map_err(AuthError::Internal)?;
        if !updated {
            return Err(AuthError::NotFound);
        }
    }

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/family/members/{user_id} — remove a member, or leave the
/// family when removing yourself. The removed user lands in a fresh personal
/// family so they keep working as a solo account.
async fn remove_member(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    let removing_self = user_id == family.user_id;
    if !removing_self {
        family.require_family_admin()?;
    }

    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let target = members
        .iter()
        .find(|m| m.user_id == user_id)
        .ok_or(AuthError::NotFound)?;

    if target.role == "family_admin" {
        let admins = db::families::count_admins(state.db(), family.family_id)
            .await
            .map_err(AuthError::Internal)?;
        if admins <= 1 && members.len() > 1 {
            return Err(AuthError::BadRequest(
                "promote another family admin before removing the last one".into(),
            ));
        }
    }

    // Sole member leaving their own family is a no-op: they are already a
    // family of one, and the family would be deleted out from under them.
    if members.len() == 1 {
        return Ok(StatusCode::NO_CONTENT);
    }

    // Leaving would found a second family, which this install does not have.
    // A pending account is still deleted below; anyone else is blocked or
    // deleted by an admin instead.
    if state.hooks().one_family() && !(target.pending && !removing_self) {
        return Err(AuthError::Conflict(
            "this server has one family: block the member or delete the account instead".into(),
        ));
    }

    // A provisioned account nobody ever claimed has no auth identity and
    // nothing of its own to preserve — delete it outright instead of
    // re-homing a phantom account into a personal family it will never sign
    // into. Its pending claim invite goes with it (ON DELETE CASCADE).
    if target.pending && !removing_self {
        db::users::delete_user(state.db(), user_id)
            .await
            .map_err(AuthError::Internal)?;
        return Ok(StatusCode::NO_CONTENT);
    }

    // Content follows its owner: anything they shared with this family goes
    // back to private rather than staying behind for the others.
    db::access::unshare_all_for_user(state.db(), user_id, family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    db::families::move_to_personal_family(state.db(), user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/family/members/{user_id}/block — deactivate a fellow member's account
/// (family_admin only): they can no longer log in, and every existing session/refresh token is
/// revoked immediately, so an already-signed-in device is cut off too — not just "can't sign
/// back in". Distinct from `remove_member`: this doesn't touch family membership or shared
/// content, it only stops the account from being usable. Scoped to the caller's own family via
/// `require_member` — a family_admin still can't touch a user outside their family, unlike the
/// instance-wide `PATCH /users/{id}` this reuses `is_active` from.
async fn block_member(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;

    if user_id == family.user_id {
        return Err(AuthError::BadRequest("you cannot block your own account".into()));
    }

    db::users::set_active(state.db(), user_id, false)
        .await
        .map_err(AuthError::Internal)?;
    db::sessions::revoke_all_for_user(state.db(), user_id)
        .await
        .map_err(AuthError::Internal)?;
    db::refresh_tokens::revoke_all_for_user(state.db(), user_id, None)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/family/members/{user_id}/unblock — reactivate a member's account
/// (family_admin only). They still need to sign in again — this doesn't restore any session that
/// `block_member` revoked.
async fn unblock_member(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;

    db::users::set_active(state.db(), user_id, true)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/family/members/{user_id}/avatar — set a fellow member's profile picture on
/// their behalf (family_admin only). Distinct from `crate::users::upload_avatar`
/// (`POST /users/me/avatar`, self-serve): this is the one admin-managed exception, for a member
/// with no easy way to set their own (e.g. a kid's provisioned account, or just a parent doing it
/// for them) — same `users.avatar_object_id` column and storage key shape either way.
async fn upload_member_avatar(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;

    // Field fully drained inside the loop body — see `crate::users::upload_avatar`'s own comment
    // on why a `Field<'_>` can't be held past a `break` here.
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
    if bytes.len() > MAX_MEMBER_AVATAR_BYTES {
        return Err(AuthError::BadRequest(format!(
            "avatar must be under {}MB",
            MAX_MEMBER_AVATAR_BYTES / 1_000_000
        )));
    }

    let ext = family_avatar_extension(&file_name).unwrap_or("jpg");
    // Same `u/{user_id}/…` personal-namespace key `crate::users::upload_avatar` uses — it's
    // still the member's own photo, just set on their behalf.
    let object_key = format!("u/{user_id}/avatar.{ext}");
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

    db::users::set_avatar(&mut *tx, user_id, Some(media_id))
        .await
        .map_err(AuthError::Internal)?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/family/members/{user_id}/avatar — remove a fellow member's photo
/// (family_admin only). Stored object left in place, same as every other cover-replace path.
async fn delete_member_avatar(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;
    db::users::set_avatar(state.db(), user_id, None)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/family/members/provision — create an account for a member
/// with no mailbox of their own (a kid, a grandparent) and hand back a
/// `claim` invite so they can set their own password (family_admin only).
/// The account has no auth identity until claimed — see `pending` on
/// `MemberResponse`.
async fn provision_member(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<ProvisionMemberRequest>,
) -> Result<(StatusCode, Json<ProvisionMemberResponse>), AuthError> {
    family.require_family_admin()?;

    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(AuthError::BadRequest("display name is required".into()));
    }

    let login_email = body.login_email.trim().to_lowercase();
    if login_email.is_empty() || !login_email.contains('@') {
        return Err(AuthError::BadRequest(
            "login identifier must look like an email address, e.g. name@yourfamily.family"
                .into(),
        ));
    }
    if db::users::find_by_email(state.db(), &login_email)
        .await
        .map_err(AuthError::Internal)?
        .is_some()
    {
        return Err(AuthError::BadRequest(
            "that login identifier is already in use".into(),
        ));
    }

    let user = db::users::insert(state.db(), &login_email, &display_name, "user")
        .await
        .map_err(AuthError::Internal)?;

    db::families::add_member(state.db(), family.family_id, user.id, "member")
        .await
        .map_err(AuthError::Internal)?;

    if let Some(label) = body.display_label.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        db::families::set_display_label(state.db(), family.family_id, user.id, Some(label))
            .await
            .map_err(AuthError::Internal)?;
    }

    let invite = db::families::create_invite(
        state.db(),
        db::families::NewInvite {
            family_id: family.family_id,
            email: None,
            code: &generate_invite_code(),
            role: "member",
            kind: "claim",
            max_uses: 1,
            label: None,
            claim_user_id: Some(user.id),
            created_by: family.user_id,
            expires_at: chrono::Utc::now() + chrono::Duration::days(CLAIM_INVITE_TTL_DAYS),
        },
    )
    .await
    .map_err(AuthError::Internal)?;

    let member = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .into_iter()
        .find(|m| m.user_id == user.id)
        .ok_or(AuthError::NotFound)?;

    Ok((
        StatusCode::CREATED,
        Json(ProvisionMemberResponse {
            member: member_to_response(member),
            invite: invite_to_response(&state, invite),
        }),
    ))
}

/// GET /api/v1/family/invites — pending invites (family_admin only).
async fn list_invites(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<InviteResponse>>, AuthError> {
    family.require_family_admin()?;

    let invites = db::families::list_invites(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(invites.into_iter().map(|i| invite_to_response(&state, i)).collect()))
}

/// POST /api/v1/family/invites — create an `email` or `link` invite
/// (family_admin only). `email` invites are mailed immediately when the
/// server has mail configured (`crate::mail`) and work for registration
/// even on instances where open registration is disabled; `link` invites
/// are always `member`-role (decision D3 — a shareable code must never be
/// able to grant admin) and may be redeemed by up to `max_uses` accounts.
async fn create_invite(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateInviteRequest>,
) -> Result<(StatusCode, Json<InviteResponse>), AuthError> {
    family.require_family_admin()?;

    let kind = body.kind.trim().to_lowercase();
    if !matches!(kind.as_str(), "email" | "link") {
        return Err(AuthError::BadRequest("kind must be 'email' or 'link'".into()));
    }
    if !matches!(body.role.as_str(), "family_admin" | "member") {
        return Err(AuthError::BadRequest(
            "role must be 'family_admin' or 'member'".into(),
        ));
    }
    if kind == "link" && body.role != "member" {
        return Err(AuthError::BadRequest(
            "link invites can only grant the 'member' role — use an email invite for family_admin"
                .into(),
        ));
    }

    let (email, ttl_days, max_uses) = if kind == "email" {
        let email = body
            .email
            .as_deref()
            .map(str::trim)
            .map(str::to_lowercase)
            .filter(|e| !e.is_empty() && e.contains('@'))
            .ok_or_else(|| AuthError::BadRequest("invalid email address".into()))?;
        (Some(email), INVITE_TTL_DAYS, 1)
    } else {
        let max_uses = body.max_uses.unwrap_or(1);
        if !(1..=20).contains(&max_uses) {
            return Err(AuthError::BadRequest("max_uses must be between 1 and 20".into()));
        }
        (None, LINK_INVITE_TTL_DAYS, max_uses)
    };

    let label = body.label.as_deref().map(str::trim).filter(|l| !l.is_empty());

    let invite = db::families::create_invite(
        state.db(),
        db::families::NewInvite {
            family_id: family.family_id,
            email: email.as_deref(),
            code: &generate_invite_code(),
            role: &body.role,
            kind: &kind,
            max_uses,
            label,
            claim_user_id: None,
            created_by: family.user_id,
            expires_at: chrono::Utc::now() + chrono::Duration::days(ttl_days),
        },
    )
    .await
    .map_err(AuthError::Internal)?;

    if kind == "email" {
        send_invite_mail_best_effort(&state, &family, &invite).await;
    }

    Ok((StatusCode::CREATED, Json(invite_to_response(&state, invite))))
}

/// DELETE /api/v1/family/invites/{id} — revoke a pending invite.
async fn delete_invite(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;

    let deleted = db::families::delete_invite(state.db(), id, family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    if !deleted {
        return Err(AuthError::NotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/family/invites/{id}/regenerate — mint a fresh code and TTL
/// for a pending invite, invalidating the old one (family_admin only). Used
/// for "resend"/"show QR again" without changing what the invite grants.
async fn regenerate_invite(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<InviteResponse>, AuthError> {
    family.require_family_admin()?;

    let existing = db::families::find_invite_by_id(state.db(), id, family.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    let ttl_days = match existing.kind.as_str() {
        "email" => INVITE_TTL_DAYS,
        "claim" => CLAIM_INVITE_TTL_DAYS,
        _ => LINK_INVITE_TTL_DAYS,
    };

    let invite = db::families::regenerate_invite(
        state.db(),
        id,
        family.family_id,
        &generate_invite_code(),
        chrono::Utc::now() + chrono::Duration::days(ttl_days),
    )
    .await
    .map_err(AuthError::Internal)?
    .ok_or_else(|| AuthError::BadRequest("invite has already been used".into()))?;

    if invite.kind == "email" {
        send_invite_mail_best_effort(&state, &family, &invite).await;
    }

    Ok(Json(invite_to_response(&state, invite)))
}

/// POST /api/v1/family/invites/accept — join the inviting family.
/// Requires an authenticated account whose email matches the invite.
async fn accept_invite(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<AcceptInviteRequest>,
) -> Result<Json<FamilyResponse>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    let invite = redeem_invite(&state, &body.code, &user.email).await?;

    // Claim before moving: mark_invite_accepted is the atomic guard against
    // two devices redeeming the same code concurrently.
    claim_invite(&state, invite.id, auth.user_id).await?;

    // Anything shared with the family being left reverts to private. Per
    // decision D1 the joiner's library does NOT follow them into the new
    // family — they share individual items there deliberately.
    if let Some(previous) = db::families::find_membership(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
    {
        db::access::unshare_all_for_user(state.db(), auth.user_id, previous.family_id)
            .await
            .map_err(AuthError::Internal)?;
    }

    db::families::move_to_family(state.db(), auth.user_id, invite.family_id, &invite.role)
        .await
        .map_err(AuthError::Internal)?;

    let record = db::families::find(state.db(), invite.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    let members = db::families::list_members(state.db(), invite.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let mine = members.iter().find(|m| m.user_id == auth.user_id);
    let (my_can_upload, my_can_generate) = match mine {
        // A family_admin always may, matching FamilyContext's rule.
        Some(m) if m.role == "family_admin" => (true, true),
        Some(m) => (m.can_upload, m.can_generate),
        None => (false, false),
    };

    Ok(Json(FamilyResponse {
        id: record.id.to_string(),
        name: record.name,
        my_role: invite.role,
        my_user_id: auth.user_id.to_string(),
        my_can_upload,
        my_can_generate,
        avatar_url: record.avatar_object_id.map(|_| "/api/v1/family/avatar".to_string()),
        members: members.into_iter().map(member_to_response).collect(),
        created_at: record.created_at.to_rfc3339(),
    }))
}

// ── Parental controls ─────────────────────────────────────────────────────
//
// These govern **shared** content only. Nothing here can reach into another
// member's private folder — `audio2_can_access` stops at the owner check.

#[derive(Serialize)]
pub struct MemberAccessResponse {
    pub user_id: String,
    /// Per-kind default: `allow_all` (the implicit default) or `deny_all`.
    pub policies: Vec<PolicyEntry>,
    /// Item-level overrides on top of the defaults.
    pub grants: Vec<GrantEntry>,
}

#[derive(Serialize)]
pub struct PolicyEntry {
    pub media_kind: String,
    pub policy: String,
}

#[derive(Serialize)]
pub struct GrantEntry {
    pub media_kind: String,
    pub item_id: String,
    pub effect: String,
}

#[derive(Deserialize)]
pub struct SetPolicyRequest {
    /// `audiobook`, `podcast`, or `music`.
    pub media_kind: String,
    /// `allow_all` or `deny_all`.
    pub policy: String,
}

#[derive(Deserialize)]
pub struct ReplaceGrantsRequest {
    pub media_kind: String,
    /// Items this member may play regardless of their default policy.
    #[serde(default)]
    pub allow: Vec<Uuid>,
    /// Items this member may not play regardless of their default policy.
    #[serde(default)]
    pub deny: Vec<Uuid>,
}

#[derive(Deserialize)]
pub struct SetAudienceRequest {
    /// Who may hear this item, by user id. Anyone left out is denied.
    ///
    /// The whole audience, not a delta: an admin editing this list is looking at all of it, and a
    /// patch shape would make two admins on two screens silently overwrite each other field by
    /// field instead of last-write-wins on something they could both see.
    pub can_listen: Vec<Uuid>,
}

#[derive(Serialize)]
pub struct AudienceEntry {
    pub user_id: String,
    pub display_name: String,
    pub display_label: Option<String>,
    pub can_listen: bool,
    /// True when access cannot be taken away: the item's owner, or a family admin. A client
    /// should show the state and no control, rather than a control that silently does nothing.
    pub locked: bool,
}

/// GET /api/v1/family/members/{user_id}/access — a member's effective
/// restrictions (family_admin only).
async fn get_member_access(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
) -> Result<Json<MemberAccessResponse>, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;

    let policies = db::access::list_policies(state.db(), family.family_id, user_id)
        .await
        .map_err(AuthError::Internal)?;
    let grants = db::access::list_grants(state.db(), family.family_id, user_id, None)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(MemberAccessResponse {
        user_id: user_id.to_string(),
        policies: policies
            .into_iter()
            .map(|p| PolicyEntry {
                media_kind: p.media_kind,
                policy: p.policy,
            })
            .collect(),
        grants: grants
            .into_iter()
            .map(|g| GrantEntry {
                media_kind: g.media_kind,
                item_id: g.item_id.to_string(),
                effect: g.effect,
            })
            .collect(),
    }))
}

/// PUT /api/v1/family/members/{user_id}/policy — set a member's default for
/// one media kind (family_admin only).
async fn set_member_policy(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<SetPolicyRequest>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;

    if !db::access::is_valid_kind(&body.media_kind) {
        return Err(AuthError::BadRequest(
            "media_kind must be 'audiobook', 'podcast', or 'music'".into(),
        ));
    }
    if !matches!(body.policy.as_str(), "allow_all" | "deny_all") {
        return Err(AuthError::BadRequest(
            "policy must be 'allow_all' or 'deny_all'".into(),
        ));
    }

    db::access::set_policy(
        state.db(),
        family.family_id,
        user_id,
        &body.media_kind,
        &body.policy,
        family.user_id,
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// PUT /api/v1/family/members/{user_id}/grants — replace a member's
/// item-level overrides for one media kind (family_admin only).
///
/// Bulk replace rather than per-item edits: the admin UI is a checkbox list,
/// and replacing wholesale keeps it consistent with what was on screen.
async fn replace_member_grants(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(user_id): Path<Uuid>,
    Json(body): Json<ReplaceGrantsRequest>,
) -> Result<StatusCode, AuthError> {
    family.require_family_admin()?;
    require_member(&state, family.family_id, user_id).await?;

    if !db::access::is_valid_kind(&body.media_kind) {
        return Err(AuthError::BadRequest(
            "media_kind must be 'audiobook', 'podcast', or 'music'".into(),
        ));
    }

    let overlap: Vec<&Uuid> = body.allow.iter().filter(|id| body.deny.contains(id)).collect();
    if !overlap.is_empty() {
        return Err(AuthError::BadRequest(
            "an item cannot be both allowed and denied".into(),
        ));
    }

    db::access::replace_grants(
        state.db(),
        family.family_id,
        user_id,
        &body.media_kind,
        &body.allow,
        &body.deny,
        family.user_id,
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/family/content/{kind}/{item_id}/audience — who in the family
/// can currently play this item. Drives the "who can listen" widget.
async fn content_audience(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((kind, item_id)): Path<(String, Uuid)>,
) -> Result<Json<Vec<AudienceEntry>>, AuthError> {
    family.require_family_admin()?;

    if !db::access::is_valid_kind(&kind) {
        return Err(AuthError::BadRequest(
            "kind must be 'audiobook', 'podcast', or 'music'".into(),
        ));
    }

    let audience = db::access::item_audience(state.db(), family.family_id, &kind, item_id)
        .await
        .map_err(AuthError::Internal)?;

    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        audience
            .into_iter()
            .filter_map(|(user_id, can_listen, locked)| {
                members.iter().find(|m| m.user_id == user_id).map(|m| AudienceEntry {
                    user_id: user_id.to_string(),
                    display_name: m.display_name.clone(),
                    display_label: m.display_label.clone(),
                    can_listen,
                    locked,
                })
            })
            .collect(),
    ))
}

/// PUT /api/v1/family/content/{kind}/{item_id}/audience — say who may hear one item
/// (family_admin only).
///
/// The other direction from `PUT /members/{id}/grants`, over the same `content_grants` rows: that
/// one answers "what may this person reach", this one answers "who may reach this". An admin
/// looking at a book thinks the second way, and had no way to say it.
///
/// Owners and family admins are filtered out rather than rejected. `audio2_can_access` gives them
/// access unconditionally, so a request that leaves one out is not refused — it just cannot take
/// away what the rule grants, and the response says so by still reporting them as listeners.
async fn set_content_audience(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((kind, item_id)): Path<(String, Uuid)>,
    Json(body): Json<SetAudienceRequest>,
) -> Result<Json<Vec<AudienceEntry>>, AuthError> {
    family.require_family_admin()?;

    if !db::access::is_valid_kind(&kind) {
        return Err(AuthError::BadRequest(
            "kind must be 'audiobook', 'podcast', or 'music'".into(),
        ));
    }

    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let wanted: std::collections::HashSet<Uuid> = body.can_listen.into_iter().collect();

    if let Some(stranger) = wanted.iter().find(|id| !members.iter().any(|m| m.user_id == **id)) {
        return Err(AuthError::BadRequest(format!(
            "{stranger} is not a member of this family"
        )));
    }

    let decisions: Vec<(Uuid, bool)> = members
        .iter()
        .filter(|m| m.role != "family_admin")
        .map(|m| (m.user_id, wanted.contains(&m.user_id)))
        .collect();

    db::access::set_item_audience(
        state.db(),
        family.family_id,
        &kind,
        item_id,
        &decisions,
        family.user_id,
    )
    .await
    .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    // Answer with what the rule now resolves to, not with what was asked for — the two differ
    // for owners and admins, and an admin who just removed someone should see that immediately
    // rather than after a refresh that contradicts the screen.
    content_audience(family, State(state), Path((kind, item_id))).await
}

/// POST /api/v1/family/content/{kind}/{item_id}/request-access
///
/// A member who cannot hear a family-shared item asks the family's admins to change that, rather
/// than the item just staying invisible with no way to act on it. Notifies every admin — a family
/// has no single "owner of permissions", and a request only one admin happens to see is not much
/// better than no request feature.
///
/// No de-duplication: a second tap sends a second notification. Acceptable for a household-sized
/// family and simpler than a request log; the client disables the button once it succeeds so a
/// double-click is the only realistic repeat, not a real gap.
async fn request_content_access(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((kind, item_id)): Path<(String, Uuid)>,
) -> Result<StatusCode, AuthError> {
    if !db::access::is_valid_kind(&kind) {
        return Err(AuthError::BadRequest(
            "kind must be 'audiobook', 'podcast', or 'music'".into(),
        ));
    }
    let Some(table) = db::access::table_for_kind(&kind) else {
        return Err(AuthError::ItemNotFound);
    };

    // Must be shared with *this* family. An item that isn't is not a permissions question this
    // feature answers — the requester should not learn anything about it either way, so a private
    // or foreign item and a nonexistent one look identical: 404.
    let title: Option<String> = sqlx::query_scalar(&format!(
        "SELECT t.title FROM {table} t WHERE t.id = $1 AND t.family_id = $2"
    ))
    .bind(item_id)
    .bind(family.family_id)
    .fetch_optional(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    let Some(title) = title else {
        return Err(AuthError::ItemNotFound);
    };

    let members = db::families::list_members(state.db(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    let requester_name = members
        .iter()
        .find(|m| m.user_id == family.user_id)
        .map(|m| m.display_label.clone().unwrap_or_else(|| m.display_name.clone()))
        .unwrap_or_else(|| "Someone".to_string());

    let notify_title = format!("{requester_name} would like access");
    let data = serde_json::json!({
        "media_kind": kind,
        "item_id": item_id.to_string(),
        "requester_id": family.user_id.to_string(),
    });

    for admin_id in members.iter().filter(|m| m.role == "family_admin").map(|m| m.user_id) {
        // Fire-and-forget, same as every other notification write in this codebase — a request
        // that failed to queue is not worth failing the response over.
        let _ = db::sync::queue_notification(
            state.db(),
            admin_id,
            "access_request",
            &notify_title,
            Some(&title),
            Some(&data),
        )
        .await;
    }

    Ok(StatusCode::NO_CONTENT)
}

/// Reject attempts to configure someone who is not in this family.
async fn require_member(
    state: &AppState,
    family_id: Uuid,
    user_id: Uuid,
) -> Result<(), AuthError> {
    let members = db::families::list_members(state.db(), family_id)
        .await
        .map_err(AuthError::Internal)?;

    if members.iter().any(|m| m.user_id == user_id) {
        Ok(())
    } else {
        Err(AuthError::NotFound)
    }
}

// ── Storage (bytes a family occupies; the credit that pays for them is the
// hosted edition's and lives in `billing::family`) ───────────────────────

/// GET /api/v1/family/storage — what the family's library occupies, per
/// media kind. Any member. The core's half of what `/family/billing` shows
/// in the hosted edition; a client that only needs bytes asks here.
async fn get_family_storage(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<BillingStorage>, AuthError> {
    let s = db::storage_usage::family_storage(state.db(), state.storage().bucket(), family.family_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(BillingStorage::from(s)))
}

impl From<db::storage_usage::StorageBreakdown> for BillingStorage {
    fn from(s: db::storage_usage::StorageBreakdown) -> Self {
        Self {
            total_bytes: s.total_bytes,
            audiobooks_bytes: s.audiobooks_bytes,
            podcasts_bytes: s.podcasts_bytes,
            music_bytes: s.music_bytes,
            other_bytes: s.other_bytes,
            unsized_objects: s.unsized_objects,
        }
    }
}

#[derive(Serialize)]
pub struct BillingStorage {
    pub total_bytes: i64,
    pub audiobooks_bytes: i64,
    pub podcasts_bytes: i64,
    pub music_bytes: i64,
    pub other_bytes: i64,
    /// Referenced objects with no recorded size — counted as zero bytes, so a
    /// non-zero value here means `total_bytes` is an undercount.
    pub unsized_objects: i64,
}

// ── Join & claim (public) ────────────────────────────────────────────────

/// GET /api/v1/join/{code} — public preview for the QR/link landing page.
/// Unknown codes are a plain 404; known-but-dead codes report their status
/// only — family name and inviter are revealed to valid-code holders alone
/// (decision D6).
async fn join_preview(
    State(state): State<AppState>,
    Path(code): Path<String>,
) -> Result<Json<JoinPreviewResponse>, AuthError> {
    // ItemNotFound (not NotFound): an unknown code is a plain 404, not the
    // account-lookup 401 `NotFound` means elsewhere in this module — this
    // endpoint has no signed-in caller to protect from enumeration.
    let invite = db::families::find_invite_by_code(state.db(), code.trim())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let dead = |status| {
        Json(JoinPreviewResponse {
            status,
            kind: None,
            family_name: None,
            inviter_name: None,
            role: None,
            member_count: None,
            expires_at: None,
            uses_left: None,
            claim: None,
        })
    };

    if invite.exhausted() {
        return Ok(dead("exhausted"));
    }
    if invite.expired() {
        return Ok(dead("expired"));
    }

    let family = db::families::find(state.db(), invite.family_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;
    let members = db::families::list_members(state.db(), invite.family_id)
        .await
        .map_err(AuthError::Internal)?;

    let inviter_name = match invite.created_by {
        Some(id) => db::users::find_by_id(state.db(), id)
            .await
            .map_err(AuthError::Internal)?
            .map(|u| u.display_name),
        None => None,
    };

    let claim = if invite.kind == "claim" {
        let claim_user_id = invite
            .claim_user_id
            .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("claim invite missing claim_user_id")))?;
        let user = db::users::find_by_id(state.db(), claim_user_id)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::NotFound)?;
        Some(ClaimPreview { display_name: user.display_name, login_email: user.email })
    } else {
        None
    };

    Ok(Json(JoinPreviewResponse {
        status: "valid",
        kind: Some(invite.kind.clone()),
        family_name: Some(family.name),
        inviter_name,
        role: Some(invite.role.clone()),
        member_count: Some(members.len() as i64),
        expires_at: Some(invite.expires_at.to_rfc3339()),
        uses_left: Some(invite.max_uses - invite.use_count),
        claim,
    }))
}

/// POST /api/v1/join/{code}/claim — set a password on a pre-provisioned
/// account and sign in. `claim` kind only; possession of the code is the
/// sole credential, same as every other invite kind.
async fn claim_provisioned_account(
    State(state): State<AppState>,
    Path(code): Path<String>,
    Json(body): Json<ClaimInviteRequest>,
) -> Result<(StatusCode, Json<crate::auth::LoginResponse>), AuthError> {
    if body.password.len() < 8 {
        return Err(AuthError::BadRequest(
            "password must be at least 8 characters".into(),
        ));
    }

    let invite = db::families::find_invite_by_code(state.db(), code.trim())
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("invite code is not valid".into()))?;

    if invite.kind != "claim" {
        return Err(AuthError::BadRequest(
            "this code is not an account-claim code".into(),
        ));
    }
    if invite.exhausted() {
        return Err(AuthError::BadRequest(
            "this invite has already been claimed".into(),
        ));
    }
    if invite.expired() {
        return Err(AuthError::BadRequest(
            "this invite has expired — ask a family admin for a new one".into(),
        ));
    }

    let claim_user_id = invite
        .claim_user_id
        .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("claim invite missing claim_user_id")))?;
    let user = db::users::find_by_id(state.db(), claim_user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    if db::users::find_identity_for_local(state.db(), user.id)
        .await
        .map_err(AuthError::Internal)?
        .is_some()
    {
        return Err(AuthError::BadRequest(
            "this account has already been claimed".into(),
        ));
    }

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("password hash failed: {e}")))?
        .to_string();

    db::users::insert_local_identity(state.db(), user.id, &password_hash)
        .await
        .map_err(AuthError::Internal)?;

    let claimed = db::families::mark_invite_accepted(state.db(), invite.id, user.id)
        .await
        .map_err(AuthError::Internal)?;
    if !claimed {
        // The identity now exists regardless — the invite bookkeeping losing
        // a concurrent race is a shrug, not a failure to report to the user.
        tracing::warn!(invite_id = %invite.id, "claim invite marker lost a race after identity was created");
    }

    let issued = crate::auth::issue_tokens(
        &state,
        &user,
        body.device_name.as_deref(),
        body.device_kind.as_deref(),
        None,
    )
    .await?;

    Ok((StatusCode::CREATED, Json(crate::auth::login_response(issued, user))))
}

// ── Shared helpers ────────────────────────────────────────────────────────

/// Validate a code and atomically claim it. Shared by the invite-aware
/// registration path (`crate::auth::register`) and `accept_invite` above.
/// Handles `email` (must match the registering address) and `link` (any
/// address) kinds; `claim` invites are rejected here — they are redeemed
/// through `claim_provisioned_account`, not registration/login.
pub(crate) async fn redeem_invite(
    state: &AppState,
    code: &str,
    email: &str,
) -> Result<db::families::FamilyInvite, AuthError> {
    let invite = db::families::find_invite_by_code(state.db(), code.trim())
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("invite code is not valid".into()))?;

    if invite.kind == "claim" {
        return Err(AuthError::BadRequest(
            "this code activates a pre-created account — use the claim link instead".into(),
        ));
    }
    if invite.exhausted() {
        return Err(AuthError::BadRequest("invite has already been used".into()));
    }
    if invite.expired() {
        return Err(AuthError::BadRequest("invite has expired".into()));
    }
    if invite.kind == "email" {
        let invite_email = invite.email.as_deref().unwrap_or_default();
        if !invite_email.eq_ignore_ascii_case(email) {
            return Err(AuthError::BadRequest(
                "invite was issued to a different email address".into(),
            ));
        }
    }

    Ok(invite)
}

/// Claim the invite for `user_id`. Separate from [`redeem_invite`] so the
/// registration path can create the account first and only then consume it.
pub(crate) async fn claim_invite(
    state: &AppState,
    invite_id: Uuid,
    user_id: Uuid,
) -> Result<(), AuthError> {
    let claimed = db::families::mark_invite_accepted(state.db(), invite_id, user_id)
        .await
        .map_err(AuthError::Internal)?;

    if !claimed {
        return Err(AuthError::BadRequest("invite has already been used".into()));
    }
    Ok(())
}

fn generate_invite_code() -> String {
    let mut bytes = [0u8; 12];
    OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn member_to_response(m: db::families::FamilyMember) -> MemberResponse {
    MemberResponse {
        user_id: m.user_id.to_string(),
        email: m.email,
        display_name: m.display_name,
        display_label: m.display_label,
        role: m.role,
        is_active: m.is_active,
        pending: m.pending,
        avatar_url: m.avatar_object_id.map(|_| format!("/api/v1/users/{}/avatar", m.user_id)),
        joined_at: m.joined_at.to_rfc3339(),
        age_bracket: m.age_bracket,
        can_upload: m.can_upload,
        can_generate: m.can_generate,
    }
}

fn invite_to_response(state: &AppState, i: db::families::FamilyInvite) -> InviteResponse {
    InviteResponse {
        id: i.id.to_string(),
        kind: i.kind.clone(),
        email: i.email.clone(),
        code: i.code.clone(),
        role: i.role.clone(),
        label: i.label.clone(),
        max_uses: i.max_uses,
        use_count: i.use_count,
        created_at: i.created_at.to_rfc3339(),
        expires_at: i.expires_at.to_rfc3339(),
        join_url: join_url(state, &i.code),
        member_user_id: i.claim_user_id.map(|id| id.to_string()),
    }
}

/// `{web_base}/join/{code}`, or `None` when neither base URL is configured —
/// clients then fall back to building it from whatever origin they're already
/// connected to.
fn join_url(state: &AppState, code: &str) -> Option<String> {
    state
        .config()
        .server
        .web_base()
        .map(|base| format!("{}/join/{code}", base.trim_end_matches('/')))
}

/// Send an `email`-kind invite by mail, best-effort. No-ops with a log line
/// when either mail isn't configured (`crate::mail::send_mail`'s own no-op)
/// or `server.base_url` isn't set (there is no absolute join URL to send —
/// the invite still works via the code shown in the admin's client).
async fn send_invite_mail_best_effort(
    state: &AppState,
    family: &FamilyContext,
    invite: &db::families::FamilyInvite,
) {
    let Some(to) = invite.email.as_deref() else { return };
    let Some(url) = join_url(state, &invite.code) else {
        tracing::warn!("server.base_url is unset — skipping invite email, code still works manually");
        return;
    };

    let inviter_name = db::users::find_by_id(state.db(), family.user_id)
        .await
        .ok()
        .flatten()
        .map(|u| u.display_name)
        .unwrap_or_else(|| "A family admin".to_string());

    let family_name = db::families::find(state.db(), family.family_id)
        .await
        .ok()
        .flatten()
        .map(|f| f.name)
        .unwrap_or_else(|| "your family".to_string());

    crate::mail::invite::send_invite_email(
        state.config().mail.as_ref(),
        to,
        crate::mail::invite::InviteEmail {
            family_name: &family_name,
            inviter_name: &inviter_name,
            join_url: &url,
            code: &invite.code,
            expires_at: invite.expires_at,
        },
    )
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── member permissions (docs/family-permissions-plan.md) ──

    #[test]
    fn unspecified_fields_keep_what_the_member_already_had() {
        let (bracket, upload, generate) =
            resolve_member_permissions(None, None, None, "teen", true, false).unwrap();
        assert_eq!(bracket, "teen");
        assert!(upload);
        assert!(!generate);
    }

    #[test]
    fn moving_a_member_to_child_clears_both_flags() {
        let (bracket, upload, generate) =
            resolve_member_permissions(Some("child"), None, None, "adult", true, true).unwrap();
        assert_eq!(bracket, "child");
        assert!(!upload, "a child must not keep upload rights");
        assert!(!generate, "a child must not keep generation rights");
    }

    #[test]
    fn a_child_cannot_be_granted_upload_or_generation() {
        assert!(
            resolve_member_permissions(Some("child"), Some(true), None, "adult", false, false)
                .is_err()
        );
        assert!(
            resolve_member_permissions(Some("child"), None, Some(true), "adult", false, false)
                .is_err()
        );
        // Also when the member is already a child and the bracket is not resent.
        assert!(
            resolve_member_permissions(None, Some(true), None, "child", false, false).is_err()
        );
    }

    // ── content reports (PLAY_COMPLIANCE.md A2) ──

    #[test]
    fn every_offered_report_reason_is_accepted() {
        for reason in db::access::REPORT_REASONS {
            assert!(
                db::access::is_valid_report_reason(reason),
                "{reason} is offered but rejected"
            );
        }
    }

    #[test]
    fn an_unknown_report_reason_is_rejected() {
        assert!(!db::access::is_valid_report_reason("spam"));
        assert!(!db::access::is_valid_report_reason(""));
    }

    #[test]
    fn offensive_is_reportable_since_play_requires_exactly_that() {
        assert!(db::access::is_valid_report_reason("offensive"));
    }

    #[test]
    fn an_unknown_age_bracket_is_rejected() {
        assert!(
            resolve_member_permissions(Some("toddler"), None, None, "adult", true, true).is_err()
        );
    }

    #[test]
    fn a_teen_may_be_granted_upload_explicitly() {
        let (bracket, upload, generate) =
            resolve_member_permissions(Some("teen"), Some(true), Some(false), "child", false, false)
                .unwrap();
        assert_eq!(bracket, "teen");
        assert!(upload);
        assert!(!generate);
    }
}
