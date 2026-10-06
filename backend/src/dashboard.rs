// SPDX-License-Identifier: AGPL-3.0-or-later
/// Instance-admin Home dashboard — counts and activity across the whole
/// server, not any one family. Cross-domain (users, families, billing,
/// stats, trash), so unlike `families::admin` this doesn't belong under any
/// single existing module. Mounted at `/admin/stats`.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::AuthUser;
use crate::db;
use axum::Router;
use axum::extract::State;
use axum::response::Json;
use axum::routing::get;
use serde::Serialize;

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(get_dashboard))
}

fn require_admin(auth: &AuthUser) -> Result<(), AuthError> {
    if auth.role != "admin" {
        Err(AuthError::SessionInvalid)
    } else {
        Ok(())
    }
}

#[derive(Serialize)]
pub struct WindowCounts {
    pub d1: i64,
    pub d7: i64,
    pub d30: i64,
}

impl From<(i64, i64, i64)> for WindowCounts {
    fn from((d1, d7, d30): (i64, i64, i64)) -> Self {
        Self { d1, d7, d30 }
    }
}

#[derive(Serialize)]
pub struct StreamingWindow {
    pub seconds: i64,
    pub sessions: i64,
    pub active_listeners: i64,
}

impl From<(i64, i64, i64)> for StreamingWindow {
    fn from((seconds, sessions, active_listeners): (i64, i64, i64)) -> Self {
        Self { seconds, sessions, active_listeners }
    }
}

#[derive(Serialize)]
pub struct StreamingWindows {
    pub d1: StreamingWindow,
    pub d7: StreamingWindow,
    pub d30: StreamingWindow,
}

#[derive(Serialize)]
pub struct DashboardStorage {
    pub audio_bytes: i64,
    pub other_bytes: i64,
    pub total_bytes: i64,
}

#[derive(Serialize)]
pub struct DashboardResponse {
    pub users_total: i64,
    pub families_total: i64,
    pub storage: DashboardStorage,
    pub credit_balance_micro: i64,
    pub trash_bytes_total: i64,
    pub registrations: WindowCounts,
    pub logins: WindowCounts,
    pub streaming: StreamingWindows,
}

/// GET /api/v1/admin/stats — instance-admin only.
async fn get_dashboard(
    auth: AuthUser,
    State(state): State<AppState>,
) -> Result<Json<DashboardResponse>, AuthError> {
    require_admin(&auth)?;

    let users = db::users::list_all(state.db()).await.map_err(AuthError::Internal)?;
    let families = db::families::list_all(state.db()).await.map_err(AuthError::Internal)?;
    let extras = state
        .hooks()
        .dashboard_extras(state.db(), state.storage())
        .await
        .map_err(AuthError::Internal)?;
    let (audio_bytes, other_bytes) = db::storage_usage::instance_storage_by_type(state.db(), state.storage().bucket())
        .await
        .map_err(AuthError::Internal)?;
    let trash = db::trash::family_stats(state.db()).await.map_err(AuthError::Internal)?;
    let registrations = db::users::registration_counts(state.db()).await.map_err(AuthError::Internal)?;
    let logins = db::sessions::login_counts(state.db()).await.map_err(AuthError::Internal)?;
    let (s1, s7, s30) = db::stats::streaming_totals(state.db()).await.map_err(AuthError::Internal)?;

    Ok(Json(DashboardResponse {
        users_total: users.len() as i64,
        families_total: families.len() as i64,
        storage: DashboardStorage {
            audio_bytes,
            other_bytes,
            total_bytes: audio_bytes + other_bytes,
        },
        // The edition's number (`Hooks::dashboard_extras`); 0 when there is no credit.
        credit_balance_micro: extras.get("credit_balance_micro").and_then(|v| v.as_i64()).unwrap_or(0),
        trash_bytes_total: trash.iter().map(|t| t.trashed_bytes).sum(),
        registrations: registrations.into(),
        logins: logins.into(),
        streaming: StreamingWindows {
            d1: s1.into(),
            d7: s7.into(),
            d30: s30.into(),
        },
    }))
}
