// SPDX-License-Identifier: AGPL-3.0-or-later
/// Devices module — push registration and the notification inbox.
///
/// Both mobile platforms are first-class: `apns` for iOS, `fcm` for Android.
/// Registration and the inbox work today; actual push *delivery* needs
/// provider credentials, so until those are configured a client polls
/// `GET /devices/notifications` on resume and gets the same information.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use axum::Router;
use axum::extract::{Json, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct RegisterTokenRequest {
    /// `apns` (iOS) or `fcm` (Android).
    pub platform: String,
    pub token: String,
    #[serde(default)]
    pub device_name: Option<String>,
}

#[derive(Deserialize)]
pub struct DeleteTokenRequest {
    pub token: String,
}

#[derive(Serialize)]
pub struct PushTokenResponse {
    pub id: String,
    pub platform: String,
    pub device_name: Option<String>,
    pub last_seen_at: String,
    /// Only the tail is echoed back; the full token is a delivery credential.
    pub token_suffix: String,
}

#[derive(Serialize)]
pub struct NotificationResponse {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub body: Option<String>,
    pub data: Option<serde_json::Value>,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct NotificationsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Deserialize)]
pub struct AckRequest {
    pub ids: Vec<Uuid>,
}

#[derive(Serialize)]
pub struct AckResponse {
    pub acknowledged: u64,
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/push-token", post(register_token))
        .route("/push-token", delete(delete_token))
        .route("/push-tokens", get(list_tokens))
        .route("/notifications", get(list_notifications))
        .route("/notifications/ack", post(ack_notifications))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// POST /api/v1/devices/push-token
///
/// Call on every app start: tokens rotate on both platforms, and
/// re-registering also refreshes `last_seen_at`. Re-registering a token that
/// belonged to another account moves it, so passing a device on does not keep
/// delivering the previous owner's notifications.
async fn register_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<RegisterTokenRequest>,
) -> Result<StatusCode, AuthError> {
    let platform = match body.platform.trim() {
        "apns" => "apns",
        "fcm" => "fcm",
        _ => {
            return Err(AuthError::BadRequest(
                "platform must be 'apns' or 'fcm'".into(),
            ));
        }
    };

    let token = body.token.trim();
    if token.is_empty() {
        return Err(AuthError::BadRequest("token is required".into()));
    }

    db::sync::register_push_token(
        state.db(),
        auth.user_id,
        platform,
        token,
        body.device_name.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/devices/push-token — call on sign-out so the device stops
/// receiving notifications for an account that is no longer signed in.
async fn delete_token(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<DeleteTokenRequest>,
) -> Result<StatusCode, AuthError> {
    db::sync::delete_push_token(state.db(), auth.user_id, body.token.trim())
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::NO_CONTENT)
}

async fn list_tokens(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<PushTokenResponse>>, AuthError> {
    let tokens = db::sync::list_push_tokens(state.db(), auth.user_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        tokens
            .into_iter()
            .map(|t| {
                let suffix = t.token.chars().rev().take(6).collect::<String>();
                PushTokenResponse {
                    id: t.id.to_string(),
                    platform: t.platform,
                    device_name: t.device_name,
                    last_seen_at: t.last_seen_at.to_rfc3339(),
                    token_suffix: suffix.chars().rev().collect(),
                }
            })
            .collect(),
    ))
}

/// GET /api/v1/devices/notifications
///
/// The polling fallback for push. Safe to call on every app resume; entries
/// stay until acknowledged, so nothing is lost if the app is killed first.
async fn list_notifications(
    auth: AuthUser,
    State(state): State<AppState>,
    Query(q): Query<NotificationsQuery>,
) -> Result<Json<Vec<NotificationResponse>>, AuthError> {
    let limit = q.limit.clamp(1, 200);

    let items = db::sync::pending_for_user(state.db(), auth.user_id, limit)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        items
            .into_iter()
            .map(|n| NotificationResponse {
                id: n.id.to_string(),
                kind: n.kind,
                title: n.title,
                body: n.body,
                data: n.data,
                created_at: n.created_at.to_rfc3339(),
            })
            .collect(),
    ))
}

/// POST /api/v1/devices/notifications/ack — mark notifications handled so
/// they stop coming back on the next poll.
async fn ack_notifications(
    auth: AuthUser,
    State(state): State<AppState>,
    Json(body): Json<AckRequest>,
) -> Result<Json<AckResponse>, AuthError> {
    let acknowledged = db::sync::mark_notifications_delivered(state.db(), auth.user_id, &body.ids)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(AckResponse { acknowledged }))
}
