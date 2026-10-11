// SPDX-License-Identifier: AGPL-3.0-or-later
/// Auth module — local credentials, OIDC (Google + Microsoft), session issuance,
/// identity linking.
pub mod at_rest;
pub mod device;
pub mod error;
pub mod middleware;
pub mod oidc;
pub mod password;
pub mod session;
pub mod totp;
pub mod verification;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::auth::middleware::InstanceAdmin;
use crate::auth::session::{SessionClaims, sign};
use crate::db;
use crate::users::models::User;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use axum::routing::get;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

use argon2::{Argon2, PasswordHash, PasswordVerifier};
use argon2::password_hash::{PasswordHasher, SaltString, rand_core::OsRng};
use argon2::password_hash::rand_core::RngCore;

// ── Public API types ──────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    /// Optional device label shown in the session list ("Anna's iPhone").
    #[serde(default)]
    pub device_name: Option<String>,
    /// One of `web|ios|android|other`; anything else is coerced to `other`.
    #[serde(default)]
    pub device_kind: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
    /// A family invite code. Presenting a valid one authorizes registration
    /// even when open self-registration is disabled, and joins the new
    /// account to the inviting family.
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct GoogleSignInRequest {
    /// One-shot ID token from a client SDK that already completed the OAuth
    /// dance itself. Mutually exclusive with `code` — provide exactly one.
    #[serde(default)]
    pub id_token: Option<String>,
    /// Authorization code from the native loopback+PKCE flow (the Mac app's
    /// path). Requires `code_verifier` and `redirect_uri`.
    #[serde(default)]
    pub code: Option<String>,
    #[serde(default)]
    pub code_verifier: Option<String>,
    #[serde(default)]
    pub redirect_uri: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_kind: Option<String>,
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AppleSignInRequest {
    pub identity_token: String,
    /// The user's name, sent by the client only on their first-ever
    /// authorization (Apple's own privacy rule, not ours) — `None` on every
    /// later sign-in.
    #[serde(default)]
    pub full_name: Option<String>,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_kind: Option<String>,
    #[serde(default)]
    pub invite_code: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ForgotPasswordRequest {
    pub email: String,
}

#[derive(Deserialize, ToSchema)]
pub struct ResetPasswordRequest {
    /// The secret from the mailed link.
    pub token: String,
    pub password: String,
}

#[derive(Serialize, ToSchema)]
pub struct LoginResponse {
    pub token: String,
    /// Long-lived, single-use rotating token for `POST /auth/refresh`.
    pub refresh_token: String,
    pub user: UserInfo,
}

#[derive(Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceSessionResponse {
    pub chain_id: String,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub signed_in_at: String,
    pub last_used_at: Option<String>,
    pub expires_at: String,
    /// True when this is the device making the request.
    pub current: bool,
}

#[derive(Serialize, ToSchema)]
pub struct UserInfo {
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub role: String,
    /// False until the user turns recommendations on. Carried on every
    /// auth response so a client knows before it renders anything —
    /// fetching `/users/me` separately would mean a frame where the app
    /// does not yet know whether it may look at listening history.
    pub recommendations_enabled: bool,
    /// Whether the address was proven (`auth::verification`). False only on
    /// a server that mails confirmation links, until the link is used; the
    /// console then shows a banner with "resend".
    pub email_verified: bool,
    /// Base subtags (`en`, `cs`) podcast discovery answers in; empty means
    /// every language. Carried on sign-in so a client can show the setting
    /// without a second call.
    pub discovery_languages: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct RegistrationStatusResponse {
    pub registration_open: bool,
}

#[derive(Serialize, ToSchema)]
pub struct ProvidersResponse {
    pub local: bool,
    pub google: GoogleProviderInfo,
    pub apple: AppleProviderInfo,
    pub microsoft: MicrosoftProviderInfo,
}

/// Shaped like [`GoogleProviderInfo`]: the desktop client id is all a native
/// client needs, because this side holds the secret and does the exchange.
#[derive(Serialize, ToSchema)]
pub struct MicrosoftProviderInfo {
    pub enabled: bool,
    pub desktop_client_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct GoogleProviderInfo {
    pub enabled: bool,
    /// The OAuth client ID native clients use for the loopback+PKCE flow.
    /// `null` while Google sign-in is unconfigured.
    pub desktop_client_id: Option<String>,
    /// The web console's client id, `null` while Google sign-in is off.
    pub web_client_id: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct AppleProviderInfo {
    pub enabled: bool,
    /// Services ID for Sign in with Apple JS; `None` until configured.
    pub web_client_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AdminCreateUserRequest {
    pub email: String,
    pub password: String,
    pub display_name: String,
    #[serde(default = "default_role")]
    pub role: String,
}

fn default_role() -> String {
    "user".to_string()
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router(limits: &crate::http::rate_limit::Limiters) -> OpenApiRouter<AppState> {
    use crate::http::openapi::map;
    OpenApiRouter::new()
        .routes(map(routes!(login), |m| limits.login.apply(m)))
        .routes(map(routes!(register), |m| limits.login.apply(m)))
        .routes(routes!(registration_status))
        .routes(routes!(providers))
        .routes(map(routes!(google_sign_in), |m| limits.login.apply(m)))
        .routes(map(routes!(microsoft_sign_in), |m| limits.login.apply(m)))
        .routes(map(routes!(apple_sign_in), |m| limits.login.apply(m)))
        .routes(map(routes!(refresh), |m| limits.refresh.apply(m)))
        .routes(routes!(fork))
        .routes(routes!(logout))
        // Signing in on a TV: the device asks for a code, someone signed in approves it.
        .routes(map(routes!(device::start), |m| limits.device.apply(m)))
        .routes(map(routes!(device::poll), |m| limits.device.apply(m)))
        .routes(map(routes!(device::describe), |m| limits.device.apply(m)))
        .routes(routes!(device::approve))
        .routes(routes!(device::deny))
        .routes(routes!(me))
        .routes(routes!(change_password))
        .routes(map(routes!(forgot_password), |m| limits.login.apply(m)))
        .routes(map(routes!(reset_password), |m| limits.login.apply(m)))
        .routes(map(routes!(verification::verify_email), |m| limits.login.apply(m)))
        .routes(map(routes!(verification::resend_verification), |m| limits.login.apply(m)))
        .routes(routes!(totp::status, totp::disable))
        .routes(routes!(totp::setup))
        .routes(routes!(totp::enable))
        .routes(map(routes!(totp::verify), |m| limits.login.apply(m)))
        .routes(routes!(list_sessions))
        .routes(routes!(delete_session))
        .routes(routes!(admin_create_user))
        // OIDC stubs — implemented later
        .route("/google/redirect", get(|| async { "TODO: Google OIDC redirect" }))
        .route("/google/callback", get(|| async { "TODO: Google OIDC callback" }))
        .route("/microsoft/redirect", get(|| async { "TODO: Microsoft OIDC redirect" }))
        .route("/microsoft/callback", get(|| async { "TODO: Microsoft OIDC callback" }))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// POST /api/v1/auth/login
/// Body: { "email": "...", "password": "..." }
/// Returns: { "token": "<JWT>", "user": { ... } }
#[utoipa::path(post, path = "/login", tag = "auth",
    request_body = LoginRequest,
    responses(
        (status = 200, body = LoginResponse),
        (status = 202, description = "The password is right and the account has two-factor sign-in on: finish with `POST /auth/totp/verify`", body = totp::MfaChallenge),
        (status = 401, description = "Unknown account, inactive account or wrong password — deliberately not told apart", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited, or too many wrong passwords for this email (`account_locked`); see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
async fn login(
    State(state): State<AppState>,
    Json(body): Json<LoginRequest>,
) -> Result<totp::SignIn, AuthError> {
    let pool = state.db();

    // Wrong passwords in a row lock the email for a growing while, whether
    // or not it has an account — so the lock says nothing about which
    // emails exist (security hardening plan §5.1). The published demo
    // account is exempt: anyone could lock it for everyone else.
    let email_lc = body.email.trim().to_lowercase();
    let lockable = !crate::demo::is_read_only_account(&state, &email_lc);
    if lockable {
        if let Some(secs) = db::login_failures::locked_for_secs(pool, &email_lc).await.map_err(AuthError::Internal)? {
            return Err(AuthError::Locked { retry_after_secs: secs.max(1) as u64 });
        }
    }

    let user = match verify_password_login(pool, &body).await {
        Ok(user) => user,
        Err(AuthError::InvalidCredentials) if lockable => {
            let failures = db::login_failures::record(pool, &email_lc, password::lock_after)
                .await
                .map_err(AuthError::Internal)?;
            if failures == password::LOCK_AT {
                // Told once, at the first lock, and only if there is someone to tell.
                if let Ok(Some(owner)) = db::users::find_by_email(pool, &email_lc).await {
                    let mail = state.config().mail.clone();
                    tokio::spawn(async move {
                        crate::mail::lockout::send_lockout_notice(mail.as_ref(), &owner.email, failures).await;
                    });
                }
            }
            return Err(AuthError::InvalidCredentials);
        }
        Err(e) => return Err(e),
    };
    if lockable {
        if let Err(e) = db::login_failures::clear(pool, &email_lc).await {
            tracing::warn!(error = %e, "could not clear login failures");
        }
    }

    // Tokens (a new device chain), or the second factor first.
    totp::finish_sign_in(&state, user, body.device_name.as_deref(), body.device_kind.as_deref(), StatusCode::OK).await
}

/// How long a mailed reset link works.
const PASSWORD_RESET_MINUTES: i64 = 30;

/// Whether this server can mail reset links: mail set up and a console
/// address to link to. Reported as `features.auth.password_reset`.
pub fn password_reset_offered(cfg: &crate::app::AppConfig) -> bool {
    cfg.mail.as_ref().is_some_and(|m| m.smtp_host().is_some()) && cfg.server.web_base().is_some()
}

/// POST /api/v1/auth/password/forgot — mails a single-use link, good for
/// thirty minutes, to an email that has an active account with a password.
/// The answer is `200` whatever the email, so it cannot be used to learn
/// which emails have accounts. Security hardening plan §5.1.
#[utoipa::path(post, path = "/password/forgot", tag = "auth",
    request_body = ForgotPasswordRequest,
    responses(
        (status = 200, description = "Accepted. A link is on its way if the email has an account that can be reset; the answer is the same either way", body = Object),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
async fn forgot_password(
    State(state): State<AppState>,
    Json(body): Json<ForgotPasswordRequest>,
) -> Result<Json<serde_json::Value>, AuthError> {
    let cfg = state.config();
    let email = body.email.trim().to_lowercase();
    let Some(web_base) = cfg.server.web_base().filter(|_| password_reset_offered(cfg)) else {
        tracing::info!("password reset asked for, but mail or the console address is not configured");
        return Ok(Json(serde_json::json!({})));
    };
    if !email.contains('@') {
        return Ok(Json(serde_json::json!({})));
    }
    let pool = state.db();
    let Some(user) = db::users::find_by_email_ci(pool, &email).await.map_err(AuthError::Internal)? else {
        return Ok(Json(serde_json::json!({})));
    };
    let has_password = user.is_active
        && db::users::find_identity_for_local(pool, user.id)
            .await
            .map_err(AuthError::Internal)?
            .is_some_and(|i| i.password_hash.is_some());
    if !has_password {
        return Ok(Json(serde_json::json!({})));
    }
    // 256-bit secret in the link; only its SHA-256 is stored, like a refresh token.
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    let expires = chrono::Utc::now() + chrono::Duration::minutes(PASSWORD_RESET_MINUTES);
    db::password_resets::insert(pool, user.id, &db::refresh_tokens::hash_token(&token), expires)
        .await
        .map_err(AuthError::Internal)?;
    let url = format!("{}/reset-password?token={token}", web_base.trim_end_matches('/'));
    let mail = cfg.mail.clone();
    let to = user.email.clone();
    tokio::spawn(async move {
        crate::mail::password_reset::send_password_reset(mail.as_ref(), &to, &url, PASSWORD_RESET_MINUTES).await;
    });
    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/auth/password/reset — sets the password behind a mailed
/// link and signs every session of the account out, this one included:
/// whoever asked for the link signs in again with the new password.
#[utoipa::path(post, path = "/password/reset", tag = "auth",
    request_body = ResetPasswordRequest,
    responses(
        (status = 200, description = "Password set; sign in again", body = Object),
        (status = 400, description = "Link invalid, used or expired, or password under 12 characters", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
async fn reset_password(
    State(state): State<AppState>,
    Json(body): Json<ResetPasswordRequest>,
) -> Result<Json<serde_json::Value>, AuthError> {
    password::check(&body.password)?;
    let pool = state.db();
    let token = body.token.trim();
    let invalid = || AuthError::BadRequest("this reset link is not valid any more; ask for a new one".into());
    if token.is_empty() || token.len() > 128 {
        return Err(invalid());
    }
    let Some(user_id) = db::password_resets::take(pool, &db::refresh_tokens::hash_token(token))
        .await
        .map_err(AuthError::Internal)?
    else {
        return Err(invalid());
    };
    let salt = SaltString::generate(&mut OsRng);
    let hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("password hash failed: {e}")))?
        .to_string();
    db::users::update_password(pool, user_id, &hash).await.map_err(AuthError::Internal)?;
    db::sessions::revoke_all_for_user(pool, user_id).await.map_err(AuthError::Internal)?;
    db::refresh_tokens::revoke_all_for_user(pool, user_id, None).await.map_err(AuthError::Internal)?;
    if let Ok(Some(user)) = db::users::find_by_id(pool, user_id).await {
        let _ = db::login_failures::clear(pool, &user.email.trim().to_lowercase()).await;
    }
    Ok(Json(serde_json::json!({})))
}

/// The account behind an email and password, or `InvalidCredentials` — for an
/// unknown email, an inactive account, no local identity and a wrong
/// password alike, deliberately not told apart.
async fn verify_password_login(pool: &sqlx::PgPool, body: &LoginRequest) -> Result<User, AuthError> {
    let user = db::users::find_by_email(pool, &body.email)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;
    if !user.is_active {
        return Err(AuthError::InvalidCredentials);
    }
    let identity = db::users::find_identity_for_local(pool, user.id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;
    let hash_str = identity.password_hash.ok_or(AuthError::InvalidCredentials)?;
    let parsed_hash = PasswordHash::new(&hash_str).map_err(|_| AuthError::InvalidCredentials)?;
    Argon2::default()
        .verify_password(body.password.as_bytes(), &parsed_hash)
        .map_err(|_| AuthError::InvalidCredentials)?;
    Ok(user)
}

/// Keep in sync with `playback::normalize_device_kind`, which gates the same values for progress
/// saves and session reports. The two lists differ on purpose: `subsonic` is a playback source,
/// never a login. Each value also has to be in the CHECK constraints (migration 0065).
pub(crate) fn normalize_device_kind(value: Option<&str>) -> &str {
    match value.map(str::trim) {
        Some(k @ ("web" | "ios" | "android" | "macos" | "windows" | "tvos")) => k,
        _ => "other",
    }
}

/// A freshly minted access JWT + refresh token secret.
pub(crate) struct IssuedTokens {
    access_token: String,
    refresh_token: String,
}

/// Mint an access JWT and a refresh token. `existing_chain` continues a device
/// chain (refresh rotation); `None` starts a new one (login/register).
pub(crate) async fn issue_tokens(
    state: &AppState,
    user: &User,
    device_name: Option<&str>,
    device_kind: Option<&str>,
    existing_chain: Option<Uuid>,
) -> Result<IssuedTokens, AuthError> {
    let pool = state.db();
    let cfg = state.config();

    let chain_id = existing_chain.unwrap_or_else(Uuid::new_v4);

    // 256-bit random secret; only its SHA-256 is stored.
    let mut secret_bytes = [0u8; 32];
    OsRng.fill_bytes(&mut secret_bytes);
    let refresh_secret: String = secret_bytes.iter().map(|b| format!("{b:02x}")).collect();

    let device_kind = normalize_device_kind(device_kind);
    let refresh_expires =
        chrono::Utc::now() + chrono::Duration::seconds(cfg.auth.refresh_ttl_secs as i64);

    // A new chain from a device unlike any the account had in the last 90
    // days is news for the owner (security hardening plan §5.2) — decided
    // before this chain is recorded. Not for the published demo account.
    let mail_configured = cfg.mail.as_ref().is_some_and(|m| m.smtp_host().is_some());
    let notify = existing_chain.is_none()
        && mail_configured
        && !crate::demo::is_read_only_account(state, &user.email)
        && !db::refresh_tokens::device_is_familiar(pool, user.id, device_kind, device_name, 90)
            .await
            .map_err(AuthError::Internal)?;

    db::refresh_tokens::create(
        pool,
        user.id,
        chain_id,
        &db::refresh_tokens::hash_token(&refresh_secret),
        device_name,
        device_kind,
        refresh_expires,
    )
    .await
    .map_err(AuthError::Internal)?;

    if notify {
        let mail = cfg.mail.clone();
        let to = user.email.clone();
        let device = format!("{} ({device_kind})", device_name.filter(|n| !n.trim().is_empty()).unwrap_or("Unknown device"));
        let when = chrono::Utc::now().format("%Y-%m-%d %H:%M UTC").to_string();
        tokio::spawn(async move {
            crate::mail::new_device::send_new_device_notice(mail.as_ref(), &to, &device, &when).await;
        });
    }

    let claims =
        SessionClaims::new(user.id, &user.role, cfg.auth.access_ttl()).with_chain(chain_id);
    let access_token = sign(&claims, &cfg.auth.session_secret).map_err(AuthError::Internal)?;

    db::sessions::insert_with_chain(
        pool,
        claims.jti,
        user.id,
        claims_expires_at(&claims),
        None,
        None,
        Some(chain_id),
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(IssuedTokens {
        access_token,
        refresh_token: refresh_secret,
    })
}

pub(crate) fn login_response(issued: IssuedTokens, user: User) -> LoginResponse {
    LoginResponse {
        token: issued.access_token,
        refresh_token: issued.refresh_token,
        user: UserInfo {
            id: user.id.to_string(),
            email: user.email,
            display_name: user.display_name,
            role: user.role,
            recommendations_enabled: user.recommendations_enabled,
            email_verified: user.email_verified_at.is_some(),
            discovery_languages: user.discovery_languages.clone(),
        },
    }
}

/// POST /api/v1/auth/refresh
/// Body: { "refresh_token": "..." }
/// Rotates the refresh token and returns a fresh access JWT. Presenting an
/// already-rotated token is treated as theft: the whole chain is revoked.
#[utoipa::path(post, path = "/refresh", tag = "auth",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "A new access token and the next refresh token", body = LoginResponse),
        (status = 401, description = "Unknown, expired or already-used refresh token (reuse revokes the device's chain), or inactive account", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
async fn refresh(
    State(state): State<AppState>,
    Json(body): Json<RefreshRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    let pool = state.db();

    let hash = db::refresh_tokens::hash_token(&body.refresh_token);
    let stored = db::refresh_tokens::find_by_hash(pool, &hash)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;

    // Reuse of a rotated/revoked token ⇒ assume the token leaked; kill the
    // whole device chain so neither party keeps access.
    if stored.revoked_at.is_some() || stored.replaced_by.is_some() {
        db::refresh_tokens::revoke_chain(pool, stored.chain_id, stored.user_id)
            .await
            .map_err(AuthError::Internal)?;
        return Err(AuthError::InvalidCredentials);
    }

    if stored.expires_at <= chrono::Utc::now() {
        return Err(AuthError::InvalidCredentials);
    }

    let user = db::users::find_by_id(pool, stored.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;

    if !user.is_active {
        return Err(AuthError::InvalidCredentials);
    }

    let issued = issue_tokens(
        &state,
        &user,
        stored.device_name.as_deref(),
        Some(&stored.device_kind),
        Some(stored.chain_id),
    )
    .await?;

    // Retire the presented token. If a concurrent request beat us to it,
    // treat it as reuse and revoke the chain.
    let new_hash = db::refresh_tokens::hash_token(&issued.refresh_token);
    let new_row = db::refresh_tokens::find_by_hash(pool, &new_hash)
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("fresh refresh token vanished")))?;

    let rotated = db::refresh_tokens::mark_rotated(pool, stored.id, new_row.id)
        .await
        .map_err(AuthError::Internal)?;

    if !rotated {
        db::refresh_tokens::revoke_chain(pool, stored.chain_id, stored.user_id)
            .await
            .map_err(AuthError::Internal)?;
        return Err(AuthError::InvalidCredentials);
    }

    Ok(Json(login_response(issued, user)))
}

#[derive(Deserialize, ToSchema)]
pub struct ForkRequest {
    pub refresh_token: String,
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_kind: Option<String>,
}

/// POST /api/v1/auth/fork
/// Body: { "refresh_token": "...", "device_name": "...", "device_kind": "..." }
///
/// A second, independent session for the same account, for a helper that
/// runs apart from the app that signed in — the Mac's Finder extension
/// (docs/file-sync-plan.md §7.1). Both then refresh on their own; sharing one
/// chain would have them present each other's rotated tokens, which reads as
/// theft and revokes it. The presented token is checked like a refresh but not
/// rotated. Holding a refresh token already means holding the account, so this
/// grants nothing new; the new device shows in the session list and can be
/// signed out on its own.
#[utoipa::path(post, path = "/fork", tag = "auth",
    request_body = ForkRequest,
    responses(
        (status = 200, description = "Tokens for a new, independent device session", body = LoginResponse),
        (status = 401, description = "Unknown, expired or already-used refresh token, or inactive account", body = crate::http::openapi::ErrorBody)))]
async fn fork(
    State(state): State<AppState>,
    Json(body): Json<ForkRequest>,
) -> Result<Json<LoginResponse>, AuthError> {
    let pool = state.db();
    let hash = db::refresh_tokens::hash_token(&body.refresh_token);
    let stored = db::refresh_tokens::find_by_hash(pool, &hash)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;
    if stored.revoked_at.is_some() || stored.replaced_by.is_some() {
        db::refresh_tokens::revoke_chain(pool, stored.chain_id, stored.user_id)
            .await
            .map_err(AuthError::Internal)?;
        return Err(AuthError::InvalidCredentials);
    }
    if stored.expires_at <= chrono::Utc::now() {
        return Err(AuthError::InvalidCredentials);
    }
    let user = db::users::find_by_id(pool, stored.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;
    if !user.is_active {
        return Err(AuthError::InvalidCredentials);
    }
    let issued = issue_tokens(&state, &user, body.device_name.as_deref(), body.device_kind.as_deref(), None).await?;
    Ok(Json(login_response(issued, user)))
}

/// GET /api/v1/auth/sessions — the caller's signed-in devices.
#[utoipa::path(get, path = "/sessions", tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = Vec<DeviceSessionResponse>), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn list_sessions(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<DeviceSessionResponse>>, AuthError> {
    let devices = db::refresh_tokens::list_devices(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        devices
            .into_iter()
            .map(|d| DeviceSessionResponse {
                current: auth.chain_id == Some(d.chain_id),
                chain_id: d.chain_id.to_string(),
                device_name: d.device_name,
                device_kind: d.device_kind,
                signed_in_at: d.signed_in_at.to_rfc3339(),
                last_used_at: d.last_used_at.map(|t| t.to_rfc3339()),
                expires_at: d.expires_at.to_rfc3339(),
            })
            .collect(),
    ))
}

/// DELETE /api/v1/auth/sessions/{chain_id} — sign out one device.
#[utoipa::path(delete, path = "/sessions/{chain_id}", tag = "auth", security(("bearer" = [])),
    params(("chain_id" = Uuid, Path, description = "The device's `chain_id` from `GET /auth/sessions`")),
    responses((status = 204, description = "Signed out (also when the chain was unknown)"), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn delete_session(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(chain_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::refresh_tokens::revoke_chain(state.db(), chain_id, auth.user_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/auth/logout
/// Header: Authorization: Bearer <token>
/// Signs out this device: revokes the session and, when the token belongs to
/// a refresh chain, the whole chain (so the refresh token dies too).
#[utoipa::path(post, path = "/logout", tag = "auth", security(("bearer" = [])),
    responses((status = 204, description = "Signed out"), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn logout(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<StatusCode, AuthError> {
    let _ = db::sessions::revoke(state.db(), auth.session_id).await;
    if let Some(chain_id) = auth.chain_id {
        let _ = db::refresh_tokens::revoke_chain(state.db(), chain_id, auth.user_id).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/auth/me
/// Header: Authorization: Bearer <token>
#[utoipa::path(get, path = "/me", tag = "auth", security(("bearer" = [])),
    responses((status = 200, body = UserInfo), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
async fn me(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<UserInfo>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::SessionInvalid)?;

    Ok(Json(UserInfo {
        id: user.id.to_string(),
        email: user.email,
        display_name: user.display_name,
        role: user.role,
        recommendations_enabled: user.recommendations_enabled,
        email_verified: user.email_verified_at.is_some(),
        discovery_languages: user.discovery_languages.clone(),
    }))
}

/// GET /api/v1/auth/registration-status
/// Public — no auth required. Returns whether self-registration is enabled.
#[utoipa::path(get, path = "/registration-status", tag = "auth",
    responses((status = 200, body = RegistrationStatusResponse)))]
async fn registration_status(
    State(state): State<AppState>,
) -> Json<RegistrationStatusResponse> {
    Json(RegistrationStatusResponse {
        registration_open: state.config().auth.registration_open,
    })
}

/// GET /api/v1/auth/providers
/// Public — no auth required. Tells clients which sign-in methods are live
/// so SSO buttons can be shown/hidden without a client rebuild when a
/// provider is configured later. See docs/sso-payments-plan.md.
#[utoipa::path(get, path = "/providers", tag = "auth",
    responses((status = 200, body = ProvidersResponse)))]
async fn providers(State(state): State<AppState>) -> Json<ProvidersResponse> {
    Json(providers_response(&state.config().auth))
}

/// An unset client id arrives as `Some("")`, not `None`: docker-compose maps
/// these variables explicitly and defaults them to empty. Clients check for
/// null to decide whether to offer a button, so collapse the two here rather
/// than handing every client an empty string to special-case.
fn non_empty(value: Option<&str>) -> Option<String> {
    value.map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
}

pub(crate) fn providers_response(auth: &crate::app::AuthConfig) -> ProvidersResponse {
    // `enabled` on the config struct is a deliberate second gate on top of "configured at all" —
    // see `GoogleAuthConfig::enabled`'s doc comment. A provider with credentials set but
    // `enabled: false` must report exactly like an unconfigured one here.
    let google = auth.google.as_ref().filter(|g| g.enabled);
    let apple = auth.apple.as_ref().filter(|a| a.enabled);
    let microsoft = auth.microsoft.as_ref().filter(|m| m.enabled);
    ProvidersResponse {
        local: auth.local_enabled,
        google: GoogleProviderInfo {
            enabled: google.is_some(),
            desktop_client_id: google.and_then(|g| non_empty(g.desktop_client_id.as_deref())),
            web_client_id: google.and_then(|g| non_empty(g.web_client_id.as_deref())),
        },
        apple: AppleProviderInfo {
            enabled: apple.is_some(),
            web_client_id: apple.and_then(|a| non_empty(a.web_client_id.as_deref())),
        },
        microsoft: MicrosoftProviderInfo {
            enabled: microsoft.is_some(),
            desktop_client_id: microsoft.and_then(|m| non_empty(m.desktop_client_id.as_deref())),
        },
    }
}

/// POST /api/v1/auth/register
/// Body: { "email": "...", "password": "...", "display_name": "..." }
#[utoipa::path(post, path = "/register", tag = "auth",
    request_body = RegisterRequest,
    responses(
        (status = 201, description = "Account created and signed in", body = LoginResponse),
        (status = 400, description = "Invalid email, password under 12 characters, empty display name, email taken, invalid invite, or registration closed", body = crate::http::openapi::ErrorBody)))]
async fn register(
    State(state): State<AppState>,
    Json(body): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<LoginResponse>), AuthError> {
    let pool = state.db();

    // 1. Validate input up front — an invite is checked against the email, so
    //    it has to be normalized before the authorization decision.
    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AuthError::BadRequest("invalid email address".into()));
    }

    // 2. Authorize: either open registration, or a valid invite for this email.
    let invite = authorize_new_account(&state, &email, body.invite_code.as_deref()).await?;

    password::check(&body.password)?;
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(AuthError::BadRequest("display name is required".into()));
    }

    // 3. Check if email already exists
    if db::users::find_by_email(pool, &email)
        .await
        .map_err(AuthError::Internal)?
        .is_some()
    {
        return Err(AuthError::BadRequest(
            "an account with this email already exists".into(),
        ));
    }

    // 4. Hash password
    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("password hash failed: {e}")))?
        .to_string();

    // 5. Create user + local identity
    let user = db::users::insert(pool, state.at_rest(), &email, &display_name, "user")
        .await
        .map_err(AuthError::Internal)?;

    db::users::insert_local_identity(pool, user.id, &password_hash)
        .await
        .map_err(AuthError::Internal)?;

    // 6. Place the account in a family: the inviting one, or its own.
    let proven = invite_proves(invite.as_ref(), &email);
    let family_id = place_in_family(&state, user.id, invite).await?;
    verification::born(&state, &user, family_id, proven).await?;

    // 7. Auto-login: mint access + refresh tokens
    let issued = issue_tokens(&state, &user, None, None, None).await?;

    Ok((StatusCode::CREATED, Json(login_response(issued, user))))
}

/// POST /api/v1/auth/google
/// Body: either `{ "id_token": "..." }` (a client SDK already holds a token)
/// or `{ "code", "code_verifier", "redirect_uri" }` (the Mac app's native
/// loopback+PKCE flow — the backend exchanges the code with Google, which is
/// the only place `desktop_client_secret` is ever used).
/// `501` while Google sign-in is unconfigured. See docs/sso-payments-plan.md.
#[utoipa::path(post, path = "/google", tag = "auth",
    request_body = GoogleSignInRequest,
    responses(
        (status = 200, description = "Signed in to an existing or newly linked account", body = LoginResponse),
        (status = 201, description = "A new account was created", body = LoginResponse),
        (status = 202, description = "The account has two-factor sign-in on: finish with `POST /auth/totp/verify`", body = totp::MfaChallenge),
        (status = 400, description = "Missing fields, a non-loopback `redirect_uri`, an invalid invite, or registration closed", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "The provider token did not verify, or the account is inactive", body = crate::http::openapi::ErrorBody),
        (status = 409, description = "An account with this email exists and the provider did not verify the address", body = crate::http::openapi::ErrorBody),
        (status = 501, description = "This provider is not configured", body = crate::http::openapi::ErrorBody)))]
async fn google_sign_in(
    State(state): State<AppState>,
    Json(body): Json<GoogleSignInRequest>,
) -> Result<totp::SignIn, AuthError> {
    let cfg = state
        .config()
        .auth
        .google
        .as_ref()
        .filter(|g| g.enabled)
        .ok_or(AuthError::ProviderNotConfigured)?;

    let id_token = match (&body.id_token, &body.code) {
        (Some(token), _) => token.clone(),
        (None, Some(code)) => {
            let code_verifier = body
                .code_verifier
                .as_deref()
                .ok_or_else(|| AuthError::BadRequest("code_verifier is required".into()))?;
            let redirect_uri = body
                .redirect_uri
                .as_deref()
                .ok_or_else(|| AuthError::BadRequest("redirect_uri is required".into()))?;
            if !is_loopback_redirect(redirect_uri) {
                return Err(AuthError::BadRequest(
                    "redirect_uri must be a loopback address".into(),
                ));
            }
            oidc::exchange_google_code(cfg, code, code_verifier, redirect_uri).await?
        }
        (None, None) => {
            return Err(AuthError::BadRequest("id_token or code is required".into()));
        }
    };

    let identity = oidc::verify_google_id_token(cfg, &id_token).await?;
    sso_sign_in(
        &state,
        "google",
        identity,
        body.device_name.as_deref(),
        body.device_kind.as_deref(),
        body.invite_code.as_deref(),
    )
    .await
}

/// POST /api/v1/auth/microsoft
///
/// The same shape as [`google_sign_in`] and for the same reason: the client runs
/// loopback + PKCE, gets a `code`, and posts it here because the client secret
/// lives on this side. Only the verification differs — see
/// [`oidc::verify_microsoft_id_token`], whose issuer check cannot be a constant.
#[utoipa::path(post, path = "/microsoft", tag = "auth",
    request_body = GoogleSignInRequest,
    responses(
        (status = 200, description = "Signed in to an existing or newly linked account", body = LoginResponse),
        (status = 201, description = "A new account was created", body = LoginResponse),
        (status = 202, description = "The account has two-factor sign-in on: finish with `POST /auth/totp/verify`", body = totp::MfaChallenge),
        (status = 400, description = "Missing fields, a non-loopback `redirect_uri`, an invalid invite, or registration closed", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "The provider token did not verify, or the account is inactive", body = crate::http::openapi::ErrorBody),
        (status = 409, description = "An account with this email exists and the provider did not verify the address", body = crate::http::openapi::ErrorBody),
        (status = 501, description = "This provider is not configured", body = crate::http::openapi::ErrorBody)))]
async fn microsoft_sign_in(
    State(state): State<AppState>,
    Json(body): Json<GoogleSignInRequest>,
) -> Result<totp::SignIn, AuthError> {
    let cfg = state
        .config()
        .auth
        .microsoft
        .as_ref()
        .filter(|m| m.enabled)
        .ok_or(AuthError::ProviderNotConfigured)?;

    let id_token = match (&body.id_token, &body.code) {
        (Some(token), _) => token.clone(),
        (None, Some(code)) => {
            let code_verifier = body
                .code_verifier
                .as_deref()
                .ok_or_else(|| AuthError::BadRequest("code_verifier is required".into()))?;
            let redirect_uri = body
                .redirect_uri
                .as_deref()
                .ok_or_else(|| AuthError::BadRequest("redirect_uri is required".into()))?;
            if !is_loopback_redirect(redirect_uri) {
                return Err(AuthError::BadRequest(
                    "redirect_uri must be a loopback address".into(),
                ));
            }
            oidc::exchange_microsoft_code(cfg, code, code_verifier, redirect_uri).await?
        }
        (None, None) => {
            return Err(AuthError::BadRequest("id_token or code is required".into()));
        }
    };

    let identity = oidc::verify_microsoft_id_token(cfg, &id_token).await?;
    sso_sign_in(
        &state,
        "microsoft",
        identity,
        body.device_name.as_deref(),
        body.device_kind.as_deref(),
        body.invite_code.as_deref(),
    )
    .await
}

/// POST /api/v1/auth/apple
/// Body: `{ "identity_token": "...", "full_name"?: "..." }`. `501` while
/// Sign in with Apple is unconfigured. See docs/sso-payments-plan.md.
#[utoipa::path(post, path = "/apple", tag = "auth",
    request_body = AppleSignInRequest,
    responses(
        (status = 200, description = "Signed in to an existing or newly linked account", body = LoginResponse),
        (status = 201, description = "A new account was created", body = LoginResponse),
        (status = 202, description = "The account has two-factor sign-in on: finish with `POST /auth/totp/verify`", body = totp::MfaChallenge),
        (status = 400, description = "Missing fields, a non-loopback `redirect_uri`, an invalid invite, or registration closed", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "The provider token did not verify, or the account is inactive", body = crate::http::openapi::ErrorBody),
        (status = 409, description = "An account with this email exists and the provider did not verify the address", body = crate::http::openapi::ErrorBody),
        (status = 501, description = "This provider is not configured", body = crate::http::openapi::ErrorBody)))]
async fn apple_sign_in(
    State(state): State<AppState>,
    Json(body): Json<AppleSignInRequest>,
) -> Result<totp::SignIn, AuthError> {
    let cfg = state
        .config()
        .auth
        .apple
        .as_ref()
        .filter(|a| a.enabled)
        .ok_or(AuthError::ProviderNotConfigured)?;

    let mut identity = oidc::verify_apple_id_token(cfg, &body.identity_token).await?;
    if let Some(full_name) = body.full_name.as_deref().map(str::trim).filter(|n| !n.is_empty()) {
        identity.display_name = Some(full_name.to_string());
    }

    sso_sign_in(
        &state,
        "apple",
        identity,
        body.device_name.as_deref(),
        body.device_kind.as_deref(),
        body.invite_code.as_deref(),
    )
    .await
}

/// Shared by [`google_sign_in`] and [`apple_sign_in`]: find an existing
/// identity, link a new identity to an existing account by verified email,
/// or create a fresh account under the same authorization gate as
/// `/auth/register`.
async fn sso_sign_in(
    state: &AppState,
    provider: &str,
    identity: oidc::ProviderIdentity,
    device_name: Option<&str>,
    device_kind: Option<&str>,
    invite_code: Option<&str>,
) -> Result<totp::SignIn, AuthError> {
    let pool = state.db();

    // 1. Already linked — sign in.
    if let Some(existing) = db::users::find_identity_by_provider(pool, provider, &identity.subject)
        .await
        .map_err(AuthError::Internal)?
    {
        let user = db::users::find_by_id(pool, existing.user_id)
            .await
            .map_err(AuthError::Internal)?
            .ok_or(AuthError::InvalidCredentials)?;
        if !user.is_active {
            return Err(AuthError::InvalidCredentials);
        }
        return totp::finish_sign_in(state, user, device_name, device_kind, StatusCode::OK).await;
    }

    let email = identity.email.trim().to_lowercase();
    if email.is_empty() {
        return Err(AuthError::BadRequest(format!(
            "{provider} account has no email address"
        )));
    }

    // 2. Link to an existing account by verified email.
    if let Some(user) = db::users::find_by_email(pool, &email)
        .await
        .map_err(AuthError::Internal)?
    {
        if !identity.email_verified {
            // A same-address account exists but this provider hasn't proven
            // ownership of the address — refuse the silent link rather than
            // letting an attacker with a spoofable "email" claim take over
            // someone else's account.
            return Err(AuthError::IdentityConflict);
        }
        if !user.is_active {
            return Err(AuthError::InvalidCredentials);
        }
        db::users::insert_oidc_identity(pool, user.id, provider, &identity.subject)
            .await
            .map_err(AuthError::Internal)?;
        return totp::finish_sign_in(state, user, device_name, device_kind, StatusCode::OK).await;
    }

    // 3. No match — create a fresh account, gated exactly like /auth/register.
    let invite = authorize_new_account(state, &email, invite_code).await?;
    let display_name = identity
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| email.split('@').next().unwrap_or(&email).to_string());

    let user = db::users::insert(pool, state.at_rest(), &email, &display_name, "user")
        .await
        .map_err(AuthError::Internal)?;
    db::users::insert_oidc_identity(pool, user.id, provider, &identity.subject)
        .await
        .map_err(AuthError::Internal)?;
    let proven = identity.email_verified || invite_proves(invite.as_ref(), &email);
    let family_id = place_in_family(state, user.id, invite).await?;
    verification::born(state, &user, family_id, proven).await?;

    let issued = issue_tokens(state, &user, device_name, device_kind, None).await?;
    Ok(totp::SignIn::Tokens(StatusCode::CREATED, login_response(issued, user)))
}

/// Authorize creating a new account: either open registration, or a valid
/// invite for `email`. Shared by `register` and `sso_sign_in`.
async fn authorize_new_account(
    state: &AppState,
    email: &str,
    invite_code: Option<&str>,
) -> Result<Option<db::families::FamilyInvite>, AuthError> {
    match invite_code.map(str::trim).filter(|c| !c.is_empty()) {
        Some(code) => Ok(Some(crate::families::redeem_invite(state, code, email).await?)),
        None => {
            if !state.config().auth.registration_open {
                return Err(AuthError::BadRequest(
                    "registration is not open — contact an administrator".into(),
                ));
            }
            Ok(None)
        }
    }
}

/// Place a freshly created account into a family: the inviting one, or its
/// own (the install's one, with [`crate::hooks::Hooks::one_family`]). Shared by `register` and `sso_sign_in`.
async fn place_in_family(
    state: &AppState,
    user_id: Uuid,
    invite: Option<db::families::FamilyInvite>,
) -> Result<Uuid, AuthError> {
    match invite {
        Some(invite) => {
            crate::families::claim_invite(state, invite.id, user_id).await?;
            db::families::move_to_family(state.db(), user_id, invite.family_id, &invite.role)
                .await
                .map_err(AuthError::Internal)?;
            Ok(invite.family_id)
        }
        None => {
            let membership = db::families::home_new_user(state.db(), user_id, state.hooks().one_family())
                .await
                .map_err(AuthError::Internal)?;
            state
                .hooks()
                .user_created(state.db(), membership.family_id, user_id)
                .await
                .map_err(AuthError::Internal)?;
            Ok(membership.family_id)
        }
    }
}

/// An invite mailed to this very address proves it; a link invite or open
/// registration proves nothing.
fn invite_proves(invite: Option<&db::families::FamilyInvite>, email: &str) -> bool {
    invite.is_some_and(|i| i.kind == "email" && i.email.as_deref().is_some_and(|e| e.eq_ignore_ascii_case(email)))
}

/// Only loopback redirect URIs are accepted for the Google authorization-code
/// exchange — the backend is the confidential client here, so this exists to
/// keep the code-exchange endpoint from being pointed at an arbitrary host,
/// not to protect a secret the request already carries.
fn is_loopback_redirect(uri: &str) -> bool {
    if !uri.starts_with("http://") {
        return false;
    }
    let Ok(parsed) = url::Url::parse(uri) else {
        return false;
    };
    match parsed.host() {
        Some(url::Host::Ipv4(addr)) => addr.is_loopback(),
        Some(url::Host::Ipv6(addr)) => addr.is_loopback(),
        Some(url::Host::Domain(d)) => d == "localhost",
        None => false,
    }
}

/// POST /api/v1/auth/password
/// Body: { "current_password": "...", "new_password": "..." }
/// Requires authentication.
#[utoipa::path(post, path = "/password", tag = "auth", security(("bearer" = [])),
    request_body = ChangePasswordRequest,
    responses(
        (status = 204, description = "Changed; every other session and device is signed out"),
        (status = 400, description = "New password under 12 characters, or current password incorrect", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Invalid access token, or the account has no password", body = crate::http::openapi::ErrorBody)))]
async fn change_password(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<ChangePasswordRequest>,
) -> Result<StatusCode, AuthError> {
    let pool = state.db();

    // 1. Validate new password
    password::check(&body.new_password)?;

    // 2. Verify current password
    let identity = db::users::find_identity_for_local(pool, auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::InvalidCredentials)?;

    let hash_str = identity.password_hash.ok_or(AuthError::InvalidCredentials)?;
    let parsed_hash =
        PasswordHash::new(&hash_str).map_err(|_| AuthError::InvalidCredentials)?;

    Argon2::default()
        .verify_password(body.current_password.as_bytes(), &parsed_hash)
        .map_err(|_| AuthError::BadRequest("current password is incorrect".into()))?;

    // 3. Hash new password and save
    let salt = SaltString::generate(&mut OsRng);
    let new_hash = Argon2::default()
        .hash_password(body.new_password.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("password hash failed: {e}")))?
        .to_string();

    db::users::update_password(pool, auth.user_id, &new_hash)
        .await
        .map_err(AuthError::Internal)?;

    // 4. Revoke all other sessions and refresh chains (keep this device)
    db::sessions::revoke_all_except(pool, auth.user_id, auth.session_id)
        .await
        .map_err(AuthError::Internal)?;
    db::refresh_tokens::revoke_all_for_user(pool, auth.user_id, auth.chain_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/auth/admin-create-user
/// Admin-only endpoint to create a user account directly (bypasses registration_open flag).
#[utoipa::path(post, path = "/admin-create-user", tag = "auth", security(("bearer" = [])),
    request_body = AdminCreateUserRequest,
    responses(
        (status = 201, body = UserInfo),
        (status = 400, description = "Invalid email, password under 12 characters, empty display name, or email in use", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody),
        (status = 403, description = "Caller is not an admin", body = crate::http::openapi::ErrorBody)))]
async fn admin_create_user(
    InstanceAdmin(_auth): InstanceAdmin,
    State(state): State<AppState>,
    Json(body): Json<AdminCreateUserRequest>,
) -> Result<(StatusCode, Json<UserInfo>), AuthError> {
    let pool = state.db();

    let email = body.email.trim().to_lowercase();
    if email.is_empty() || !email.contains('@') {
        return Err(AuthError::BadRequest("invalid email address".into()));
    }
    password::check(&body.password)?;
    let display_name = body.display_name.trim().to_string();
    if display_name.is_empty() {
        return Err(AuthError::BadRequest("display name is required".into()));
    }
    let role = if body.role == "admin" { "admin" } else { "user" };

    if db::users::find_by_email(pool, &email)
        .await
        .map_err(AuthError::Internal)?
        .is_some()
    {
        return Err(AuthError::BadRequest("email already in use".into()));
    }

    let salt = SaltString::generate(&mut OsRng);
    let password_hash = Argon2::default()
        .hash_password(body.password.as_bytes(), &salt)
        .map_err(|e| AuthError::Internal(anyhow::anyhow!("password hash failed: {e}")))?
        .to_string();

    let user = db::users::insert(pool, state.at_rest(), &email, &display_name, role)
        .await
        .map_err(AuthError::Internal)?;

    db::users::insert_local_identity(pool, user.id, &password_hash)
        .await
        .map_err(AuthError::Internal)?;

    let membership = db::families::home_new_user(pool, user.id, state.hooks().one_family())
        .await
        .map_err(AuthError::Internal)?;
    state
        .hooks()
        .user_created(pool, membership.family_id, user.id)
        .await
        .map_err(AuthError::Internal)?;
    // The admin typed the address: that vouches for it.
    verification::born(&state, &user, membership.family_id, true).await?;

    Ok((
        StatusCode::CREATED,
        Json(UserInfo {
            id: user.id.to_string(),
            email: user.email,
            display_name: user.display_name,
            role: user.role,
            recommendations_enabled: user.recommendations_enabled,
            email_verified: user.email_verified_at.is_some(),
            discovery_languages: user.discovery_languages.clone(),
        }),
    ))
}

// ── Helpers ───────────────────────────────────────────────────────────────

fn claims_expires_at(claims: &SessionClaims) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(claims.exp, 0)
        .unwrap_or_else(chrono::Utc::now)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unset_client_id_is_reported_as_null_not_an_empty_string() {
        // docker-compose maps these explicitly and defaults them to empty, so
        // Some("") is the shape that actually reaches us.
        assert_eq!(non_empty(Some("")), None);
        assert_eq!(non_empty(Some("   ")), None);
        assert_eq!(non_empty(None), None);
    }

    #[test]
    fn a_real_client_id_survives_untouched() {
        assert_eq!(
            non_empty(Some("123-abc.apps.googleusercontent.com")),
            Some("123-abc.apps.googleusercontent.com".to_string())
        );
    }
    use crate::app::AuthConfig;

    fn base_auth_config() -> AuthConfig {
        AuthConfig {
            session_secret: "test-secret".into(),
            session_ttl_secs: None,
            access_ttl_secs: None,
            refresh_ttl_secs: 7_776_000,
            local_enabled: true,
            registration_open: false,
            dev_seed_admin: false,
            google: None,
            apple: None,
            microsoft: None,
        }
    }

    #[test]
    fn providers_all_disabled_by_default() {
        let resp = providers_response(&base_auth_config());
        assert!(resp.local);
        assert!(!resp.google.enabled);
        assert!(resp.google.desktop_client_id.is_none());
        assert!(!resp.apple.enabled);
    }

    #[test]
    fn providers_reflects_configured_google() {
        let mut cfg = base_auth_config();
        cfg.google = Some(crate::app::config::GoogleAuthConfig {
            enabled: true,
            client_ids: "abc.apps.googleusercontent.com".into(),
            desktop_client_id: Some("desktop-id".into()),
            desktop_client_secret: Some("desktop-secret".into()),
            web_client_id: None,
        });
        let resp = providers_response(&cfg);
        assert!(resp.google.enabled);
        assert_eq!(resp.google.desktop_client_id.as_deref(), Some("desktop-id"));
    }

    #[test]
    fn providers_reflects_configured_apple() {
        let mut cfg = base_auth_config();
        cfg.apple = Some(crate::app::config::AppleAuthConfig {
            enabled: true,
            client_ids: "audio.own.mac,audio.own.web".into(),
            web_client_id: Some("audio.own.web".into()),
        });
        let resp = providers_response(&cfg);
        assert!(resp.apple.enabled);
        assert_eq!(resp.apple.web_client_id.as_deref(), Some("audio.own.web"));
    }

    /// The whole point of `enabled`: credentials can be configured ahead of the public rollout
    /// (for real end-to-end testing) without going live for real users until it's flipped.
    #[test]
    fn providers_stay_hidden_when_configured_but_disabled() {
        let mut cfg = base_auth_config();
        cfg.google = Some(crate::app::config::GoogleAuthConfig {
            enabled: false,
            client_ids: "abc.apps.googleusercontent.com".into(),
            desktop_client_id: Some("desktop-id".into()),
            desktop_client_secret: Some("desktop-secret".into()),
            web_client_id: None,
        });
        cfg.apple = Some(crate::app::config::AppleAuthConfig {
            enabled: false,
            client_ids: "audio.own.mac".into(),
            web_client_id: None,
        });
        let resp = providers_response(&cfg);
        assert!(!resp.google.enabled, "credentials being set must not imply enabled");
        assert!(resp.google.desktop_client_id.is_none(), "no client id leaks while disabled");
        assert!(!resp.apple.enabled);
    }

    #[test]
    fn loopback_redirect_accepts_127_0_0_1_and_localhost() {
        assert!(is_loopback_redirect("http://127.0.0.1:51234"));
        assert!(is_loopback_redirect("http://localhost:51234/"));
        assert!(is_loopback_redirect("http://[::1]:51234"));
    }

    #[test]
    fn loopback_redirect_rejects_https_and_external_hosts() {
        assert!(!is_loopback_redirect("https://127.0.0.1:51234"));
        assert!(!is_loopback_redirect("http://evil.example.com"));
        assert!(!is_loopback_redirect("http://127.0.0.1.evil.example.com"));
        assert!(!is_loopback_redirect("not a url"));
    }
}
