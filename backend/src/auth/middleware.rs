// SPDX-License-Identifier: AGPL-3.0-or-later
/// Axum extractor that authenticates a request from `Authorization: Bearer <jwt>`.
///
/// Use it in any handler:
/// ```ignore
/// async fn my_handler(auth: AuthUser, ...) -> impl IntoResponse { ... }
/// ```
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::session::{SessionClaims, verify};
use crate::db;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::header;
use uuid::Uuid;

/// Authenticated user extracted from the JWT in `Authorization: Bearer`.
#[derive(Debug, Clone)]
pub struct AuthUser {
    pub user_id: Uuid,
    pub session_id: Uuid,
    pub role: String,
    /// Refresh-token chain (device) of this session, when known.
    pub chain_id: Option<Uuid>,
}

impl FromRequestParts<AppState> for AuthUser {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let secret = &state.config().auth.session_secret;

        let token = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
            .ok_or(AuthError::SessionInvalid)?;

        let claims: SessionClaims = verify(token, secret)?;

        // A signed JWT is not enough: the server-side session row may have
        // been revoked (logout, device sign-out, admin revoke, password
        // change on another device).
        if db::sessions::is_revoked(state.db(), claims.jti)
            .await
            .map_err(AuthError::Internal)?
        {
            return Err(AuthError::SessionInvalid);
        }

        if crate::demo::refuses(state, &parts.method, parts.uri.path(), claims.sub).await {
            return Err(AuthError::DemoReadOnly);
        }

        Ok(AuthUser {
            user_id: claims.sub,
            session_id: claims.jti,
            role: claims.role,
            chain_id: claims.chain,
        })
    }
}
