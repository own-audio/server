// SPDX-License-Identifier: AGPL-3.0-or-later
//! `GET /api/v1/library/folders` — the configured folders and their last scan;
//! `POST /api/v1/library/folders/scan` — scan now. Family admins only.

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::families::FamilyContext;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize, sqlx::FromRow)]
pub struct FolderStatus {
    id: Uuid,
    path: String,
    kind: String,
    visibility: String,
    scan_started_at: Option<chrono::DateTime<chrono::Utc>>,
    scan_finished_at: Option<chrono::DateTime<chrono::Utc>>,
    scan_error: Option<String>,
    files_seen: i32,
    files_added: i32,
    files_missing: i64,
}

#[derive(Serialize)]
pub struct FoldersResponse {
    scanning: bool,
    files_scanned: u64,
    folders: Vec<FolderStatus>,
}

pub fn router() -> Router<AppState> {
    Router::new().route("/", get(list)).route("/scan", post(scan))
}

async fn list(family: FamilyContext, State(state): State<AppState>) -> Result<Json<FoldersResponse>, AuthError> {
    if !family.is_family_admin() {
        return Err(AuthError::Forbidden);
    }
    let folders = sqlx::query_as::<_, FolderStatus>(
        "SELECT f.id, f.path, f.kind, f.visibility, f.scan_started_at, f.scan_finished_at, f.scan_error,
                f.files_seen, f.files_added,
                (SELECT count(*) FROM library_files lf WHERE lf.folder_id = f.id AND lf.missing) AS files_missing
         FROM library_folders f
         WHERE f.family_id = $1
         ORDER BY f.path",
    )
    .bind(family.family_id)
    .fetch_all(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    let (scanning, files_scanned) = super::scan_status();
    Ok(Json(FoldersResponse { scanning, files_scanned, folders }))
}

async fn scan(family: FamilyContext) -> Result<StatusCode, AuthError> {
    if !family.is_family_admin() {
        return Err(AuthError::Forbidden);
    }
    super::request_scan();
    Ok(StatusCode::ACCEPTED)
}
