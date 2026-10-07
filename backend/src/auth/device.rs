// SPDX-License-Identifier: AGPL-3.0-or-later
//! Signing in on a device with no comfortable keyboard.
//!
//! The Apple TV asks for a code, shows it, and polls. Someone already signed in — on a phone or
//! the web — looks the code up and approves it, and the TV gets the same tokens a password login
//! would have produced. This is RFC 8628 in shape (device code, user code, polling with
//! `authorization_pending` / `slow_down`), not to the letter: both ends are our own.
//!
//! It is also how an account that can only sign in with Google gets onto a TV at all, since the
//! Google SDK doesn't exist on tvOS.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::auth::{issue_tokens, login_response};
use crate::db;
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Ten minutes: long enough to fetch a phone from the next room, short enough that a code left
/// on screen stops being useful.
const REQUEST_TTL_SECS: i64 = 600;
/// What the client is told to wait between polls, and what `slow_down` enforces.
pub const POLL_INTERVAL_SECS: i64 = 5;

/// No 0/O, 1/I/L or 5/S: the whole point is reading it off a TV across a room and typing it
/// somewhere else.
const CODE_ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRTUVWXYZ2346789";
const CODE_LENGTH: usize = 8;
/// A hyphen in the middle, so eight characters read as two chunks.
const CODE_GROUP: usize = 4;

#[derive(Deserialize, ToSchema)]
pub struct StartRequest {
    #[serde(default)]
    pub device_name: Option<String>,
    #[serde(default)]
    pub device_kind: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct StartResponse {
    /// The polling credential. Never shown to anyone; only its hash is stored.
    pub device_code: String,
    /// The short code shown on the TV and typed by the approver.
    pub user_code: String,
    /// Where to approve it — shown under the code, and what the QR encodes. Empty when the
    /// server has no `base_url` configured.
    pub verification_url: String,
    pub expires_in: i64,
    pub interval: i64,
}

#[derive(Deserialize, ToSchema)]
pub struct PollRequest {
    pub device_code: String,
}

#[derive(Serialize, ToSchema)]
pub struct PendingResponse {
    /// `authorization_pending` | `slow_down` | `denied` | `expired`, as in RFC 8628.
    pub status: &'static str,
}

#[derive(Serialize, ToSchema)]
pub struct DeviceRequestInfo {
    pub user_code: String,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub expires_at: String,
}

/// A fresh device code (a 256-bit secret) and the short user code that goes with it.
pub fn generate_codes() -> (String, String) {
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let device_code: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    (device_code, generate_user_code())
}

fn generate_user_code() -> String {
    let mut bytes = [0u8; CODE_LENGTH];
    OsRng.fill_bytes(&mut bytes);
    let code: String = bytes
        .iter()
        .map(|b| CODE_ALPHABET[*b as usize % CODE_ALPHABET.len()] as char)
        .collect();
    format_user_code(&code)
}

/// `ABCD-EFGH`. Also the shape the lookup normalizes to, so what someone types matches what the
/// TV showed however they typed it.
pub fn format_user_code(code: &str) -> String {
    let cleaned = normalize_user_code(code);
    if cleaned.len() <= CODE_GROUP {
        return cleaned;
    }
    let (head, tail) = cleaned.split_at(CODE_GROUP);
    format!("{head}-{tail}")
}

/// Upper-cased, hyphens and spaces dropped, and the characters people reliably confuse folded
/// onto the ones in the alphabet: someone reading "0" for "O" off a TV should still get in.
pub fn normalize_user_code(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| match c.to_ascii_uppercase() {
            'O' => '0',
            'I' | 'L' => '1',
            'S' => '5',
            other => other,
        })
        .map(|c| match c {
            // …and then back onto the alphabet, which has none of the ambiguous pair members.
            '0' => 'Q',
            '1' => 'J',
            '5' => 'Z',
            other => other,
        })
        .collect()
}

/// What a poll should answer, given the stored request. Pure, so the state machine is testable
/// without a database.
#[derive(Debug, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Approved,
}

pub fn poll_outcome(
    status: &str,
    expired: bool,
    secs_since_last_poll: Option<i64>,
    interval_secs: i64,
) -> PollOutcome {
    // An expired request is expired whatever its status says — except one already approved and
    // consumed, which is simply gone as far as the client is concerned.
    match status {
        "denied" => return PollOutcome::Denied,
        "consumed" => return PollOutcome::Expired,
        _ => {}
    }
    if expired {
        return PollOutcome::Expired;
    }
    if status == "approved" {
        return PollOutcome::Approved;
    }
    match secs_since_last_poll {
        Some(elapsed) if elapsed < interval_secs => PollOutcome::SlowDown,
        _ => PollOutcome::Pending,
    }
}

/// POST /api/v1/auth/device/start — public. The TV asks for a code to show.
#[utoipa::path(post, path = "/device/start", tag = "auth",
    request_body = StartRequest,
    responses((status = 201, body = StartResponse), (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
pub async fn start(
    State(state): State<AppState>,
    Json(body): Json<StartRequest>,
) -> Result<(StatusCode, Json<StartResponse>), AuthError> {
    let (device_code, user_code) = generate_codes();
    let device_kind = crate::auth::normalize_device_kind(body.device_kind.as_deref());
    let expires_at = Utc::now() + chrono::Duration::seconds(REQUEST_TTL_SECS);

    db::device_auth::create(
        state.db(),
        &db::device_auth::hash_code(&device_code),
        &user_code,
        body.device_name.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        device_kind,
        expires_at,
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok((
        StatusCode::CREATED,
        Json(StartResponse {
            device_code,
            user_code,
            // The web console's own host, the same source the family-invite links use. Without
            // it configured the TV still works — it just can't print a URL under the code.
            verification_url: state
                .config()
                .server
                .web_base()
                .map(|base| format!("{}/link", base.trim_end_matches('/')))
                .unwrap_or_default(),
            expires_in: REQUEST_TTL_SECS,
            interval: POLL_INTERVAL_SECS,
        }),
    ))
}

/// POST /api/v1/auth/device/poll — public. The TV asks whether it has been let in yet.
#[utoipa::path(post, path = "/device/poll", tag = "auth",
    request_body = PollRequest,
    responses(
        (status = 200, description = "Approved: the device is signed in", body = crate::auth::LoginResponse),
        (status = 202, description = "Not signed in (yet): `authorization_pending`, `slow_down`, `denied` or `expired`", body = PendingResponse),
        (status = 400, description = "Unknown device code (answered `expired`)", body = crate::http::openapi::ErrorBody),
        (status = 401, description = "The approving account no longer exists or is inactive", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
pub async fn poll(
    State(state): State<AppState>,
    Json(body): Json<PollRequest>,
) -> Result<axum::response::Response, AuthError> {
    use axum::response::IntoResponse;

    let pool = state.db();
    let request = db::device_auth::find_by_code_hash(pool, &db::device_auth::hash_code(&body.device_code))
        .await
        .map_err(AuthError::Internal)?
        // An unknown device code is "expired" rather than "not found": nothing here should help
        // anyone work out which codes exist.
        .ok_or_else(|| AuthError::BadRequest("expired".into()))?;

    let now = Utc::now();
    let elapsed = request.last_polled_at.map(|last| (now - last).num_seconds());
    let outcome = poll_outcome(&request.status, request.expired(now), elapsed, POLL_INTERVAL_SECS);
    db::device_auth::touch_poll(pool, request.id, now)
        .await
        .map_err(AuthError::Internal)?;

    let pending = |status: &'static str| {
        Ok((StatusCode::ACCEPTED, Json(PendingResponse { status })).into_response())
    };
    match outcome {
        PollOutcome::Pending => pending("authorization_pending"),
        PollOutcome::SlowDown => pending("slow_down"),
        PollOutcome::Denied => pending("denied"),
        PollOutcome::Expired => pending("expired"),
        PollOutcome::Approved => {
            let user_id = request
                .approved_user_id
                .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("approved request has no user")))?;
            // Exactly once, however many pollers race: the loser is told the code is gone.
            if !db::device_auth::consume(pool, request.id)
                .await
                .map_err(AuthError::Internal)?
            {
                return pending("expired");
            }
            let user = db::users::find_by_id(pool, user_id)
                .await
                .map_err(AuthError::Internal)?
                .ok_or(AuthError::NotFound)?;
            if !user.is_active {
                return Err(AuthError::InvalidCredentials);
            }
            let issued = issue_tokens(
                &state,
                &user,
                request.device_name.as_deref(),
                Some(request.device_kind.as_str()),
                None,
            )
            .await?;
            Ok(Json(login_response(issued, user)).into_response())
        }
    }
}

/// GET /api/v1/auth/device/{user_code} — authenticated. What the approver is being asked to let in.
#[utoipa::path(get, path = "/device/{user_code}", tag = "auth", security(("bearer" = [])),
    params(("user_code" = String, Path, description = "The code shown on the TV; case, hyphens and look-alike characters are forgiven")),
    responses((status = 200, body = DeviceRequestInfo), (status = 400, description = "The code is unknown, expired or already used", body = crate::http::openapi::ErrorBody), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody), (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
pub async fn describe(
    _auth: AuthUser,
    State(state): State<AppState>,
    Path(user_code): Path<String>,
) -> Result<Json<DeviceRequestInfo>, AuthError> {
    let request = db::device_auth::find_pending_by_user_code(state.db(), &format_user_code(&user_code))
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("that code is not valid any more".into()))?;

    Ok(Json(DeviceRequestInfo {
        user_code: request.user_code,
        device_name: request.device_name,
        device_kind: request.device_kind,
        expires_at: request.expires_at.to_rfc3339(),
    }))
}

/// POST /api/v1/auth/device/{user_code}/approve — authenticated. The TV is signed in as *this*
/// account, which is why no password is involved: possession of a signed-in session is the proof.
#[utoipa::path(post, path = "/device/{user_code}/approve", tag = "auth", security(("bearer" = [])),
    params(("user_code" = String, Path, description = "The code shown on the TV; case, hyphens and look-alike characters are forgiven")),
    responses((status = 204, description = "Approved; the device's next poll signs it in"), (status = 400, description = "The code is unknown, expired or already used", body = crate::http::openapi::ErrorBody), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
pub async fn approve(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(user_code): Path<String>,
) -> Result<StatusCode, AuthError> {
    resolve(auth, state, user_code, "approved").await
}

/// POST /api/v1/auth/device/{user_code}/deny — authenticated.
#[utoipa::path(post, path = "/device/{user_code}/deny", tag = "auth", security(("bearer" = [])),
    params(("user_code" = String, Path, description = "The code shown on the TV; case, hyphens and look-alike characters are forgiven")),
    responses((status = 204, description = "Denied"), (status = 400, description = "The code is unknown, expired or already used", body = crate::http::openapi::ErrorBody), (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
pub async fn deny(
    auth: AuthUser,
    State(state): State<AppState>,
    Path(user_code): Path<String>,
) -> Result<StatusCode, AuthError> {
    resolve(auth, state, user_code, "denied").await
}

async fn resolve(
    auth: AuthUser,
    state: AppState,
    user_code: String,
    status: &str,
) -> Result<StatusCode, AuthError> {
    let pool = state.db();
    let request = db::device_auth::find_pending_by_user_code(pool, &format_user_code(&user_code))
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::BadRequest("that code is not valid any more".into()))?;

    let approved_user = (status == "approved").then_some(auth.user_id);
    if !db::device_auth::resolve(pool, request.id, status, approved_user)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::BadRequest("that code is not valid any more".into()));
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_codes_avoid_characters_people_confuse() {
        for _ in 0..200 {
            let (_, code) = generate_codes();
            assert_eq!(code.len(), CODE_LENGTH + 1, "{code} should be ABCD-EFGH");
            assert_eq!(&code[CODE_GROUP..CODE_GROUP + 1], "-");
            for c in code.chars().filter(|c| *c != '-') {
                assert!(
                    CODE_ALPHABET.contains(&(c as u8)),
                    "{c} is not in the alphabet — 0/O, 1/I/L and 5/S are excluded on purpose"
                );
            }
        }
    }

    #[test]
    fn device_codes_are_long_random_secrets() {
        let (first, _) = generate_codes();
        let (second, _) = generate_codes();
        assert_eq!(first.len(), 64);
        assert_ne!(first, second);
    }

    #[test]
    fn typing_a_code_the_wrong_way_still_matches() {
        let code = format_user_code("QJZ4-N7PX");
        assert_eq!(normalize_user_code("qjz4 n7px"), normalize_user_code(&code));
        // Read off a TV: O for Q's neighbour 0, l for J's 1, s for Z's 5.
        assert_eq!(normalize_user_code("0JZ4N7PX"), normalize_user_code(&code));
        assert_eq!(normalize_user_code("QlZ4-N7PX"), "QJZ4N7PX");
        assert_eq!(normalize_user_code("QJs4-N7PX"), "QJZ4N7PX");
    }

    #[test]
    fn a_pending_request_polls_as_pending_and_then_asks_for_patience() {
        assert_eq!(poll_outcome("pending", false, None, 5), PollOutcome::Pending);
        assert_eq!(poll_outcome("pending", false, Some(9), 5), PollOutcome::Pending);
        assert_eq!(poll_outcome("pending", false, Some(1), 5), PollOutcome::SlowDown);
    }

    #[test]
    fn approval_beats_the_poll_interval_but_not_expiry() {
        assert_eq!(poll_outcome("approved", false, Some(0), 5), PollOutcome::Approved);
        assert_eq!(poll_outcome("approved", true, None, 5), PollOutcome::Expired);
    }

    #[test]
    fn denial_and_reuse_are_final() {
        assert_eq!(poll_outcome("denied", false, None, 5), PollOutcome::Denied);
        // Already swapped for tokens: a second poll must not be told to keep waiting.
        assert_eq!(poll_outcome("consumed", false, None, 5), PollOutcome::Expired);
        assert_eq!(poll_outcome("pending", true, None, 5), PollOutcome::Expired);
    }
}
