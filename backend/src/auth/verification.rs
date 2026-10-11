// SPDX-License-Identifier: AGPL-3.0-or-later
//! Proving that an email address belongs to the account that claims it
//! (security hardening plan §5.1, H6).
//!
//! An account is born verified when something already vouched for the
//! address — a sign-in provider that says so, an invite sent to that very
//! address, the admin who typed it — and otherwise gets a mailed link. A
//! server with no mail, or no console address to link to, has nothing to
//! verify against and takes the address as given: self-hosters' installs are
//! by invite or admin anyway, and locking them out would protect nothing.
//!
//! What verification gates: inviting people (`POST /family/invites`), and
//! whatever the edition hangs on [`Hooks::email_verified`] — the hosted
//! welcome credit, the one thing a stream of fresh registrations could farm.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::users::models::User;
use argon2::password_hash::rand_core::{OsRng, RngCore};
use axum::extract::{Json, State};
use serde::Deserialize;
use utoipa::ToSchema;
use uuid::Uuid;

const LINK_HOURS: i64 = 24;

/// Whether this server mails verification links: mail set up and a console
/// address to link to. Reported as `features.auth.email_verification`.
pub fn offered(cfg: &crate::app::AppConfig) -> bool {
    cfg.mail.as_ref().is_some_and(|m| m.smtp_host().is_some()) && cfg.server.web_base().is_some()
}

/// A new account. `proven` says whether something already vouched for the
/// address; if not, and the server can, a link goes out. Either way the
/// account ends up verified or with a link on its way, never stuck.
pub async fn born(state: &AppState, user: &User, family_id: Uuid, proven: bool) -> Result<(), AuthError> {
    if proven || !offered(state.config()) {
        mark_verified(state, user.id, family_id).await
    } else {
        send_link(state, user).await;
        Ok(())
    }
}

/// Records the proof and tells the edition, once.
async fn mark_verified(state: &AppState, user_id: Uuid, family_id: Uuid) -> Result<(), AuthError> {
    let changed = db::users::mark_email_verified(state.db(), user_id).await.map_err(AuthError::Internal)?;
    if changed {
        state
            .hooks()
            .email_verified(state.db(), family_id, user_id)
            .await
            .map_err(AuthError::Internal)?;
    }
    Ok(())
}

/// Mails a fresh link; only its hash is stored, like a refresh token.
async fn send_link(state: &AppState, user: &User) {
    let cfg = state.config();
    let Some(web_base) = cfg.server.web_base() else { return };
    let mut secret = [0u8; 32];
    OsRng.fill_bytes(&mut secret);
    let token: String = secret.iter().map(|b| format!("{b:02x}")).collect();
    let expires = chrono::Utc::now() + chrono::Duration::hours(LINK_HOURS);
    if let Err(e) = db::email_verifications::insert(state.db(), user.id, &db::refresh_tokens::hash_token(&token), expires).await {
        tracing::warn!(user = %user.id, error = %e, "could not record a verification link");
        return;
    }
    let url = format!("{}/verify-email?token={token}", web_base.trim_end_matches('/'));
    let mail = cfg.mail.clone();
    let to = user.email.clone();
    tokio::spawn(async move {
        crate::mail::verify_email::send_verification(mail.as_ref(), &to, &url, LINK_HOURS).await;
    });
}

#[derive(Deserialize, ToSchema)]
pub struct VerifyEmailRequest {
    /// The secret from the mailed link.
    pub token: String,
}

/// POST /api/v1/auth/email/verify — the mailed link's landing call. Public:
/// the person may open it on a device that is not signed in.
#[utoipa::path(post, path = "/email/verify", tag = "auth",
    request_body = VerifyEmailRequest,
    responses(
        (status = 200, description = "The address is confirmed", body = Object),
        (status = 400, description = "Link invalid, used or expired", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)))]
pub async fn verify_email(
    State(state): State<AppState>,
    meta: crate::auth::audit::RequestMeta,
    Json(body): Json<VerifyEmailRequest>,
) -> Result<Json<serde_json::Value>, AuthError> {
    let token = body.token.trim();
    let invalid = || AuthError::BadRequest("this confirmation link is not valid any more; ask for a new one from the app".into());
    if token.is_empty() || token.len() > 128 {
        return Err(invalid());
    }
    let Some(user_id) = db::email_verifications::take(state.db(), &db::refresh_tokens::hash_token(token))
        .await
        .map_err(AuthError::Internal)?
    else {
        return Err(invalid());
    };
    let Some(membership) = db::families::find_membership(state.db(), user_id).await.map_err(AuthError::Internal)? else {
        return Err(invalid());
    };
    mark_verified(&state, user_id, membership.family_id).await?;
    crate::auth::audit::record(&state, &meta, Some(user_id), None, "email.verified", serde_json::json!({})).await;
    Ok(Json(serde_json::json!({})))
}

/// POST /api/v1/auth/email/resend — another link for the signed-in account.
/// `200` whether or not one went out (already verified, or the server does
/// not mail links); the response says which.
#[utoipa::path(post, path = "/email/resend", tag = "auth",
    responses(
        (status = 200, description = "`sent`: whether a link went out; `verified`: whether the address already is", body = Object),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody),
        (status = 429, description = "Rate limited; see `Retry-After`", body = crate::http::openapi::ErrorBody)),
    security(("bearer" = [])))]
pub async fn resend_verification(auth: AuthUser, State(state): State<AppState>) -> Result<Json<serde_json::Value>, AuthError> {
    let user = db::users::find_by_id(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::SessionInvalid)?;
    if user.email_verified_at.is_some() {
        return Ok(Json(serde_json::json!({ "sent": false, "verified": true })));
    }
    if !offered(state.config()) {
        return Ok(Json(serde_json::json!({ "sent": false, "verified": false })));
    }
    send_link(&state, &user).await;
    Ok(Json(serde_json::json!({ "sent": true, "verified": false })))
}

/// For routes that need a proven address: `403 email_unverified` otherwise.
pub async fn require_verified(state: &AppState, user_id: Uuid) -> Result<(), AuthError> {
    let user = db::users::find_by_id(state.db(), user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::SessionInvalid)?;
    if user.email_verified_at.is_none() {
        return Err(AuthError::EmailUnverified);
    }
    Ok(())
}
