// SPDX-License-Identifier: AGPL-3.0-or-later
//! Every v2 error as RFC 9457 problem details (`application/problem+json`):
//! `type`, `title`, `status`, a stable `code` to match on, `detail` for people,
//! `errors[]` for fields.
//!
//! Handlers return [`Problem`]; everything else that can answer with an error
//! under `/api/v2` — the extractors' rejections, the rate limiter, axum's body
//! rejections, the fallback — is rewritten into the same shape by [`middleware`],
//! so no error leaves in another shape.

use crate::auth::error::AuthError;
use axum::body::to_bytes;
use axum::extract::Request;
use axum::http::{HeaderValue, StatusCode, header};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use serde::Serialize;
use serde_json::Value;
use utoipa::ToSchema;

pub const CONTENT_TYPE: &str = "application/problem+json";

/// The body of every v2 error.
#[derive(Serialize, ToSchema, Debug)]
pub struct ProblemBody {
    /// `urn:own-audio:problem:<code>`.
    #[serde(rename = "type")]
    pub type_: String,
    /// The status's reason phrase.
    pub title: String,
    pub status: u16,
    /// Stable and machine-readable: `not_found`, `invalid_body`, `rate_limited`…
    pub code: String,
    /// What went wrong, in English, for people; not meant to be matched on.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// One entry per invalid field.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub errors: Vec<FieldError>,
    /// Members a particular problem adds: the token endpoint's OAuth `error` and
    /// `error_description`, a two-factor challenge's `mfa_token`.
    #[serde(flatten)]
    #[schema(ignore)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Serialize, ToSchema, Debug, Clone)]
pub struct FieldError {
    /// The field, as a JSON Pointer into the request body or the query parameter's name.
    pub field: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

#[derive(Debug)]
pub struct Problem {
    status: StatusCode,
    code: String,
    detail: Option<String>,
    errors: Vec<FieldError>,
    retry_after: Option<u64>,
    extra: serde_json::Map<String, Value>,
}

impl Problem {
    pub fn new(status: StatusCode, code: impl Into<String>) -> Self {
        Problem { status, code: code.into(), detail: None, errors: Vec::new(), retry_after: None, extra: Default::default() }
    }

    pub fn detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }

    pub fn field(mut self, field: impl Into<String>, code: impl Into<String>, detail: impl Into<String>) -> Self {
        self.errors.push(FieldError { field: field.into(), code: code.into(), detail: Some(detail.into()) });
        self
    }

    /// An extra member of the body.
    pub fn with(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.extra.insert(key.to_string(), value.into());
        self
    }

    pub fn not_found() -> Self {
        Problem::new(StatusCode::NOT_FOUND, "not_found")
    }

    pub fn bad_request(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Problem::new(StatusCode::BAD_REQUEST, code).detail(detail)
    }

    pub fn forbidden() -> Self {
        Problem::new(StatusCode::FORBIDDEN, "forbidden")
    }

    pub fn internal(e: impl Into<anyhow::Error>) -> Self {
        tracing::error!("v2 internal error: {:#}", e.into());
        Problem::new(StatusCode::INTERNAL_SERVER_ERROR, "internal")
    }

    pub fn status(&self) -> StatusCode {
        self.status
    }

    pub fn code(&self) -> &str {
        &self.code
    }

    fn body(&self) -> ProblemBody {
        ProblemBody {
            type_: format!("urn:own-audio:problem:{}", self.code),
            title: self.status.canonical_reason().unwrap_or("Error").to_string(),
            status: self.status.as_u16(),
            code: self.code.clone(),
            detail: self.detail.clone(),
            errors: self.errors.clone(),
            extra: self.extra.clone(),
        }
    }
}

impl IntoResponse for Problem {
    fn into_response(self) -> Response {
        let body = serde_json::to_vec(&self.body()).unwrap_or_default();
        let mut response = (self.status, body).into_response();
        response.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static(CONTENT_TYPE));
        if let Some(secs) = self.retry_after {
            response.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from(secs));
        }
        response
    }
}

impl From<anyhow::Error> for Problem {
    fn from(e: anyhow::Error) -> Self {
        Problem::internal(e)
    }
}

impl From<AuthError> for Problem {
    fn from(e: AuthError) -> Self {
        let detail = e.to_string();
        match e {
            AuthError::InvalidCredentials | AuthError::NotFound => Problem::new(StatusCode::UNAUTHORIZED, "invalid_credentials"),
            AuthError::SessionInvalid => Problem::new(StatusCode::UNAUTHORIZED, "unauthorized"),
            AuthError::BadRequest(message) => Problem::bad_request("bad_request", message),
            AuthError::ItemNotFound => Problem::not_found(),
            AuthError::Forbidden => Problem::forbidden(),
            AuthError::InsufficientScope(_) => Problem::new(StatusCode::FORBIDDEN, "insufficient_scope").detail(detail),
            AuthError::DemoReadOnly => Problem::new(StatusCode::FORBIDDEN, "demo_read_only").detail(detail),
            AuthError::EmailUnverified => Problem::new(StatusCode::FORBIDDEN, "email_unverified").detail(detail),
            AuthError::IdentityConflict => Problem::new(StatusCode::CONFLICT, "identity_conflict").detail(detail),
            AuthError::ProviderNotConfigured => Problem::new(StatusCode::NOT_IMPLEMENTED, "provider_not_configured"),
            AuthError::Conflict(message) => Problem::new(StatusCode::CONFLICT, "conflict").detail(message),
            AuthError::StorageRefused(message) => Problem::new(StatusCode::PAYMENT_REQUIRED, "storage_refused").detail(message),
            AuthError::Locked { retry_after_secs } => Problem {
                retry_after: Some(retry_after_secs),
                ..Problem::new(StatusCode::TOO_MANY_REQUESTS, "account_locked").detail(detail)
            },
            AuthError::Internal(e) => Problem::internal(e),
        }
    }
}

/// The code for an error answer that brought none of its own.
fn code_for(status: StatusCode) -> &'static str {
    match status {
        StatusCode::BAD_REQUEST => "bad_request",
        StatusCode::UNAUTHORIZED => "unauthorized",
        StatusCode::PAYMENT_REQUIRED => "payment_required",
        StatusCode::FORBIDDEN => "forbidden",
        StatusCode::NOT_FOUND => "not_found",
        StatusCode::METHOD_NOT_ALLOWED => "method_not_allowed",
        StatusCode::CONFLICT => "conflict",
        StatusCode::GONE => "gone",
        StatusCode::PAYLOAD_TOO_LARGE => "too_large",
        StatusCode::UNSUPPORTED_MEDIA_TYPE => "unsupported_media_type",
        StatusCode::UNPROCESSABLE_ENTITY => "invalid_body",
        StatusCode::TOO_MANY_REQUESTS => "rate_limited",
        StatusCode::NOT_IMPLEMENTED => "not_implemented",
        s if s.is_server_error() => "internal",
        _ => "error",
    }
}

/// Rewrites every error answer under `/api/v2` that is not problem details yet.
/// The original's `code` (or its `error` when that is a code, like
/// `account_locked`) is kept; its message becomes `detail`, except for server
/// errors, whose text stays in the log.
pub async fn middleware(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let status = response.status();
    if !(status.is_client_error() || status.is_server_error()) {
        return response;
    }
    let is_problem = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with(CONTENT_TYPE));
    if is_problem {
        return response;
    }
    let (parts, body) = response.into_parts();
    let bytes = to_bytes(body, 64 * 1024).await.unwrap_or_default();
    let mut problem = Problem::new(status, code_for(status));
    if let Ok(json) = serde_json::from_slice::<Value>(&bytes) {
        let message = json.get("error").and_then(Value::as_str).map(str::to_string);
        let is_code = |s: &str| !s.is_empty() && s.chars().all(|c| c.is_ascii_lowercase() || c == '_');
        if let Some(code) = json.get("code").and_then(Value::as_str) {
            problem.code = code.to_string();
            problem.detail = message;
        } else if let Some(message) = message {
            if is_code(&message) {
                problem.code = message;
            } else {
                problem.detail = Some(message);
            }
        }
    } else if !bytes.is_empty() && status.is_client_error() {
        // axum's own rejections are plain text ("Failed to deserialize the JSON body…").
        let text = String::from_utf8_lossy(&bytes);
        problem.detail = Some(text.chars().take(300).collect());
    }
    if status.is_server_error() {
        problem.detail = None;
    }
    let mut answer = problem.into_response();
    // Keep what the original said about retrying and authenticating.
    for name in [header::RETRY_AFTER, header::WWW_AUTHENTICATE, header::ALLOW] {
        if let Some(value) = parts.headers.get(&name) {
            answer.headers_mut().insert(name, value.clone());
        }
    }
    answer
}

/// Unknown paths under `/api/v2`.
pub async fn fallback() -> Problem {
    Problem::not_found()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_has_the_rfc_fields() {
        let p = Problem::bad_request("invalid_cursor", "the cursor is not one this server made")
            .field("cursor", "invalid", "not base64url");
        let v = serde_json::to_value(p.body()).unwrap();
        assert_eq!(v["type"], "urn:own-audio:problem:invalid_cursor");
        assert_eq!(v["title"], "Bad Request");
        assert_eq!(v["status"], 400);
        assert_eq!(v["code"], "invalid_cursor");
        assert_eq!(v["errors"][0]["field"], "cursor");
    }

    #[test]
    fn auth_errors_keep_their_meaning() {
        assert_eq!(Problem::from(AuthError::ItemNotFound).status(), StatusCode::NOT_FOUND);
        assert_eq!(Problem::from(AuthError::EmailUnverified).code(), "email_unverified");
        let locked = Problem::from(AuthError::Locked { retry_after_secs: 30 });
        assert_eq!((locked.status(), locked.code()), (StatusCode::TOO_MANY_REQUESTS, "account_locked"));
    }
}
