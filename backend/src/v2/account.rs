// SPDX-License-Identifier: AGPL-3.0-or-later
//! The account (docs/api-v2-completion-plan.md, phase 1): signing up, the
//! password and its reset, confirming the address, the security trail,
//! two-factor sign-in, the profile and its picture, deleting the account, and
//! the Subsonic key. The work is v1's — the same password rule, lockout,
//! mails and trail — called, not copied; what v2 adds is a stable problem
//! `code` for every refusal, and refusing what this server cannot do (mail
//! that is not set up) instead of pretending it did.
//!
//! Secrets travel only in bodies. Operations that hand out or change a
//! credential (the password, two-factor, the Subsonic key, deleting the
//! account) need a signed-in session: a personal access token cannot reach
//! them, so a script's narrow token never becomes the account.

use crate::app::AppState;
use crate::auth::audit::{self, RequestMeta};
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::auth::{password, totp, verification};
use crate::db;
use crate::families::FamilyContext;
use crate::v2::me::{self, Me};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// `501 feature_unavailable`: this server cannot do it as configured (discovery says so).
pub(crate) fn unavailable(feature: &str, detail: &str) -> Problem {
    Problem::new(StatusCode::NOT_IMPLEMENTED, "feature_unavailable").detail(detail).with("feature", feature)
}

/// The password rule (`auth::password`) as a problem on `field`.
pub(crate) fn check_password(value: &str, field: &str) -> Result<(), Problem> {
    password::check(value).map_err(|e| {
        let detail = match e {
            AuthError::BadRequest(m) => m,
            other => other.to_string(),
        };
        Problem::bad_request("weak_password", detail.clone()).field(field, "weak_password", detail)
    })
}

/// Credential operations are for a signed-in person, not a script's token.
fn session_only(auth: &AuthUser, what: &str) -> Result<(), Problem> {
    if auth.scopes.is_some() {
        return Err(Problem::forbidden().detail(format!("sign in to {what}; a personal access token cannot")));
    }
    Ok(())
}

fn invalid_token(detail: &str) -> Problem {
    Problem::bad_request("invalid_token", detail).field("/token", "invalid", "ask for a new link")
}

// ── Signing up ──────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub email: String,
    /// Twelve characters or more; a few words are fine.
    pub password: String,
    pub display_name: String,
    /// A family invitation: lets the account in when registration is closed, and puts it in
    /// the inviting household.
    pub invite_code: Option<String>,
}

/// A new account with a password, in its own household or the inviting one. Answers the
/// account; signing in is `POST /auth/token`. On a server that mails confirmation links, the
/// address is unconfirmed until the link is used (`email_verified`), unless an invitation sent
/// to this very address vouched for it.
#[utoipa::path(post, path = "/auth/register", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "The account; sign in with `POST /auth/token`", body = Me),
        (status = 400, description = "`invalid_email`, `weak_password`, `invalid_display_name`, `invalid_invite`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`registration_closed`: this server takes new accounts by invitation only", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`email_taken`", body = crate::v2::problem::ProblemBody)))]
pub async fn register(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<RegisterRequest>) -> Result<(StatusCode, Json<Me>), Problem> {
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(Problem::bad_request("invalid_email", "an email address is required").field("/email", "invalid", "not an email address"));
    }
    check_password(&body.password, "/password")?;
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(Problem::bad_request("invalid_display_name", "a name is required").field("/display_name", "required", "not blank"));
    }
    let invite = match body.invite_code.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Some(crate::families::redeem_invite(&state, code, &email).await.map_err(invalid_invite)?),
        None if !state.config().auth.registration_open => {
            return Err(Problem::new(StatusCode::FORBIDDEN, "registration_closed")
                .detail("this server takes new accounts by invitation only"));
        }
        None => None,
    };
    if db::users::find_by_email(state.db(), &email).await?.is_some() {
        return Err(Problem::new(StatusCode::CONFLICT, "email_taken").detail("an account with this email exists"));
    }
    let user = crate::auth::create_local_account(&state, &email, &body.password, &display_name, invite)
        .await
        .map_err(invalid_invite)?;
    audit::record(&state, &meta, Some(user.id), None, "account.created", serde_json::json!({ "via": "register" })).await;
    Ok((StatusCode::CREATED, Json(me::build_for(&state, user.id).await?)))
}

fn invalid_invite(e: AuthError) -> Problem {
    match e {
        AuthError::BadRequest(message) => Problem::bad_request("invalid_invite", message).field("/invite_code", "invalid", ""),
        other => other.into(),
    }
}

// ── The password ────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct PasswordResetRequest {
    pub email: String,
}

/// Mails a link to set a new password, good for thirty minutes. `202` whatever the address,
/// so it says nothing about which addresses have accounts. `501` on a server that does not
/// mail (`auth.password_reset` in discovery is `false`): there an admin sets a new password.
#[utoipa::path(post, path = "/auth/password-reset", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body = PasswordResetRequest,
    responses(
        (status = 202, description = "A link is on its way if the address has an account with a password"),
        (status = 501, description = "`feature_unavailable`: this server does not mail", body = crate::v2::problem::ProblemBody)))]
pub async fn password_reset(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<PasswordResetRequest>) -> Result<StatusCode, Problem> {
    if !crate::auth::password_reset_offered(state.config()) {
        return Err(unavailable("password_reset", "this server does not mail reset links; ask its admin"));
    }
    let _ = crate::auth::forgot_password(State(state), meta, Json(crate::auth::ForgotPasswordRequest { email: body.email })).await?;
    Ok(StatusCode::ACCEPTED)
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordResetConfirm {
    /// The secret from the mailed link.
    pub token: String,
    pub password: String,
}

/// Sets the password behind a mailed link and signs every device of the account out.
#[utoipa::path(post, path = "/auth/password-reset/confirm", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body = PasswordResetConfirm,
    responses(
        (status = 204, description = "Set; sign in again"),
        (status = 400, description = "`weak_password`, or `invalid_token`: the link is unknown, used or expired", body = crate::v2::problem::ProblemBody)))]
pub async fn password_reset_confirm(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<PasswordResetConfirm>) -> Result<StatusCode, Problem> {
    check_password(&body.password, "/password")?;
    let request = crate::auth::ResetPasswordRequest { token: body.token, password: body.password };
    let _ = crate::auth::reset_password(State(state), meta, Json(request)).await.map_err(|e| match e {
        AuthError::BadRequest(_) => invalid_token("this reset link is not valid any more; ask for a new one"),
        other => other.into(),
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct PasswordChange {
    pub current_password: String,
    pub new_password: String,
}

/// A new password. Every other device of the account is signed out; this one stays.
#[utoipa::path(put, path = "/me/password", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body = PasswordChange,
    responses(
        (status = 204, description = "Changed"),
        (status = 400, description = "`weak_password`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`invalid_credentials`: the current password is wrong; or asked with a personal access token", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`no_password`: the account signs in with a provider only", body = crate::v2::problem::ProblemBody)))]
pub async fn change_password(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta, Json(body): Json<PasswordChange>) -> Result<StatusCode, Problem> {
    session_only(&auth, "change the password")?;
    check_password(&body.new_password, "/new_password")?;
    let request = crate::auth::ChangePasswordRequest { current_password: body.current_password, new_password: body.new_password };
    crate::auth::change_password(auth, State(state), meta, Json(request)).await.map_err(|e| match e {
        // v1's two refusals once the new password passed the rule: no password at all, or the wrong one.
        AuthError::InvalidCredentials => no_password(),
        AuthError::BadRequest(_) => wrong_password("/current_password"),
        other => other.into(),
    })?;
    Ok(StatusCode::NO_CONTENT)
}

fn no_password() -> Problem {
    Problem::new(StatusCode::CONFLICT, "no_password").detail("this account signs in with a provider and has no password")
}

fn wrong_password(field: &str) -> Problem {
    Problem::new(StatusCode::FORBIDDEN, "invalid_credentials").detail("the password is not right").field(field, "invalid", "")
}

// ── Confirming the address ──────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct EmailVerification {
    /// The secret from the mailed link.
    pub token: String,
    /// A token from the page's human check (Cloudflare Turnstile), when it showed one. Optional:
    /// the address is confirmed without it; an edition may grant less for it.
    #[serde(default)]
    pub human_check: Option<String>,
}

/// The mailed confirmation link's landing call; works without signing in.
#[utoipa::path(post, path = "/auth/email-verification", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body = EmailVerification,
    responses(
        (status = 204, description = "The address is confirmed"),
        (status = 400, description = "`invalid_token`: the link is unknown, used or expired", body = crate::v2::problem::ProblemBody)))]
pub async fn verify_email(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<EmailVerification>) -> Result<StatusCode, Problem> {
    let _ = verification::verify_email(State(state), meta, Json(verification::VerifyEmailRequest { token: body.token, human_check: body.human_check }))
        .await
        .map_err(|e| match e {
            AuthError::BadRequest(_) => invalid_token("this confirmation link is not valid any more; ask for a new one"),
            other => other.into(),
        })?;
    Ok(StatusCode::NO_CONTENT)
}

/// Another confirmation link for the signed-in account. `409 already_verified` when there is
/// nothing to confirm, `501` on a server that does not mail (`auth.email_verification` in
/// discovery), where every address counts as confirmed.
#[utoipa::path(post, path = "/me/email-verification", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses(
        (status = 202, description = "A link is on its way"),
        (status = 409, description = "`already_verified`", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable`: this server does not mail", body = crate::v2::problem::ProblemBody)))]
pub async fn resend_verification(auth: AuthUser, State(state): State<AppState>) -> Result<StatusCode, Problem> {
    let Json(answer) = verification::resend_verification(auth, State(state)).await?;
    if answer["verified"] == true {
        return Err(Problem::new(StatusCode::CONFLICT, "already_verified").detail("the address is confirmed"));
    }
    if answer["sent"] != true {
        return Err(unavailable("email_verification", "this server does not mail confirmation links"));
    }
    Ok(StatusCode::ACCEPTED)
}

// ── The security trail ──────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct SecurityEvent {
    pub id: String,
    /// `login.ok`, `login.failed`, `login.locked`, `login.mfa_pending`, `login.mfa_ok`,
    /// `login.mfa_failed`, `password.changed`, `password.reset_requested`, `password.reset`,
    /// `email.verified`, `totp.enabled`, `totp.disabled`, `session.revoked`,
    /// `session.revoked_all`, `session.forked`, `token.created`, `token.revoked`,
    /// `account.created`, `role.changed`…; new kinds may appear.
    #[serde(rename = "type")]
    pub kind: String,
    pub at: DateTime<Utc>,
    /// The device's name when the event names one, else the client's user agent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub device: Option<String>,
    /// Where it came from, shortened to the network (`203.0.113.*`, `2001:db8:1:2::*`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip_hint: Option<String>,
    /// Someone else did it: an admin.
    pub by_admin: bool,
}

/// What happened to the account's security, newest first; kept a year.
#[utoipa::path(get, path = "/me/security-events", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(PageQuery),
    responses((status = 200, body = Page<SecurityEvent>), (status = 400, description = "`invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody)))]
pub async fn security_events(auth: AuthUser, State(state): State<AppState>, Query(q): Query<PageQuery>) -> Result<Json<Page<SecurityEvent>>, Problem> {
    let limit = q.limit()?;
    let after = q.after::<(DateTime<Utc>, Uuid)>()?;
    let rows = db::security_events::page_for_user(state.db(), auth.user_id, after, limit + 1).await?;
    let keys: Vec<(DateTime<Utc>, Uuid)> = rows.iter().map(|r| (r.at, r.id)).collect();
    let items: Vec<(SecurityEvent, (DateTime<Utc>, Uuid))> = rows.into_iter().zip(keys).map(|(r, k)| (event(r), k)).collect();
    let page = Page::from_rows(items, limit, |(_, k)| *k);
    Ok(Json(Page { items: page.items.into_iter().map(|(e, _)| e).collect(), next: page.next, included: None }))
}

fn event(row: db::security_events::SecurityEvent) -> SecurityEvent {
    let named = row.detail.get("device_name").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    SecurityEvent {
        id: row.id.to_string(),
        kind: row.kind,
        at: row.at,
        device: named.or(row.user_agent),
        ip_hint: row.ip.as_deref().and_then(ip_hint),
        by_admin: row.actor_id.is_some(),
    }
}

/// An address cut to its network, enough to tell home from somewhere else.
fn ip_hint(ip: &str) -> Option<String> {
    match ip.parse::<std::net::IpAddr>().ok()? {
        std::net::IpAddr::V4(v4) => {
            let [a, b, c, _] = v4.octets();
            Some(format!("{a}.{b}.{c}.*"))
        }
        std::net::IpAddr::V6(v6) => {
            let s = v6.segments();
            Some(format!("{:x}:{:x}:{:x}:{:x}::*", s[0], s[1], s[2], s[3]))
        }
    }
}

// ── Two-factor sign-in ──────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct TwoFactorSetup {
    /// Base32, for typing into an app by hand.
    pub secret: String,
    /// For the QR code the app scans.
    pub otpauth_uri: String,
}

/// Starts setting up two-factor sign-in: a fresh secret for an authenticator app, pending until
/// `PUT /me/two-factor` confirms it with a code. Starting again replaces a pending secret.
#[utoipa::path(post, path = "/me/two-factor", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses(
        (status = 201, body = TwoFactorSetup),
        (status = 403, description = "Asked with a personal access token", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`two_factor_on`: turn it off first", body = crate::v2::problem::ProblemBody)))]
pub async fn start_two_factor(auth: AuthUser, State(state): State<AppState>) -> Result<(StatusCode, Json<TwoFactorSetup>), Problem> {
    session_only(&auth, "set up two-factor sign-in")?;
    let Json(setup) = totp::setup(auth, State(state)).await.map_err(|e| match e {
        AuthError::Conflict(_) => two_factor_on(),
        other => other.into(),
    })?;
    Ok((StatusCode::CREATED, Json(TwoFactorSetup { secret: setup.secret, otpauth_uri: setup.otpauth_uri })))
}

#[derive(Deserialize, ToSchema)]
pub struct TwoFactorCode {
    /// Six digits from the app; to turn it off, a recovery code works too.
    pub code: String,
}

#[derive(Serialize, ToSchema)]
pub struct RecoveryCodes {
    /// Shown this once; each works once in place of a code.
    pub recovery_codes: Vec<String>,
}

/// Turns two-factor sign-in on: the first code from the app proves it holds the secret.
#[utoipa::path(put, path = "/me/two-factor", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body = TwoFactorCode,
    responses(
        (status = 200, body = RecoveryCodes),
        (status = 400, description = "`invalid_code`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Asked with a personal access token", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`two_factor_not_pending`: start with `POST /me/two-factor`; `two_factor_on`", body = crate::v2::problem::ProblemBody)))]
pub async fn enable_two_factor(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta, Json(body): Json<TwoFactorCode>) -> Result<Json<RecoveryCodes>, Problem> {
    session_only(&auth, "turn two-factor sign-in on")?;
    match db::totp::find(state.db(), auth.user_id).await? {
        None => return Err(Problem::new(StatusCode::CONFLICT, "two_factor_not_pending").detail("start with POST /me/two-factor")),
        Some(t) if t.enabled_at.is_some() => return Err(two_factor_on()),
        Some(_) => {}
    }
    let Json(codes) = totp::enable(auth, State(state), meta, Json(totp::CodeRequest { code: body.code }))
        .await
        .map_err(invalid_code)?;
    Ok(Json(RecoveryCodes { recovery_codes: codes.recovery_codes }))
}

/// Turns two-factor sign-in off with a code from the app or a recovery code, so a session
/// alone cannot remove it. A setup still pending is dropped without one.
#[utoipa::path(delete, path = "/me/two-factor", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body(content = TwoFactorCode, description = "Not needed to drop a pending setup"),
    responses(
        (status = 204, description = "Off"),
        (status = 400, description = "`invalid_code`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Asked with a personal access token", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`two_factor_off`", body = crate::v2::problem::ProblemBody)))]
pub async fn disable_two_factor(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta, body: Option<Json<TwoFactorCode>>) -> Result<StatusCode, Problem> {
    session_only(&auth, "turn two-factor sign-in off")?;
    if db::totp::find(state.db(), auth.user_id).await?.is_none() {
        return Err(Problem::new(StatusCode::CONFLICT, "two_factor_off").detail("two-factor sign-in is not on"));
    }
    let code = body.map(|Json(b)| b.code).unwrap_or_default();
    totp::disable(auth, State(state), meta, Json(totp::CodeRequest { code })).await.map_err(invalid_code)?;
    Ok(StatusCode::NO_CONTENT)
}

fn two_factor_on() -> Problem {
    Problem::new(StatusCode::CONFLICT, "two_factor_on").detail("two-factor sign-in is already on")
}

fn invalid_code(e: AuthError) -> Problem {
    match e {
        AuthError::BadRequest(_) => Problem::bad_request("invalid_code", "that code is not right; check the app and the time on this device")
            .field("/code", "invalid", ""),
        other => other.into(),
    }
}

// ── The profile ─────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct MePatch {
    pub display_name: Option<String>,
    pub recommendations_enabled: Option<bool>,
    /// The whole list (`en`, `cs`, or tags like `en-US`, kept as their base); `[]` for every
    /// language. At most 20.
    pub discovery_languages: Option<Vec<String>>,
}

/// Changes the profile; only what the body names.
#[utoipa::path(patch, path = "/me", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body = MePatch,
    responses(
        (status = 200, body = Me),
        (status = 400, description = "`invalid_display_name`, `too_many_languages`", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_me(family: FamilyContext, State(state): State<AppState>, Json(body): Json<MePatch>) -> Result<Json<Me>, Problem> {
    let name = match body.display_name.as_deref().map(str::trim) {
        Some("") => {
            return Err(Problem::bad_request("invalid_display_name", "a name is required").field("/display_name", "required", "not blank"));
        }
        other => other,
    };
    let languages = body.discovery_languages.as_deref().map(fold_languages).transpose()?;
    let pool = state.db();
    if let Some(name) = name {
        db::users::update_display_name(pool, family.user_id, name).await?;
    }
    if let Some(enabled) = body.recommendations_enabled {
        db::users::set_recommendations_enabled(pool, family.user_id, enabled).await?;
    }
    if let Some(languages) = languages {
        db::users::set_discovery_languages(pool, family.user_id, &languages).await?;
    }
    Ok(Json(me::build(&state, &family).await?))
}

/// Each tag folded to its base subtag, as every filter in the catalogue reads it, once.
fn fold_languages(tags: &[String]) -> Result<Vec<String>, Problem> {
    let mut folded: Vec<String> = Vec::new();
    for tag in tags {
        if let Some(base) = crate::podcasts::base_language(tag) {
            if !folded.contains(&base) {
                folded.push(base);
            }
        }
    }
    if folded.len() > 20 {
        return Err(Problem::bad_request("too_many_languages", "at most 20 languages").field("/discovery_languages", "too_long", "at most 20"));
    }
    Ok(folded)
}

#[derive(Deserialize, ToSchema, Default)]
pub struct AccountDeletion {
    /// Required when the account has a password.
    pub password: Option<String>,
}

/// Deletes the account and everything it owns, at once (its trash included). An account with a
/// password confirms with it; one that signs in only with a provider sends no body.
#[utoipa::path(delete, path = "/me", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body(content = AccountDeletion, description = "Needed when the account has a password"),
    responses(
        (status = 204, description = "Deleted; every token of the account stops working"),
        (status = 400, description = "`password_required`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`invalid_credentials`; or asked with a personal access token", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`last_admin`: the server's only admin cannot leave it without one", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_me(auth: AuthUser, State(state): State<AppState>, body: Option<Json<AccountDeletion>>) -> Result<StatusCode, Problem> {
    session_only(&auth, "delete the account")?;
    let pool = state.db();
    let has_password = db::users::find_identity_for_local(pool, auth.user_id).await?.is_some_and(|i| i.password_hash.is_some());
    if has_password {
        let Some(given) = body.and_then(|Json(b)| b.password).filter(|p| !p.is_empty()) else {
            return Err(Problem::bad_request("password_required", "confirm with the account's password").field("/password", "required", ""));
        };
        let user = db::users::find_by_id(pool, auth.user_id).await?.ok_or_else(|| Problem::new(StatusCode::UNAUTHORIZED, "unauthorized"))?;
        let login = crate::auth::LoginRequest { email: user.email, password: given, device_name: None, device_kind: None };
        crate::auth::verify_password_login(pool, &login).await.map_err(|e| match e {
            AuthError::InvalidCredentials => wrong_password("/password"),
            other => other.into(),
        })?;
    }
    crate::users::delete_self(auth, State(state)).await.map_err(|e| match e {
        AuthError::BadRequest(_) => Problem::new(StatusCode::CONFLICT, "last_admin").detail("the server's only admin cannot delete the account"),
        other => other.into(),
    })?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct ImageRequest {
    /// From `POST /uploads` (any kind, an image of at most 5 MB).
    pub upload_id: String,
}

/// A picture's bytes, as the request body.
#[derive(ToSchema)]
#[schema(value_type = String, format = Binary)]
#[allow(dead_code)]
pub struct ImageBytes(Vec<u8>);

/// The account's picture: `{upload_id}` of an image put with `POST /uploads`, or the image
/// itself as the body (`image/jpeg`, `image/png`, `image/webp`…, at most 5 MB), for anyone who
/// may not add to the library.
#[utoipa::path(put, path = "/me/image", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    request_body(content(
        (ImageRequest = "application/json"),
        (ImageBytes = "image/*"))),
    responses(
        (status = 200, body = Me),
        (status = 400, description = "`not_an_image`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "No such upload in this household", body = crate::v2::problem::ProblemBody),
        (status = 413, description = "`too_large`: over 5 MB", body = crate::v2::problem::ProblemBody),
        (status = 415, description = "`unsupported_media_type`: neither JSON nor an image", body = crate::v2::problem::ProblemBody)))]
pub async fn put_image(family: FamilyContext, State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Result<Json<Me>, Problem> {
    match picture(&state, &family, &headers, body).await? {
        Picture::Stored(object) => db::users::set_avatar(state.db(), family.user_id, Some(object)).await?,
        Picture::Bytes { bytes, content_type, ext } => crate::users::store_avatar(&state, family.user_id, bytes, &content_type, ext).await?,
    }
    Ok(Json(me::build(&state, &family).await?))
}

/// A person's or a household's picture, as a request brought it.
pub(crate) enum Picture {
    /// An upload of the caller's, registered.
    Stored(Uuid),
    /// The image itself, to store.
    Bytes { bytes: Bytes, content_type: String, ext: &'static str },
}

/// The body of a `PUT …/image` for a picture of people: `{upload_id}` as JSON, or the image itself
/// (at most 5 MB either way).
pub(crate) async fn picture(state: &AppState, family: &FamilyContext, headers: &HeaderMap, body: Bytes) -> Result<Picture, Problem> {
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or("").trim().to_ascii_lowercase())
        .unwrap_or_default();
    let too_large = || Problem::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large").detail("a picture is at most 5 MB");
    if content_type == "application/json" {
        let request: ImageRequest = serde_json::from_slice(&body).map_err(|e| Problem::bad_request("invalid_body", e.to_string()))?;
        let adopted = crate::uploads::adopt(state, family, request.upload_id.trim()).await.map_err(|e| match e {
            AuthError::NotFound => Problem::not_found(),
            AuthError::BadRequest(m) => Problem::bad_request("invalid_upload", m).field("/upload_id", "invalid", ""),
            other => other.into(),
        })?;
        if !adopted.content_type.starts_with("image/") {
            return Err(Problem::bad_request("not_an_image", format!("the upload is {}", adopted.content_type)).field("/upload_id", "invalid", ""));
        }
        if adopted.size_bytes > crate::users::MAX_AVATAR_BYTES as i64 {
            return Err(too_large());
        }
        Ok(Picture::Stored(adopted.media_object_id))
    } else if content_type.starts_with("image/") {
        if body.is_empty() {
            return Err(Problem::bad_request("not_an_image", "the body is empty"));
        }
        if body.len() > crate::users::MAX_AVATAR_BYTES {
            return Err(too_large());
        }
        let ext = image_extension(&content_type);
        Ok(Picture::Bytes { bytes: body, content_type, ext })
    } else {
        Err(Problem::new(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported_media_type")
            .detail("send `{\"upload_id\": …}` as JSON, or the image itself"))
    }
}

fn image_extension(content_type: &str) -> &'static str {
    match content_type {
        "image/png" => "png",
        "image/webp" => "webp",
        "image/gif" => "gif",
        "image/heic" => "heic",
        "image/avif" => "avif",
        _ => "jpg",
    }
}

/// Removes the account's picture.
#[utoipa::path(delete, path = "/me/image", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses((status = 204, description = "Removed, or there was none")))]
pub async fn delete_image(auth: AuthUser, State(state): State<AppState>) -> Result<StatusCode, Problem> {
    crate::users::delete_avatar(auth, State(state)).await?;
    Ok(StatusCode::NO_CONTENT)
}

// ── The Subsonic key ────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct SubsonicKey {
    /// What a Subsonic app asks for as the user name.
    pub username: String,
    /// What it asks for as the password (or API key).
    pub key: String,
}

/// The key third-party Subsonic players sign in with, made on first asking.
#[utoipa::path(get, path = "/me/subsonic-key", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses(
        (status = 200, body = SubsonicKey),
        (status = 403, description = "Asked with a personal access token", body = crate::v2::problem::ProblemBody)))]
pub async fn subsonic_key(auth: AuthUser, State(state): State<AppState>) -> Result<Json<SubsonicKey>, Problem> {
    session_only(&auth, "see the Subsonic key")?;
    let Json(k) = crate::users::get_subsonic_key(auth, State(state)).await?;
    Ok(Json(SubsonicKey { username: k.username, key: k.api_key }))
}

/// A new Subsonic key; players set up with the old one stop working.
#[utoipa::path(post, path = "/me/subsonic-key", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    responses(
        (status = 200, body = SubsonicKey),
        (status = 403, description = "Asked with a personal access token", body = crate::v2::problem::ProblemBody)))]
pub async fn new_subsonic_key(auth: AuthUser, State(state): State<AppState>) -> Result<Json<SubsonicKey>, Problem> {
    session_only(&auth, "change the Subsonic key")?;
    let Json(k) = crate::users::regenerate_subsonic_key(auth, State(state)).await?;
    Ok(Json(SubsonicKey { username: k.username, key: k.api_key }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_address_is_cut_to_its_network() {
        assert_eq!(ip_hint("203.0.113.77").as_deref(), Some("203.0.113.*"));
        assert_eq!(ip_hint("2001:db8:1:2:3:4:5:6").as_deref(), Some("2001:db8:1:2::*"));
        assert_eq!(ip_hint("not an address"), None);
    }

    #[test]
    fn languages_fold_to_their_base_once() {
        let folded = fold_languages(&["en-US".into(), "en".into(), "cs-CZ".into()]).unwrap();
        assert_eq!(folded, vec!["en", "cs"]);
        let many: Vec<String> = (0..21).map(|i| format!("{}{}", (b'a' + (i / 26) as u8) as char, (b'a' + (i % 26) as u8) as char)).collect();
        assert_eq!(fold_languages(&many).unwrap_err().code(), "too_many_languages");
    }

    #[test]
    fn a_short_password_is_weak_on_its_field() {
        let p = check_password("short", "/new_password").unwrap_err();
        assert_eq!((p.status(), p.code()), (StatusCode::BAD_REQUEST, "weak_password"));
        assert!(check_password("a few words are fine", "/password").is_ok());
    }

    #[test]
    fn a_device_is_named_or_the_user_agent() {
        let row = |detail: serde_json::Value| db::security_events::SecurityEvent {
            id: Uuid::nil(),
            at: Utc::now(),
            kind: "login.ok".into(),
            detail,
            actor_id: None,
            ip: Some("198.51.100.4".into()),
            user_agent: Some("curl/8".into()),
        };
        let e = event(row(serde_json::json!({ "device_name": "Anna's iPhone" })));
        assert_eq!(e.device.as_deref(), Some("Anna's iPhone"));
        assert_eq!(e.ip_hint.as_deref(), Some("198.51.100.*"));
        assert_eq!(event(row(serde_json::json!({}))).device.as_deref(), Some("curl/8"));
    }
}
