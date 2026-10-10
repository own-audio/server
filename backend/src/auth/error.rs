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

    /// The family has no room for more storage: the open-source quota
    /// (`STORAGE__FAMILY_QUOTA_BYTES`) or the hosted edition's credit
    /// (`storage::quota`). `402`: the request is well-formed and the caller
    /// has the right; what is missing is something the family has to get
    /// more of, and that is the status clients already treat that way.
    #[error("{0}")]
    StorageRefused(String),

    /// Too many wrong passwords in a row for this email (`auth::password`):
    /// `429` with `Retry-After`, the status every client already treats as
    /// "wait", whether or not the email has an account.
    #[error("too many sign-in attempts; try again in {retry_after_secs} seconds")]
    Locked { retry_after_secs: u64 },

    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        if let AuthError::Locked { retry_after_secs } = &self {
            return (
                StatusCode::TOO_MANY_REQUESTS,
                [("Retry-After", retry_after_secs.to_string())],
                Json(json!({ "error": "account_locked", "retry_after_secs": retry_after_secs })),
            )
                .into_response();
        }
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
            AuthError::StorageRefused(message) => (StatusCode::PAYMENT_REQUIRED, message.clone()),
            AuthError::Locked { .. } => unreachable!("answered above"),
            AuthError::Internal(e) => {
                tracing::error!("auth internal error: {e:#}");
                (StatusCode::INTERNAL_SERVER_ERROR, "internal server error".to_string())
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}
