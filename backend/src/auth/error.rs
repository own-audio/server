// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("invalid credentials")]
    InvalidCredentials,

    #[error("bad request: {0}")]
    BadRequest(String),

    #[error("account not found")]
    NotFound,

    /// A requested item does not exist **or** is not visible to this viewer — the two are
    /// deliberately not distinguished, so a family member cannot probe for what someone else
    /// keeps private. Separate from `NotFound` (an account, answered `401` on purpose during
    /// sign-in so it cannot be used to enumerate emails): a `401` here would tell every client
    /// its session had expired, costing a pointless token refresh and, in some, a sign-out.
    #[error("not found")]
    ItemNotFound,

    #[error("session expired or invalid")]
    SessionInvalid,

    #[error("forbidden")]
    Forbidden,

    /// A public demo's shared account tried to change something (`crate::demo`).
    #[error("the demo account is read-only")]
    DemoReadOnly,

    #[error("identity already linked to another account")]
    IdentityConflict,

    #[error("provider not configured")]
    ProviderNotConfigured,

    /// The request is valid but the item is not in the state it needs — a
    /// podcast episode streamed before it was downloaded. `409` with the
    /// given message, never a `500`: nothing failed on the server.
    #[error("{0}")]
    Conflict(String),

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AuthError::InvalidCredentials | AuthError::NotFound => {
                (StatusCode::UNAUTHORIZED, self.to_string())
            }
            AuthError::BadRequest(message) => (StatusCode::BAD_REQUEST, message.clone()),
            AuthError::ItemNotFound => (StatusCode::NOT_FOUND, self.to_string()),
            AuthError::SessionInvalid => (StatusCode::UNAUTHORIZED, self.to_string()),
            AuthError::Forbidden | AuthError::DemoReadOnly => (StatusCode::FORBIDDEN, self.to_string()),
            AuthError::IdentityConflict => (StatusCode::CONFLICT, self.to_string()),
            AuthError::ProviderNotConfigured => (StatusCode::NOT_IMPLEMENTED, self.to_string()),
            AuthError::Conflict(message) => (StatusCode::CONFLICT, message.clone()),
            AuthError::Internal(e) => {
                tracing::error!("auth internal error: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
