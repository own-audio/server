// SPDX-License-Identifier: AGPL-3.0-or-later
//! Two-factor sign-in with an authenticator app (RFC 6238 TOTP: SHA-1,
//! 30-second steps, six digits, one step of drift either way) and recovery
//! codes. Security hardening plan §5.1; optional per user.
//!
//! Setting up: `POST /auth/totp/setup` hands out a fresh secret (and the
//! `otpauth://` URI the app scans); `POST /auth/totp/enable` with the first
//! code proves the app has it, switches it on and returns the recovery codes
//! — shown once, stored as hashes. Signing in: the password (or provider)
//! step answers `202` with a short-lived `mfa_token` instead of tokens, and
//! `POST /auth/totp/verify` with the code (or a recovery code) finishes it.
//! A code is accepted once: the step it belongs to is remembered.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::auth::audit::{self, RequestMeta};
use crate::auth::{issue_tokens, login_response, LoginResponse};
use crate::db;
use crate::users::models::User;
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::extract::{Json, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha1::Sha1;
use utoipa::ToSchema;
use uuid::Uuid;

const STEP_SECS: u64 = 30;
const DIGITS: u32 = 1_000_000;
const CHALLENGE_SECS: i64 = 300;
const RECOVERY_CODES: usize = 8;
const ISSUER: &str = "own.audio";

// ── The algorithm ────────────────────────────────────────────────────────

/// The six-digit code for one 30-second step.
pub fn code_at(secret: &[u8], step: u64) -> u32 {
    let mut mac = Hmac::<Sha1>::new_from_slice(secret).expect("HMAC takes any key length");
    mac.update(&step.to_be_bytes());
    let h = mac.finalize().into_bytes();
    let off = (h[19] & 0x0f) as usize;
    let bin = ((h[off] as u32 & 0x7f) << 24) | ((h[off + 1] as u32) << 16) | ((h[off + 2] as u32) << 8) | h[off + 3] as u32;
    bin % DIGITS
}

/// The step a typed code belongs to, if it is right for now, a step before
/// or a step after (clocks drift) and later than the last step accepted.
pub fn matching_step(secret: &[u8], typed: &str, now_secs: u64, last_used_step: i64) -> Option<i64> {
    let typed = typed.trim().replace(' ', "");
    let wanted: u32 = typed.parse().ok().filter(|_| typed.len() == 6)?;
    let current = now_secs / STEP_SECS;
    [current.saturating_sub(1), current, current + 1]
        .into_iter()
        .filter(|step| *step as i64 > last_used_step)
        .find(|step| code_at(secret, *step) == wanted)
        .map(|s| s as i64)
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn otpauth_uri(email: &str, secret_b32: &str) -> String {
    let label: String = form_urlencoded::byte_serialize(format!("{ISSUER}:{email}").as_bytes()).collect();
    let issuer: String = form_urlencoded::byte_serialize(ISSUER.as_bytes()).collect();
    format!("otpauth://totp/{label}?secret={secret_b32}&issuer={issuer}&algorithm=SHA1&digits=6&period=30")
}

/// `xxxxx-xxxxx`, from the base32 alphabet without the easily confused letters.
fn recovery_code() -> String {
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut bytes = [0u8; 10];
    OsRng.fill_bytes(&mut bytes);
    let s: String = bytes.iter().map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char).collect();
    format!("{}-{}", &s[..5], &s[5..])
}

fn normalize_recovery(code: &str) -> String {
    code.trim().to_lowercase().replace([' ', '-'], "")
}

fn recovery_hash(code: &str) -> String {
    db::refresh_tokens::hash_token(&format!("recovery:{}", normalize_recovery(code)))
}

// ── Finishing a sign-in ──────────────────────────────────────────────────

/// What a sign-in answers: tokens, or a challenge when the account has
/// two-factor sign-in on.
pub enum SignIn {
    Tokens(StatusCode, LoginResponse),
    Mfa(MfaChallenge),
}

#[derive(Serialize, ToSchema)]
pub struct MfaChallenge {
    /// Always `true`; the field a client switches on.
    pub mfa_required: bool,
    /// Presented to `POST /auth/totp/verify` with the code; five minutes.
    pub mfa_token: String,
    pub expires_in_secs: i64,
    /// What finishes it: `totp` (the app's code) or `recovery` (a recovery code).
    pub methods: Vec<&'static str>,
}

impl IntoResponse for SignIn {
    fn into_response(self) -> Response {
        match self {
            SignIn::Tokens(status, body) => (status, Json(body)).into_response(),
            SignIn::Mfa(challenge) => (StatusCode::ACCEPTED, Json(challenge)).into_response(),
        }
    }
}

/// The password (or provider) step is done: tokens, or the second factor.
pub async fn finish_sign_in(
    state: &AppState,
    user: User,
    device_name: Option<&str>,
    device_kind: Option<&str>,
    status: StatusCode,
) -> Result<SignIn, AuthError> {
    let pool = state.db();
    if !db::totp::is_enabled(pool, user.id).await.map_err(AuthError::Internal)? {
        let issued = issue_tokens(state, &user, device_name, device_kind, None).await?;
        return Ok(SignIn::Tokens(status, login_response(issued, user)));
    }
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    let expires = chrono::Utc::now() + chrono::Duration::seconds(CHALLENGE_SECS);
    db::totp::insert_challenge(pool, &db::refresh_tokens::hash_token(&token), user.id, device_name, device_kind, expires)
        .await
        .map_err(AuthError::Internal)?;
    Ok(SignIn::Mfa(MfaChallenge {
        mfa_required: true,
        mfa_token: token,
        expires_in_secs: CHALLENGE_SECS,
        methods: vec!["totp", "recovery"],
    }))
}

// ── Routes ───────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct TotpStatus {
    pub enabled: bool,
    /// Recovery codes not used yet; 0 when two-factor is off.
    pub recovery_codes_left: i64,
}

/// GET /api/v1/auth/totp — whether two-factor sign-in is on for this account.
#[utoipa::path(get, path = "/totp", tag = "auth", security(("bearer" = [])),
    responses(
        (status = 200, body = TotpStatus),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
pub async fn status(auth: AuthUser, State(state): State<AppState>) -> Result<Json<TotpStatus>, AuthError> {
    let enabled = db::totp::is_enabled(state.db(), auth.user_id).await.map_err(AuthError::Internal)?;
    let left = if enabled { db::totp::recovery_codes_left(state.db(), auth.user_id).await.map_err(AuthError::Internal)? } else { 0 };
    Ok(Json(TotpStatus { enabled, recovery_codes_left: left }))
}

#[derive(Serialize, ToSchema)]
pub struct TotpSetup {
    /// Base32, for typing into an app by hand.
    pub secret: String,
    /// For the QR code the app scans.
    pub otpauth_uri: String,
}

/// POST /api/v1/auth/totp/setup — a fresh secret, pending until the first
/// code confirms it. `409` while two-factor is already on (turn it off first).
#[utoipa::path(post, path = "/totp/setup", tag = "auth", security(("bearer" = [])),
    responses(
        (status = 200, body = TotpSetup),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody),
        (status = 409, description = "Two-factor sign-in is already on", body = crate::http::openapi::ErrorBody)))]
pub async fn setup(auth: AuthUser, State(state): State<AppState>) -> Result<Json<TotpSetup>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::SessionInvalid)?;
    let mut raw = [0u8; 20];
    OsRng.fill_bytes(&mut raw);
    let secret = base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &raw);
    let started = db::totp::start(state.db(), user.id, &state.totp_cipher().seal(&secret))
        .await
        .map_err(AuthError::Internal)?;
    if !started {
        return Err(AuthError::Conflict("two-factor sign-in is already on; turn it off before setting it up again".into()));
    }
    Ok(Json(TotpSetup { otpauth_uri: otpauth_uri(&user.email, &secret), secret }))
}

#[derive(Deserialize, ToSchema)]
pub struct CodeRequest {
    /// The six digits from the app (or, where allowed, a recovery code).
    pub code: String,
}

#[derive(Serialize, ToSchema)]
pub struct RecoveryCodes {
    /// Shown once; each works once. Keep them somewhere safe.
    pub recovery_codes: Vec<String>,
}

/// POST /api/v1/auth/totp/enable — the first code from the app proves it
/// holds the secret; two-factor is switched on and the recovery codes are
/// returned, this once.
#[utoipa::path(post, path = "/totp/enable", tag = "auth", security(("bearer" = [])),
    request_body = CodeRequest,
    responses(
        (status = 200, body = RecoveryCodes),
        (status = 400, description = "No setup pending, or the code is wrong", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
pub async fn enable(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta, Json(body): Json<CodeRequest>) -> Result<Json<RecoveryCodes>, AuthError> {
    let pool = state.db();
    let Some(totp) = db::totp::find(pool, auth.user_id).await.map_err(AuthError::Internal)? else {
        return Err(AuthError::BadRequest("set up two-factor sign-in first".into()));
    };
    if totp.enabled_at.is_some() {
        return Err(AuthError::BadRequest("two-factor sign-in is already on".into()));
    }
    let secret = open_secret(&state, &totp.secret)?;
    let Some(step) = matching_step(&secret, &body.code, now_secs(), totp.last_used_step) else {
        return Err(AuthError::BadRequest("that code is not right; check the app and the time on this device".into()));
    };
    let codes: Vec<String> = (0..RECOVERY_CODES).map(|_| recovery_code()).collect();
    let hashes: Vec<String> = codes.iter().map(|c| recovery_hash(c)).collect();
    db::totp::replace_recovery_codes(pool, auth.user_id, &hashes).await.map_err(AuthError::Internal)?;
    db::totp::enable(pool, auth.user_id, step).await.map_err(AuthError::Internal)?;
    audit::record(&state, &meta, Some(auth.user_id), None, "totp.enabled", serde_json::json!({})).await;
    Ok(Json(RecoveryCodes { recovery_codes: codes }))
}

/// DELETE /api/v1/auth/totp — turns two-factor sign-in off. Needs a current
/// code or a recovery code: a stolen session alone cannot remove it.
#[utoipa::path(delete, path = "/totp", tag = "auth", security(("bearer" = [])),
    request_body = CodeRequest,
    responses(
        (status = 204, description = "Two-factor sign-in is off"),
        (status = 400, description = "Not on, or the code is wrong", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
pub async fn disable(auth: AuthUser, State(state): State<AppState>, meta: RequestMeta, Json(body): Json<CodeRequest>) -> Result<StatusCode, AuthError> {
    let pool = state.db();
    let Some(totp) = db::totp::find(pool, auth.user_id).await.map_err(AuthError::Internal)? else {
        return Err(AuthError::BadRequest("two-factor sign-in is not on".into()));
    };
    if totp.enabled_at.is_none() {
        // A pending setup is simply dropped.
        db::totp::remove(pool, auth.user_id).await.map_err(AuthError::Internal)?;
        return Ok(StatusCode::NO_CONTENT);
    }
    if !check_code(&state, auth.user_id, &totp, &body.code).await? {
        return Err(AuthError::BadRequest("that code is not right".into()));
    }
    db::totp::remove(pool, auth.user_id).await.map_err(AuthError::Internal)?;
    audit::record(&state, &meta, Some(auth.user_id), None, "totp.disabled", serde_json::json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct VerifyRequest {
    /// From the `202` answer of the sign-in call.
    pub mfa_token: String,
    /// The six digits from the app, or a recovery code.
    pub code: String,
}

/// POST /api/v1/auth/totp/verify — the second step of a sign-in.
#[utoipa::path(post, path = "/totp/verify", tag = "auth",
    request_body = VerifyRequest,
    responses(
        (status = 200, description = "Signed in", body = LoginResponse),
        (status = 400, description = "The code is wrong", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "The challenge is unknown, spent or expired: sign in again", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
pub async fn verify(State(state): State<AppState>, meta: RequestMeta, Json(body): Json<VerifyRequest>) -> Result<Json<LoginResponse>, AuthError> {
    let pool = state.db();
    let token_hash = db::refresh_tokens::hash_token(body.mfa_token.trim());
    let Some(challenge) = db::totp::find_challenge(pool, &token_hash).await.map_err(AuthError::Internal)? else {
        return Err(AuthError::SessionInvalid);
    };
    let Some(totp) = db::totp::find(pool, challenge.user_id).await.map_err(AuthError::Internal)?.filter(|t| t.enabled_at.is_some()) else {
        // Switched off between the two steps: nothing to verify against.
        return Err(AuthError::SessionInvalid);
    };
    if !check_code(&state, challenge.user_id, &totp, &body.code).await? {
        audit::record(&state, &meta, Some(challenge.user_id), None, "login.mfa_failed", serde_json::json!({})).await;
        return Err(AuthError::BadRequest("that code is not right".into()));
    }
    if !db::totp::spend_challenge(pool, &token_hash).await.map_err(AuthError::Internal)? {
        return Err(AuthError::SessionInvalid);
    }
    let user = db::users::find_by_id(pool, challenge.user_id)
        .await
        .map_err(AuthError::Internal)?
        .filter(|u| u.is_active)
        .ok_or(AuthError::InvalidCredentials)?;
    let issued = issue_tokens(&state, &user, challenge.device_name.as_deref(), challenge.device_kind.as_deref(), None).await?;
    audit::record(&state, &meta, Some(user.id), None, "login.mfa_ok", serde_json::json!({ "device_kind": challenge.device_kind, "device_name": challenge.device_name })).await;
    Ok(Json(login_response(issued, user)))
}

/// A current code (accepted once) or an unused recovery code.
async fn check_code(state: &AppState, user_id: Uuid, totp: &db::totp::Totp, typed: &str) -> Result<bool, AuthError> {
    let pool = state.db();
    let secret = open_secret(state, &totp.secret)?;
    if let Some(step) = matching_step(&secret, typed, now_secs(), totp.last_used_step) {
        return db::totp::use_step(pool, user_id, step).await.map_err(AuthError::Internal);
    }
    if normalize_recovery(typed).len() == 10 {
        return db::totp::use_recovery_code(pool, user_id, &recovery_hash(typed)).await.map_err(AuthError::Internal);
    }
    Ok(false)
}

fn open_secret(state: &AppState, sealed: &str) -> Result<Vec<u8>, AuthError> {
    let (b32, _) = state.totp_cipher().open(sealed).map_err(AuthError::Internal)?;
    base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &b32)
        .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("stored TOTP secret is not base32")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc_6238_test_vector() {
        // RFC 6238 appendix B, SHA-1, secret "12345678901234567890", T = 59 → step 1.
        let secret = b"12345678901234567890";
        assert_eq!(code_at(secret, 1), 287_082);
        assert_eq!(code_at(secret, 1_111_111_109 / 30), 81_804);
    }

    #[test]
    fn a_code_works_once_and_within_a_step_of_drift() {
        let secret = b"12345678901234567890";
        let now = 59u64;
        let code = format!("{:06}", code_at(secret, 1));
        assert_eq!(matching_step(secret, &code, now, 0), Some(1));
        assert_eq!(matching_step(secret, &code, now + 30, 0), Some(1), "one step late still passes");
        assert_eq!(matching_step(secret, &code, now, 1), None, "already used");
        assert_eq!(matching_step(secret, "000000", now, 0), None);
        assert_eq!(matching_step(secret, "28708", now, 0), None, "six digits or nothing");
    }

    #[test]
    fn recovery_codes_look_right_and_hash_regardless_of_form() {
        let c = recovery_code();
        assert_eq!(c.len(), 11);
        assert_eq!(recovery_hash(&c), recovery_hash(&c.to_uppercase().replace('-', " ")));
    }

    #[test]
    fn otpauth_uri_names_issuer_and_account() {
        let uri = otpauth_uri("a@b.c", "ABC");
        assert!(uri.starts_with("otpauth://totp/own.audio%3Aa%40b.c?secret=ABC&issuer=own.audio"), "{uri}");
    }
}
