// SPDX-License-Identifier: AGPL-3.0-or-later
//! The security audit trail (security hardening plan §9): one row per event
//! that changes who can get into an account or what they may do — sign-ins
//! (and failures), passwords, two-factor, sessions, roles, accounts. Kept a
//! year, readable by the account's owner (`GET /auth/security-events`).
//!
//! Recording is best effort and never fails the request it describes: a
//! sign-in that worked is not undone because the trail could not be
//! written. Nothing secret goes in `detail` — kinds and names, never
//! passwords, codes or tokens.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use crate::db::security_events::{NewEvent, SecurityEvent};
use axum::extract::{ConnectInfo, FromRequestParts, Json, State};
use axum::http::request::Parts;
use axum::http::{header, HeaderMap};
use std::net::SocketAddr;
use uuid::Uuid;

pub const RETENTION_DAYS: i64 = 365;

/// Where a request came from, for the trail: the client address (through
/// proxy headers, as the rate limiter sees it) and the user agent.
#[derive(Clone, Debug, Default)]
pub struct RequestMeta {
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

impl RequestMeta {
    pub fn from_parts(headers: &HeaderMap, peer: Option<SocketAddr>, trust_proxy_headers: bool) -> Self {
        let ip = crate::http::rate_limit::client_ip(peer.map(|p| p.ip()), headers, trust_proxy_headers).to_string();
        let user_agent = headers
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .map(|v| v.chars().take(200).collect());
        Self { ip: Some(ip), user_agent }
    }
}

impl FromRequestParts<AppState> for RequestMeta {
    type Rejection = std::convert::Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let peer = parts.extensions.get::<ConnectInfo<SocketAddr>>().map(|c| c.0);
        Ok(Self::from_parts(&parts.headers, peer, state.config().server.rate_limit.trust_proxy_headers))
    }
}

/// Records an event about `user_id`; `actor` when someone else did it.
pub async fn record(state: &AppState, meta: &RequestMeta, user_id: Option<Uuid>, actor: Option<Uuid>, kind: &str, detail: serde_json::Value) {
    let event = NewEvent {
        user_id,
        actor_id: actor.filter(|a| Some(*a) != user_id),
        kind,
        detail,
        ip: meta.ip.clone(),
        user_agent: meta.user_agent.clone(),
    };
    if let Err(e) = db::security_events::insert(state.db(), event).await {
        tracing::warn!(kind, error = %e, "security event not recorded");
    }
}

/// GET /api/v1/auth/security-events — the account's own trail, newest first.
#[utoipa::path(get, path = "/security-events", tag = "auth", security(("bearer" = [])),
    responses(
        (status = 200, description = "The last 100 events about this account", body = Vec<SecurityEvent>),
        (status = 401, description = "Missing, invalid or revoked access token", body = crate::http::openapi::ErrorBody)))]
pub async fn list_own(auth: AuthUser, State(state): State<AppState>) -> Result<Json<Vec<SecurityEvent>>, AuthError> {
    let events = db::security_events::list_for_user(state.db(), auth.user_id, 100)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(events))
}
