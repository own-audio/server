// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign-in in the standard shape (DESIGN §5.1; docs/api-v2-plan.md, V2).
//!
//! `POST /auth/token` is the one place tokens come from, answered with the
//! OAuth 2.0 token response (RFC 6749 §5.1) whatever the grant: password,
//! refresh, the device code (RFC 8628), a provider's id token (RFC 7523's
//! jwt-bearer) or a two-factor code. The work behind each grant is v1's —
//! lockout, the trail, rotation with reuse detection, two-factor — called, not
//! copied. Errors carry OAuth's `error` and `error_description` beside the
//! problem-details members, so OAuth libraries and our clients both read them.

use crate::app::AppState;
use crate::auth::audit::RequestMeta;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::auth::{LoginRequest, LoginResponse, RefreshRequest, device, oidc, totp};
use crate::db;
use crate::v2::problem::Problem;
use axum::extract::{FromRequest, Path, Request, State};
use axum::http::{StatusCode, header};
use axum::{Form, Json};
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const GRANT_PASSWORD: &str = "password";
pub const GRANT_REFRESH: &str = "refresh_token";
pub const GRANT_DEVICE: &str = "urn:ietf:params:oauth:grant-type:device_code";
pub const GRANT_ASSERTION: &str = "urn:ietf:params:oauth:grant-type:jwt-bearer";
pub const GRANT_MFA: &str = "urn:own-audio:params:oauth:grant-type:mfa-otp";

/// Every scope there is (DESIGN §5.4).
pub const SCOPES: &[&str] = &["library:read", "library:write", "playback", "user:write", "remote", "family:admin", "admin"];

/// A body sent as a form (what OAuth libraries send) or as JSON.
pub struct FormOrJson<T>(pub T);

impl<T: DeserializeOwned + Send> FromRequest<AppState> for FormOrJson<T> {
    type Rejection = Problem;

    async fn from_request(req: Request, state: &AppState) -> Result<Self, Self::Rejection> {
        let form = req
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
        let invalid = |e: String| oauth(StatusCode::BAD_REQUEST, "invalid_request", e);
        if form {
            Form::<T>::from_request(req, state).await.map(|Form(t)| FormOrJson(t)).map_err(|e| invalid(e.body_text()))
        } else {
            Json::<T>::from_request(req, state).await.map(|Json(t)| FormOrJson(t)).map_err(|e| invalid(e.body_text()))
        }
    }
}

/// An OAuth error (RFC 6749 §5.2) as problem details.
fn oauth(status: StatusCode, error: &str, description: impl Into<String>) -> Problem {
    let description = description.into();
    Problem::new(status, error)
        .detail(description.clone())
        .with("error", error)
        .with("error_description", description)
}

/// What a v1 sign-in failure means to an OAuth client.
fn oauth_from(e: AuthError) -> Problem {
    match e {
        AuthError::InvalidCredentials | AuthError::NotFound | AuthError::SessionInvalid => {
            oauth(StatusCode::BAD_REQUEST, "invalid_grant", "the credentials, code or token are not valid")
        }
        AuthError::BadRequest(message) => oauth(StatusCode::BAD_REQUEST, "invalid_request", message),
        AuthError::ProviderNotConfigured => {
            oauth(StatusCode::BAD_REQUEST, "invalid_request", "this server does not accept that provider")
        }
        other => {
            let problem = Problem::from(other);
            let code = problem.code().to_string();
            problem.with("error", code)
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct TokenRequest {
    /// `password`, `refresh_token`, `urn:ietf:params:oauth:grant-type:device_code`,
    /// `urn:ietf:params:oauth:grant-type:jwt-bearer` or
    /// `urn:own-audio:params:oauth:grant-type:mfa-otp`.
    pub grant_type: String,
    /// password: the account's email.
    pub username: Option<String>,
    pub password: Option<String>,
    /// refresh_token
    pub refresh_token: Option<String>,
    /// device_code: from `POST /auth/device`.
    pub device_code: Option<String>,
    /// jwt-bearer: `apple`, `google` or `microsoft`, and the provider's id token.
    pub provider: Option<String>,
    pub assertion: Option<String>,
    /// jwt-bearer, Apple only: the name Apple gives once, at the first sign-in.
    pub full_name: Option<String>,
    /// jwt-bearer: an invitation, when the account is new.
    pub invite_code: Option<String>,
    /// mfa-otp: from the `mfa_required` answer, and the app's code or a recovery code.
    pub mfa_token: Option<String>,
    pub otp: Option<String>,
    /// The device, for the session list ("Anna's iPhone"; `ios`, `android`, `web`, `tv`…).
    pub device_name: Option<String>,
    pub device_kind: Option<String>,
}

/// The OAuth 2.0 token response.
#[derive(Serialize, ToSchema)]
pub struct TokenResponse {
    pub access_token: String,
    /// Always `Bearer`.
    pub token_type: String,
    /// Seconds the access token is good for.
    pub expires_in: u64,
    /// Rotates on every use; presenting a used one signs the device out.
    pub refresh_token: String,
    /// What the token may do, space-separated.
    pub scope: String,
}

impl TokenRequest {
    fn need<'a>(&'a self, value: &'a Option<String>, name: &str) -> Result<&'a str, Problem> {
        value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| oauth(StatusCode::BAD_REQUEST, "invalid_request", format!("`{name}` is required for this grant")))
    }
}

/// Tokens for an account: a password, a refresh token, an approved device, a
/// provider's id token, or a two-factor code. An account with two-factor
/// sign-in on answers the first step with `403 mfa_required` and an `mfa_token`
/// for the `mfa-otp` grant.
#[utoipa::path(post, path = "/auth/token", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body(content = TokenRequest, content_type = "application/x-www-form-urlencoded",
        description = "As a form, as OAuth clients send it; JSON is accepted too"),
    responses(
        (status = 200, body = TokenResponse),
        (status = 400, description = "OAuth `error`: `invalid_request`, `invalid_grant`, `unsupported_grant_type`; for the device code also `authorization_pending`, `slow_down`, `access_denied`, `expired_token`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`mfa_required`: finish with the `mfa-otp` grant and the `mfa_token` given", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`identity_conflict`: an account with the provider's email exists and the provider did not verify it", body = crate::v2::problem::ProblemBody)))]
pub async fn token(
    State(state): State<AppState>,
    meta: RequestMeta,
    FormOrJson(body): FormOrJson<TokenRequest>,
) -> Result<([(header::HeaderName, &'static str); 2], Json<TokenResponse>), Problem> {
    let signed = match body.grant_type.as_str() {
        GRANT_PASSWORD => {
            let login = LoginRequest {
                email: body.need(&body.username, "username")?.to_string(),
                password: body.need(&body.password, "password")?.to_string(),
                device_name: body.device_name.clone(),
                device_kind: body.device_kind.clone(),
            };
            crate::auth::login(State(state.clone()), meta, Json(login)).await.map_err(oauth_from)?
        }
        GRANT_REFRESH => {
            let refresh = RefreshRequest { refresh_token: body.need(&body.refresh_token, "refresh_token")?.to_string() };
            let Json(response) = crate::auth::refresh(State(state.clone()), Json(refresh)).await.map_err(oauth_from)?;
            totp::SignIn::Tokens(StatusCode::OK, response)
        }
        GRANT_MFA => {
            let verify = totp::VerifyRequest {
                mfa_token: body.need(&body.mfa_token, "mfa_token")?.to_string(),
                code: body.need(&body.otp, "otp")?.to_string(),
            };
            let Json(response) = totp::verify(State(state.clone()), meta, Json(verify)).await.map_err(oauth_from)?;
            totp::SignIn::Tokens(StatusCode::OK, response)
        }
        GRANT_DEVICE => totp::SignIn::Tokens(StatusCode::OK, device_grant(&state, body.need(&body.device_code, "device_code")?).await?),
        GRANT_ASSERTION => provider_grant(&state, &body).await?,
        other => {
            return Err(oauth(StatusCode::BAD_REQUEST, "unsupported_grant_type", format!("`{other}` is not a grant this server knows")));
        }
    };
    match signed {
        // RFC 6749 §5.1: tokens are never cached.
        totp::SignIn::Tokens(_, response) => Ok((
            [(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")],
            Json(token_response(&state, response).await?),
        )),
        totp::SignIn::Mfa(challenge) => Err(oauth(StatusCode::FORBIDDEN, "mfa_required", "finish with the mfa-otp grant")
            .with("mfa_token", challenge.mfa_token)
            .with("mfa_expires_in", challenge.expires_in_secs)
            .with("mfa_methods", challenge.methods)),
    }
}

/// A password sign-in gets every scope the account has.
async fn token_response(state: &AppState, response: LoginResponse) -> Result<TokenResponse, Problem> {
    let user_id: uuid::Uuid = response.user.id.parse().map_err(|e: uuid::Error| Problem::internal(anyhow::anyhow!(e)))?;
    let membership = db::families::ensure_membership(state.db(), user_id, state.hooks().one_family()).await?;
    let scope = SCOPES
        .iter()
        .filter(|s| match **s {
            "family:admin" => membership.role == "family_admin" || response.user.role == "admin",
            "admin" => response.user.role == "admin",
            _ => true,
        })
        .copied()
        .collect::<Vec<_>>()
        .join(" ");
    Ok(TokenResponse {
        access_token: response.token,
        token_type: "Bearer".into(),
        expires_in: state.config().auth.access_ttl(),
        refresh_token: response.refresh_token,
        scope,
    })
}

/// RFC 8628 §3.4–3.5: the device's poll, with the RFC's error codes.
async fn device_grant(state: &AppState, device_code: &str) -> Result<LoginResponse, Problem> {
    let pool = state.db();
    let pending = |error: &str, description: &str| oauth(StatusCode::BAD_REQUEST, error, description);
    let Some(request) = db::device_auth::find_by_code_hash(pool, &db::device_auth::hash_code(device_code)).await? else {
        return Err(pending("expired_token", "the device code is unknown or expired"));
    };
    let now = Utc::now();
    let elapsed = request.last_polled_at.map(|last| (now - last).num_seconds());
    let outcome = device::poll_outcome(&request.status, request.expired(now), elapsed, device::POLL_INTERVAL_SECS);
    db::device_auth::touch_poll(pool, request.id, now).await?;
    match outcome {
        device::PollOutcome::Pending => Err(pending("authorization_pending", "not approved yet")),
        device::PollOutcome::SlowDown => Err(pending("slow_down", "poll less often")),
        device::PollOutcome::Denied => Err(pending("access_denied", "the request was denied")),
        device::PollOutcome::Expired => Err(pending("expired_token", "the device code is unknown or expired")),
        device::PollOutcome::Approved => {
            let user_id = request.approved_user_id.ok_or_else(|| Problem::internal(anyhow::anyhow!("approved request has no user")))?;
            if !db::device_auth::consume(pool, request.id).await? {
                return Err(pending("expired_token", "the device code was already used"));
            }
            let user = db::users::find_by_id(pool, user_id)
                .await?
                .filter(|u| u.is_active)
                .ok_or_else(|| oauth(StatusCode::BAD_REQUEST, "invalid_grant", "the approving account is not active"))?;
            let issued = crate::auth::issue_tokens(state, &user, request.device_name.as_deref(), Some(request.device_kind.as_str()), None)
                .await
                .map_err(oauth_from)?;
            Ok(crate::auth::login_response(issued, user))
        }
    }
}

/// A provider's id token (DESIGN §5.2): verified against the provider's keys,
/// then matched to an account as v1's provider sign-in does.
async fn provider_grant(state: &AppState, body: &TokenRequest) -> Result<totp::SignIn, Problem> {
    let provider = body.need(&body.provider, "provider")?;
    let assertion = body.need(&body.assertion, "assertion")?;
    let auth = &state.config().auth;
    let not_here = || oauth(StatusCode::BAD_REQUEST, "invalid_request", format!("this server does not accept `{provider}`"));
    let identity = match provider {
        "google" => oidc::verify_google_id_token(auth.google.as_ref().filter(|g| g.enabled).ok_or_else(not_here)?, assertion).await,
        "microsoft" => {
            oidc::verify_microsoft_id_token(auth.microsoft.as_ref().filter(|m| m.enabled).ok_or_else(not_here)?, assertion).await
        }
        "apple" => {
            let cfg = auth.apple.as_ref().filter(|a| a.enabled).ok_or_else(not_here)?;
            oidc::verify_apple_id_token(cfg, assertion).await.map(|mut identity| {
                if let Some(name) = body.full_name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
                    identity.display_name = Some(name.to_string());
                }
                identity
            })
        }
        _ => return Err(not_here()),
    }
    .map_err(oauth_from)?;
    crate::auth::sso_sign_in(
        state,
        provider,
        identity,
        body.device_name.as_deref(),
        body.device_kind.as_deref(),
        body.invite_code.as_deref(),
    )
    .await
    .map_err(oauth_from)
}

#[derive(Deserialize, ToSchema)]
pub struct RevokeRequest {
    /// A refresh token (signs that device out) or a personal access token.
    pub token: String,
    pub token_type_hint: Option<String>,
}

/// Revoke a token (RFC 7009). Answers `200` whether or not the token was
/// known, so it says nothing about which tokens exist.
#[utoipa::path(post, path = "/auth/revoke", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body(content = RevokeRequest, content_type = "application/x-www-form-urlencoded",
        description = "As a form, as OAuth clients send it; JSON is accepted too"),
    responses((status = 200, description = "Revoked, or never known")))]
pub async fn revoke(State(state): State<AppState>, FormOrJson(body): FormOrJson<RevokeRequest>) -> Result<StatusCode, Problem> {
    let pool = state.db();
    let token = body.token.trim();
    let hash = db::refresh_tokens::hash_token(token);
    if token.starts_with(db::personal_tokens::PREFIX) {
        db::personal_tokens::revoke_by_hash(pool, &hash).await?;
    } else if let Some(stored) = db::refresh_tokens::find_by_hash(pool, &hash).await? {
        db::refresh_tokens::revoke_chain(pool, stored.chain_id, stored.user_id).await?;
    }
    Ok(StatusCode::OK)
}

#[derive(Deserialize, ToSchema, Default)]
pub struct DeviceAuthorizationRequest {
    /// Shown to the person approving it: "Living room TV".
    pub device_name: Option<String>,
    pub device_kind: Option<String>,
}

/// RFC 8628 §3.2.
#[derive(Serialize, ToSchema)]
pub struct DeviceAuthorizationResponse {
    /// Secret: the device polls `POST /auth/token` with it.
    pub device_code: String,
    /// Shown on the screen: `ABCD-EFGH`.
    pub user_code: String,
    /// Where to type the code; empty when the server has no console address.
    pub verification_uri: String,
    /// The same with the code filled in, for a QR code.
    pub verification_uri_complete: String,
    pub expires_in: i64,
    /// Seconds between polls.
    pub interval: i64,
}

/// Start signing in a TV or car (RFC 8628): show `user_code` (or a QR code of
/// `verification_uri_complete`), then poll `POST /auth/token` with
/// `grant_type=urn:ietf:params:oauth:grant-type:device_code`.
#[utoipa::path(post, path = "/auth/device", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body(content = DeviceAuthorizationRequest, content_type = "application/x-www-form-urlencoded",
        description = "As a form or JSON; may be empty"),
    responses((status = 200, body = DeviceAuthorizationResponse)))]
pub async fn device_authorization(State(state): State<AppState>, request: Request) -> Result<Json<DeviceAuthorizationResponse>, Problem> {
    // An empty body is fine: a TV has nothing to say but its name.
    let has_body = request.headers().get(header::CONTENT_LENGTH).and_then(|v| v.to_str().ok()).is_some_and(|v| v != "0");
    let body = if has_body {
        FormOrJson::<DeviceAuthorizationRequest>::from_request(request, &state).await?.0
    } else {
        DeviceAuthorizationRequest::default()
    };
    let (device_code, user_code) = device::generate_codes();
    let device_kind = crate::auth::normalize_device_kind(body.device_kind.as_deref());
    let expires_at = Utc::now() + chrono::Duration::seconds(device::REQUEST_TTL_SECS);
    db::device_auth::create(
        state.db(),
        &db::device_auth::hash_code(&device_code),
        &user_code,
        body.device_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        device_kind,
        expires_at,
    )
    .await?;
    let base = state.config().server.web_base().map(|b| format!("{}/link", b.trim_end_matches('/')));
    Ok(Json(DeviceAuthorizationResponse {
        verification_uri: base.clone().unwrap_or_default(),
        verification_uri_complete: base.map(|b| format!("{b}?code={user_code}")).unwrap_or_default(),
        device_code,
        user_code,
        expires_in: device::REQUEST_TTL_SECS,
        interval: device::POLL_INTERVAL_SECS,
    }))
}

#[derive(Serialize, ToSchema)]
pub struct DeviceRequest {
    pub user_code: String,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub expires_at: String,
}

/// What a code on a TV is asking for, before approving it.
#[utoipa::path(get, path = "/auth/device/{user_code}", tag = "auth", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(("user_code" = String, Path, description = "As shown; case, hyphens and look-alike characters are forgiven")),
    responses((status = 200, body = DeviceRequest), (status = 404, description = "Unknown, expired or already used", body = crate::v2::problem::ProblemBody)))]
pub async fn describe_device(_auth: AuthUser, State(state): State<AppState>, Path(user_code): Path<String>) -> Result<Json<DeviceRequest>, Problem> {
    let request = db::device_auth::find_pending_by_user_code(state.db(), &device::format_user_code(&user_code))
        .await?
        .ok_or_else(Problem::not_found)?;
    Ok(Json(DeviceRequest {
        user_code: request.user_code,
        device_name: request.device_name,
        device_kind: request.device_kind,
        expires_at: request.expires_at.to_rfc3339(),
    }))
}

/// Let the device in as the caller: its next poll gets tokens.
#[utoipa::path(post, path = "/auth/device/{user_code}/approve", tag = "auth", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(("user_code" = String, Path, description = "As shown")),
    responses((status = 204, description = "Approved"), (status = 404, description = "Unknown, expired or already used", body = crate::v2::problem::ProblemBody)))]
pub async fn approve_device(auth: AuthUser, State(state): State<AppState>, Path(user_code): Path<String>) -> Result<StatusCode, Problem> {
    resolve(&state, auth, &user_code, "approved").await
}

/// Refuse the device.
#[utoipa::path(post, path = "/auth/device/{user_code}/deny", tag = "auth", security(("bearer" = [])),
    extensions(("x-permission" = json!("user")), ("x-scope" = json!("user:write"))),
    params(("user_code" = String, Path, description = "As shown")),
    responses((status = 204, description = "Denied"), (status = 404, description = "Unknown, expired or already used", body = crate::v2::problem::ProblemBody)))]
pub async fn deny_device(auth: AuthUser, State(state): State<AppState>, Path(user_code): Path<String>) -> Result<StatusCode, Problem> {
    resolve(&state, auth, &user_code, "denied").await
}

async fn resolve(state: &AppState, auth: AuthUser, user_code: &str, status: &str) -> Result<StatusCode, Problem> {
    // A personal token cannot sign a device in: that would turn a script's
    // narrow token into a full session.
    if auth.scopes.is_some() {
        return Err(Problem::forbidden().detail("sign in to approve a device"));
    }
    let pool = state.db();
    let request = db::device_auth::find_pending_by_user_code(pool, &device::format_user_code(user_code))
        .await?
        .ok_or_else(Problem::not_found)?;
    let approved = (status == "approved").then_some(auth.user_id);
    if !db::device_auth::resolve(pool, request.id, status, approved).await? {
        return Err(Problem::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
