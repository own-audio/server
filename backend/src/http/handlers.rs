// SPDX-License-Identifier: AGPL-3.0-or-later
use axum::Json;
use axum::extract::OriginalUri;
use axum::http::StatusCode;
use serde_json::{Value, json};

/// GET /health — liveness probe.
pub async fn health() -> Json<Value> {
    Json(json!({ "status": "ok" }))
}

/// Fallback for unknown paths under `/rest`. Without it those fall through
/// to the SPA fallback and a client asking for a mistyped endpoint gets 200 and a page of
/// HTML instead of an error it can act on.
pub async fn not_found() -> (StatusCode, Json<Value>) {
    (StatusCode::NOT_FOUND, Json(json!({ "error": "not_found" })))
}

/// Fallback for unknown paths under `/api/v1`: a hosted-only path that this
/// build does not serve answers `501 feature_unavailable` naming the
/// `features` key, anything else `404 not_found` (API_COMPATIBILITY.md §6).
/// `OriginalUri` because inside a `nest` the request's own URI has the
/// prefix stripped.
pub async fn api_fallback(OriginalUri(uri): OriginalUri) -> (StatusCode, Json<Value>) {
    match crate::http::router::hosted_feature_for(uri.path()) {
        Some(feature) => (
            StatusCode::NOT_IMPLEMENTED,
            Json(json!({ "error": "feature_unavailable", "feature": feature })),
        ),
        None => not_found().await,
    }
}
