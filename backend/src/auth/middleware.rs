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
    /// A personal access token's scopes; `None` for a signed-in session, which
    /// has every scope its account has.
    pub scopes: Option<Vec<String>>,
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

        if token.starts_with(db::personal_tokens::PREFIX) {
            return personal_token(parts, state, token).await;
        }

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

        if crate::demo::refuses(state, &parts.method, &original_path(parts), claims.sub).await {
            return Err(AuthError::DemoReadOnly);
        }

        Ok(AuthUser {
            user_id: claims.sub,
            session_id: claims.jti,
            role: claims.role,
            chain_id: claims.chain,
            scopes: None,
        })
    }
}

/// The path as the client sent it: a nested router sees it without its prefix
/// (`/playback/sessions` for `/api/v1/playback/sessions`).
fn original_path(parts: &Parts) -> String {
    parts
        .extensions
        .get::<axum::extract::OriginalUri>()
        .map_or_else(|| parts.uri.path().to_string(), |u| u.path().to_string())
}

/// A personal access token (docs/api-v2-plan.md, V2): good on `/api/v2` only,
/// and only on operations whose scope it holds (`x-scope` in the v2 document).
async fn personal_token(parts: &Parts, state: &AppState, token: &str) -> Result<AuthUser, AuthError> {
    let path = original_path(parts);
    if !path.starts_with("/api/v2/") {
        return Err(AuthError::SessionInvalid);
    }
    let (pat, role) = db::personal_tokens::authenticate(state.db(), &db::refresh_tokens::hash_token(token))
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::SessionInvalid)?;
    let matched = parts.extensions.get::<axum::extract::MatchedPath>().map(|m| m.as_str().to_string());
    let needed = crate::v2::required_scope(parts.method.as_str(), matched.as_deref().unwrap_or(&path))
        .ok_or(AuthError::Forbidden)?;
    if !pat.scopes.iter().any(|s| s == needed) {
        return Err(AuthError::InsufficientScope(needed.to_string()));
    }
    if crate::demo::refuses(state, &parts.method, &path, pat.user_id).await {
        return Err(AuthError::DemoReadOnly);
    }
    Ok(AuthUser { user_id: pat.user_id, session_id: pat.id, role, chain_id: None, scopes: Some(pat.scopes) })
}

/// The caller is an instance admin, or the request stops here with `403` —
/// before any body is read, so a caller without the right never learns what
/// the body should have looked like (security hardening plan §5.3). Routes
/// declare it by taking this instead of [`AuthUser`].
pub struct InstanceAdmin(pub AuthUser);

impl FromRequestParts<AppState> for InstanceAdmin {
    type Rejection = AuthError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let auth = AuthUser::from_request_parts(parts, state).await?;
        if auth.role != "admin" {
            return Err(AuthError::Forbidden);
        }
        Ok(InstanceAdmin(auth))
    }
}
