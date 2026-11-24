// SPDX-License-Identifier: AGPL-3.0-or-later
//! Sign-in in the standard shape (DESIGN §5.1; docs/api-v2-plan.md, V2).
//!
//! `POST /auth/token` is the one place tokens come from, answered with the
//! OAuth 2.0 token response (RFC 6749 §5.1) whatever the grant: password,
//! refresh, the device code (RFC 8628), a provider's id token (RFC 7523's
//! jwt-bearer), a provider's authorization code with PKCE (RFC 7636, the
//! desktop apps' loopback sign-in), a refresh token exchanged for a second
//! session (RFC 8693, a helper process), the code of Sign in with Apple run in a
//! browser for apps without Apple's SDK (`auth::apple_web`, with PKCE) or a
//! two-factor code. The work behind each grant is v1's —
//! lockout, the trail, rotation with reuse detection, two-factor — called, not
//! copied. Errors carry OAuth's `error` and `error_description` beside the
//! problem-details members, so OAuth libraries and our clients both read them.

use crate::app::AppState;
use crate::auth::audit::{self, RequestMeta};
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::auth::{LoginRequest, LoginResponse, RefreshRequest, apple_web, device, oidc, totp};
use crate::db;
use crate::v2::problem::Problem;
use axum::extract::{FromRequest, Path, Query, Request, State};
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
pub const GRANT_TOKEN_EXCHANGE: &str = "urn:ietf:params:oauth:grant-type:token-exchange";
pub const GRANT_CODE: &str = "authorization_code";
/// The code `GET /auth/apple/start` … `/auth/apple/return` hands an app (`auth::apple_web`).
pub const GRANT_APPLE_CODE: &str = "urn:own-audio:params:oauth:grant-type:apple-code";
pub const TOKEN_TYPE_REFRESH: &str = "urn:ietf:params:oauth:token-type:refresh_token";
pub const TOKEN_TYPE_ACCESS: &str = "urn:ietf:params:oauth:token-type:access_token";

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

/// The refresh-like grants: their credential is a refresh token, a long random secret nobody
/// guesses, and a family's devices present one every few minutes from one address behind a
/// home router. They count against v1 refresh's bucket (`SERVER__RATE_LIMIT__REFRESH_PER_MINUTE`)
/// so a household's devices refreshing at once cannot use up its members' sign-in attempts.
/// The token exchange presents a live refresh token the same way. Every grant that checks a
/// password, a code or a provider's word keeps sign-in's smaller bucket (and the per-account
/// lockout behind the password grant).
fn refresh_like(grant_type: &str) -> bool {
    matches!(grant_type, GRANT_REFRESH | GRANT_TOKEN_EXCHANGE)
}

/// What axum's form and JSON extractors accept by default; the handler reads the body again.
const TOKEN_BODY_LIMIT: usize = 2 * 1024 * 1024;

#[derive(Deserialize)]
struct GrantOnly {
    grant_type: String,
}

/// The grant a token request names, read the way [`FormOrJson`] reads it; `None` when the
/// body does not parse, which then counts as a sign-in and is refused by the handler.
fn grant_of(content_type: Option<&str>, body: &[u8]) -> Option<String> {
    let form = content_type.is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    if form {
        form_urlencoded::parse(body).find(|(k, _)| k == "grant_type").map(|(_, v)| v.into_owned())
    } else {
        serde_json::from_slice::<GrantOnly>(body).ok().map(|g| g.grant_type)
    }
}

#[derive(Clone)]
struct TokenLimits {
    login: crate::http::rate_limit::Limiter,
    refresh: crate::http::rate_limit::Limiter,
}

/// `POST /auth/token` with the bucket of the grant it carries (see [`refresh_like`]).
pub fn limit_by_grant(
    limits: &crate::http::rate_limit::Limiters,
    route: axum::routing::MethodRouter<AppState>,
) -> axum::routing::MethodRouter<AppState> {
    if !limits.login.is_enabled() && !limits.refresh.is_enabled() {
        return route;
    }
    let state = TokenLimits { login: limits.login.clone(), refresh: limits.refresh.clone() };
    route.route_layer(axum::middleware::from_fn_with_state(state, by_grant))
}

async fn by_grant(State(limits): State<TokenLimits>, request: Request, next: axum::middleware::Next) -> axum::response::Response {
    use axum::response::IntoResponse;
    let (parts, body) = request.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, TOKEN_BODY_LIMIT).await else {
        // Counted as a sign-in before refusing, so an oversized body is no free try.
        if let Some(refused) = limits.login.refusal(&parts.extensions, &parts.headers) {
            return refused;
        }
        return Problem::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large").detail("the request body is too large").into_response();
    };
    let content_type = parts.headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok());
    let limiter = match grant_of(content_type, &bytes) {
        Some(grant) if refresh_like(&grant) => &limits.refresh,
        _ => &limits.login,
    };
    if let Some(refused) = limiter.refusal(&parts.extensions, &parts.headers) {
        return refused;
    }
    next.run(Request::from_parts(parts, axum::body::Body::from(bytes))).await
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
    /// `urn:ietf:params:oauth:grant-type:jwt-bearer`, `authorization_code`,
    /// `urn:ietf:params:oauth:grant-type:token-exchange`,
    /// `urn:own-audio:params:oauth:grant-type:apple-code` or
    /// `urn:own-audio:params:oauth:grant-type:mfa-otp`. `auth.grants` in discovery lists
    /// those this server takes.
    pub grant_type: String,
    /// password: the account's email.
    pub username: Option<String>,
    pub password: Option<String>,
    /// refresh_token
    pub refresh_token: Option<String>,
    /// device_code: from `POST /auth/device`.
    pub device_code: Option<String>,
    /// jwt-bearer: `apple`, `google` or `microsoft`, and the provider's id token;
    /// authorization_code: `google` or `microsoft`.
    pub provider: Option<String>,
    pub assertion: Option<String>,
    /// jwt-bearer, Apple only: the name Apple gives once, at the first sign-in.
    pub full_name: Option<String>,
    /// jwt-bearer, authorization_code and apple-code: an invitation, when the account is new.
    pub invite_code: Option<String>,
    /// authorization_code: the code the provider sent to the app's loopback address, the
    /// PKCE verifier its challenge was made from, and that loopback address
    /// (`http://127.0.0.1:<port>/…`), exactly as sent to the provider. The desktop client ids
    /// are `auth.clients` in discovery.
    ///
    /// apple-code: the `code` this server sent to the app's return address after
    /// `GET /auth/apple/start`, and the PKCE verifier of the `code_challenge` given there. Good
    /// once, for five minutes; a wrong verifier spends it too.
    pub code: Option<String>,
    pub code_verifier: Option<String>,
    pub redirect_uri: Option<String>,
    /// token-exchange: a live refresh token of the account, and its type,
    /// `urn:ietf:params:oauth:token-type:refresh_token`. Answers tokens for a new, separate
    /// device (a helper process that refreshes on its own); the presented token keeps working.
    pub subject_token: Option<String>,
    pub subject_token_type: Option<String>,
    /// token-exchange: `urn:ietf:params:oauth:token-type:access_token` (the default) or
    /// `…:refresh_token`; either way both are answered.
    pub requested_token_type: Option<String>,
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
    /// token-exchange only (RFC 8693 §2.2.1): `urn:ietf:params:oauth:token-type:access_token`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issued_token_type: Option<String>,
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
/// provider's id token or authorization code, a refresh token exchanged for a
/// second device, or a two-factor code. An account with two-factor
/// sign-in on answers the first step with `403 mfa_required` and an `mfa_token`
/// for the `mfa-otp` grant.
#[utoipa::path(post, path = "/auth/token", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body(content = TokenRequest, content_type = "application/x-www-form-urlencoded",
        description = "As a form, as OAuth clients send it; JSON is accepted too"),
    responses(
        (status = 200, body = TokenResponse),
        (status = 400, description = "OAuth `error`: `invalid_request`, `invalid_grant`, `unsupported_grant_type`; for the device code also `authorization_pending`, `slow_down`, `access_denied`, `expired_token`; `account_locked` (429) after a run of wrong passwords", body = crate::v2::problem::ProblemBody),
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
        GRANT_CODE => code_grant(&state, &body).await?,
        GRANT_APPLE_CODE => apple_code_grant(&state, &body).await?,
        GRANT_TOKEN_EXCHANGE => totp::SignIn::Tokens(StatusCode::OK, token_exchange(&state, &meta, &body).await?),
        other => {
            return Err(oauth(StatusCode::BAD_REQUEST, "unsupported_grant_type", format!("`{other}` is not a grant this server knows")));
        }
    };
    match signed {
        // RFC 6749 §5.1: tokens are never cached.
        totp::SignIn::Tokens(_, response) => {
            let mut answer = token_response(&state, response).await?;
            if body.grant_type == GRANT_TOKEN_EXCHANGE {
                answer.issued_token_type = Some(TOKEN_TYPE_ACCESS.into());
            }
            Ok(([(header::CACHE_CONTROL, "no-store"), (header::PRAGMA, "no-cache")], Json(answer)))
        }
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
        issued_token_type: None,
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

/// What an authorization-code grant names, checked before anyone is asked: the
/// provider, the code, a PKCE verifier as RFC 7636 §4.1 has it, and a loopback
/// redirect (the code endpoint is not to be pointed at another host).
fn code_request(body: &TokenRequest) -> Result<(&str, &str, &str, &str), Problem> {
    let provider = body.need(&body.provider, "provider")?;
    let code = body.need(&body.code, "code")?;
    let verifier = body.need(&body.code_verifier, "code_verifier")?;
    let redirect = body.need(&body.redirect_uri, "redirect_uri")?;
    let unreserved = |c: char| c.is_ascii_alphanumeric() || matches!(c, '-' | '.' | '_' | '~');
    if !(43..=128).contains(&verifier.len()) || !verifier.chars().all(unreserved) {
        return Err(oauth(StatusCode::BAD_REQUEST, "invalid_request", "`code_verifier` is 43 to 128 characters of A-Z, a-z, 0-9, -, ., _ and ~ (RFC 7636)"));
    }
    if !crate::auth::is_loopback_redirect(redirect) {
        return Err(oauth(StatusCode::BAD_REQUEST, "invalid_request", "`redirect_uri` must be a loopback address (http://127.0.0.1, [::1] or localhost)"));
    }
    Ok((provider, code, verifier, redirect))
}

/// The desktop apps' sign-in (RFC 8252): the app ran the provider's page with a
/// PKCE challenge and a loopback redirect, and sends the code it got; this
/// server, which holds the client secret, exchanges it with the provider and
/// signs in with the id token it gets back, as the jwt-bearer grant does.
async fn code_grant(state: &AppState, body: &TokenRequest) -> Result<totp::SignIn, Problem> {
    let (provider, code, verifier, redirect) = code_request(body)?;
    let auth = &state.config().auth;
    let not_here = || oauth(StatusCode::BAD_REQUEST, "invalid_request", format!("this server does not take `{provider}` codes"));
    let refused = |e: AuthError| match e {
        // The provider would not exchange the code: used, expired, or not made for this verifier.
        AuthError::BadRequest(_) => oauth(StatusCode::BAD_REQUEST, "invalid_grant", "the provider did not accept the code"),
        other => oauth_from(other),
    };
    let identity = match provider {
        "google" => {
            let cfg = auth.google.as_ref().filter(|g| g.enabled && desktop_client(g.desktop_client_id.as_deref()) && desktop_client(g.desktop_client_secret.as_deref()))
                .ok_or_else(not_here)?;
            let id_token = oidc::exchange_google_code(cfg, code, verifier, redirect).await.map_err(refused)?;
            oidc::verify_google_id_token(cfg, &id_token).await
        }
        "microsoft" => {
            let cfg = auth.microsoft.as_ref().filter(|m| m.enabled && desktop_client(m.desktop_client_id.as_deref())).ok_or_else(not_here)?;
            let id_token = oidc::exchange_microsoft_code(cfg, code, verifier, redirect).await.map_err(refused)?;
            oidc::verify_microsoft_id_token(cfg, &id_token).await
        }
        _ => return Err(not_here()),
    }
    .map_err(oauth_from)?;
    crate::auth::sso_sign_in(state, provider, identity, body.device_name.as_deref(), body.device_kind.as_deref(), body.invite_code.as_deref())
        .await
        .map_err(oauth_from)
}

/// The code of a browser sign-in with Apple (`auth::apple_web`): spent once, then the PKCE
/// verifier must make the challenge the app started with, and the account is found or made
/// exactly as for Apple's id token in the jwt-bearer grant.
async fn apple_code_grant(state: &AppState, body: &TokenRequest) -> Result<totp::SignIn, Problem> {
    let code = body.need(&body.code, "code")?;
    let verifier = body.need(&body.code_verifier, "code_verifier")?;
    if !apple_web::verifier_valid(verifier) {
        return Err(oauth(StatusCode::BAD_REQUEST, "invalid_request", "`code_verifier` is 43 to 128 characters of A-Z, a-z, 0-9, -, ., _ and ~ (RFC 7636)"));
    }
    if apple_web::configured(state.config()).is_none() {
        return Err(oauth(StatusCode::BAD_REQUEST, "invalid_request", "this server does not run Sign in with Apple in a browser"));
    }
    let invalid = || oauth(StatusCode::BAD_REQUEST, "invalid_grant", "the code is unknown, expired or already used, or the verifier does not match it");
    let taken = db::apple_web::take_code(state.db(), &db::apple_web::hash(code)).await?.ok_or_else(invalid)?;
    if !apple_web::pkce_matches(verifier, &taken.code_challenge) {
        tracing::info!("apple web sign-in: a code was presented with the wrong verifier and is spent");
        return Err(invalid());
    }
    let identity = oidc::ProviderIdentity {
        subject: taken.subject,
        email: taken.email,
        email_verified: taken.email_verified,
        display_name: taken.full_name,
    };
    crate::auth::sso_sign_in(state, "apple", identity, body.device_name.as_deref(), body.device_kind.as_deref(), body.invite_code.as_deref())
        .await
        .map_err(oauth_from)
}

/// `GET /auth/apple/start`'s query.
#[derive(Deserialize, utoipa::IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AppleStartParams {
    /// Where to send the app the answer: its own scheme, one of the server's allowed addresses
    /// (`audio.own.musicplayer:/apple` and the other own.audio apps' by default).
    pub redirect_uri: Option<String>,
    /// The PKCE challenge (RFC 7636): base64url of the SHA-256 of the app's verifier.
    pub code_challenge: Option<String>,
    /// `S256`, the only method taken.
    pub code_challenge_method: Option<String>,
    /// The app's own state, handed back with the answer; up to 1024 characters.
    pub state: Option<String>,
}

const APP_STATE_MAX: usize = 1024;

fn apple_web_off() -> Problem {
    Problem::new(StatusCode::NOT_IMPLEMENTED, "feature_unavailable")
        .detail("this server does not run Sign in with Apple in a browser")
        .with("feature", "apple_web")
}

/// A redirect no cache keeps and no page after it learns of.
fn redirect(status: StatusCode, location: &str) -> axum::response::Response {
    use axum::response::IntoResponse;
    (
        status,
        [(header::LOCATION, location), (header::CACHE_CONTROL, "no-store"), (header::REFERRER_POLICY, "no-referrer")],
    )
        .into_response()
}

/// Sign in with Apple in a browser, for apps without Apple's SDK (Android): open this in a
/// browser tab (a Custom Tab), and the person signs in at Apple. The app gets back
/// `<redirect_uri>?code=…&state=…`, or `?error=access_denied|server_error&state=…`, and spends
/// the code with the `urn:own-audio:params:oauth:grant-type:apple-code` grant and its PKCE
/// verifier. `auth.apple_web` in discovery says whether this server runs it.
#[utoipa::path(get, path = "/auth/apple/start", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    params(AppleStartParams),
    responses(
        (status = 302, description = "To Apple's sign-in page (`Location`)"),
        (status = 400, description = "`invalid_request`: a return address that is not allowed, or a missing or malformed PKCE challenge", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable` (`feature: apple_web`): Apple, its Services ID or the server's public address is not configured", body = crate::v2::problem::ProblemBody)))]
pub async fn apple_start(State(state): State<AppState>, Query(params): Query<AppleStartParams>) -> Result<axum::response::Response, Problem> {
    let Some((apple, client_id, base)) = apple_web::configured(state.config()) else {
        return Err(apple_web_off());
    };
    let bad = |detail: &str| Problem::bad_request("invalid_request", detail);
    let redirect_uri = params.redirect_uri.as_deref().map(str::trim).unwrap_or_default();
    if !apple_web::redirect_allowed(apple, redirect_uri) {
        return Err(bad("`redirect_uri` is not an app return address this server allows"));
    }
    if params.code_challenge_method.as_deref() != Some("S256") {
        return Err(bad("`code_challenge_method` must be `S256`"));
    }
    let challenge = params.code_challenge.as_deref().unwrap_or_default();
    if !apple_web::challenge_valid(challenge) {
        return Err(bad("`code_challenge` is the base64url of a SHA-256, 43 characters (RFC 7636)"));
    }
    let app_state = params.state.as_deref().filter(|s| !s.is_empty());
    if app_state.is_some_and(|s| s.len() > APP_STATE_MAX) {
        return Err(bad("`state` is at most 1024 characters"));
    }
    let server_state = apple_web::secret();
    let nonce = apple_web::secret();
    db::apple_web::create(
        state.db(),
        &db::apple_web::NewSignIn {
            state_hash: &db::apple_web::hash(&server_state),
            nonce: &nonce,
            code_challenge: challenge,
            redirect_uri,
            app_state,
            expires_at: Utc::now() + chrono::Duration::seconds(apple_web::TTL_SECS),
        },
    )
    .await?;
    let to_apple = apple_web::authorize_url(client_id, &apple_web::return_url(base), &server_state, &nonce);
    Ok(redirect(StatusCode::FOUND, &to_apple))
}

/// Where Apple posts its answer (the Services ID's Return URL): not for apps to call. A sign-in
/// it knows ends in `303` to the app's return address, with a code or an error; an unknown,
/// expired or finished one is `400`, with nowhere to send the browser.
#[utoipa::path(post, path = "/auth/apple/return", tag = "auth",
    extensions(("x-permission" = json!("public"))),
    request_body(content = apple_web::AppleReturn, content_type = "application/x-www-form-urlencoded",
        description = "Apple's form_post"),
    responses(
        (status = 303, description = "To the app: `<redirect_uri>?code=…&state=…`, or `?error=access_denied|server_error&state=…`"),
        (status = 400, description = "`unknown_sign_in`: no started sign-in has this state (unknown, expired after five minutes, or already answered)", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable` (`feature: apple_web`)", body = crate::v2::problem::ProblemBody)))]
pub async fn apple_return(State(state): State<AppState>, Form(answer): Form<apple_web::AppleReturn>) -> Result<axum::response::Response, Problem> {
    let Some((apple, _, _)) = apple_web::configured(state.config()) else {
        return Err(apple_web_off());
    };
    let unknown = || Problem::bad_request("unknown_sign_in", "this sign-in is unknown, expired or already finished; start again from the app");
    let server_state = answer.state.as_deref().filter(|s| !s.is_empty()).ok_or_else(unknown)?;
    let pool = state.db();
    let pending = db::apple_web::take_state(pool, &db::apple_web::hash(server_state)).await?.ok_or_else(unknown)?;
    let back = |pairs: &[(&str, &str)]| apple_web::back_to_app(&pending.redirect_uri, pairs, pending.app_state.as_deref());
    let refused = |error: &str| back(&[("error", error)]);
    let location = match apple_web::outcome(apple, &pending.nonce, &answer).await {
        apple_web::Outcome::Signed(identity) => {
            let code = apple_web::secret();
            let signed = db::apple_web::Signed {
                code_hash: &db::apple_web::hash(&code),
                subject: &identity.subject,
                email: &identity.email,
                email_verified: identity.email_verified,
                full_name: identity.display_name.as_deref(),
                expires_at: Utc::now() + chrono::Duration::seconds(apple_web::TTL_SECS),
            };
            match db::apple_web::sign(pool, pending.id, &signed).await {
                Ok(()) => back(&[("code", &code)]),
                Err(e) => {
                    tracing::error!("apple web sign-in: could not keep the sign-in: {e:#}");
                    refused("server_error")
                }
            }
        }
        outcome => {
            if let Err(e) = db::apple_web::delete(pool, pending.id).await {
                tracing::warn!("apple web sign-in: could not delete a finished sign-in: {e:#}");
            }
            refused(if matches!(outcome, apple_web::Outcome::Denied) { "access_denied" } else { "server_error" })
        }
    };
    Ok(redirect(StatusCode::SEE_OTHER, &location))
}

/// Whether a desktop client setting is there: compose hands an unset one over as empty.
pub(crate) fn desktop_client(value: Option<&str>) -> bool {
    value.is_some_and(|v| !v.trim().is_empty())
}

/// RFC 8693: a live refresh token for a second, independent device chain — a
/// helper that runs apart from the app that signed in (the Mac's Finder
/// extension). Sharing one chain would have the two present each other's
/// rotated tokens, which reads as theft. The presented token is checked like a
/// refresh and keeps working; the new device can be signed out on its own.
async fn token_exchange(state: &AppState, meta: &RequestMeta, body: &TokenRequest) -> Result<LoginResponse, Problem> {
    let subject = body.need(&body.subject_token, "subject_token")?;
    if body.need(&body.subject_token_type, "subject_token_type")? != TOKEN_TYPE_REFRESH {
        return Err(oauth(StatusCode::BAD_REQUEST, "invalid_request", format!("`subject_token_type` is `{TOKEN_TYPE_REFRESH}`")));
    }
    if let Some(wanted) = body.requested_token_type.as_deref().filter(|t| !t.is_empty()) {
        if wanted != TOKEN_TYPE_ACCESS && wanted != TOKEN_TYPE_REFRESH {
            return Err(oauth(StatusCode::BAD_REQUEST, "invalid_request", "`requested_token_type` is an access or a refresh token"));
        }
    }
    let response = crate::auth::fork_session(state, subject, body.device_name.as_deref(), body.device_kind.as_deref())
        .await
        .map_err(oauth_from)?;
    if let Ok(user) = response.user.id.parse::<uuid::Uuid>() {
        let kind = crate::auth::normalize_device_kind(body.device_kind.as_deref());
        audit::record(state, meta, Some(user), None, "session.forked", serde_json::json!({ "device_kind": kind, "device_name": body.device_name })).await;
    }
    Ok(response)
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
