// SPDX-License-Identifier: AGPL-3.0-or-later
/// Jobs module — feed polling, metadata fetch, thumbnail processing, cleanup, imports.
pub mod models;
pub mod worker;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::auth::middleware::InstanceAdmin;
use crate::db;
use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct JobResponse {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub error: Option<String>,
    /// Structured output a job chose to report on completion (e.g. the
    /// cleanup sweep's `{"deleted_families": [...]}`) — most job types leave
    /// this null.
    pub result: Option<serde_json::Value>,
    pub attempts: i32,
    pub scheduled_at: String,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub created_at: String,
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_jobs))
        .routes(routes!(get_job))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/jobs/  — admin only
///
/// The 50 most recent background jobs.
#[utoipa::path(get, path = "/", tag = "jobs", security(("bearer" = [])),
    responses((status = 200, body = Vec<JobResponse>),
        (status = 403, description = "Not an instance admin", body = crate::http::openapi::ErrorBody)))]
async fn list_jobs(
    InstanceAdmin(_auth): InstanceAdmin,
    State(state): State<AppState>,
) -> Result<Json<Vec<JobResponse>>, AuthError> {
    let jobs = db::jobs::list_recent(state.db(), 50)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(jobs.into_iter().map(job_to_response).collect()))
}

/// GET /api/v1/jobs/:id  — admin only
///
/// One background job.
#[utoipa::path(get, path = "/{id}", tag = "jobs", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Job id")),
    responses((status = 200, body = JobResponse),
        (status = 403, description = "Not an instance admin", body = crate::http::openapi::ErrorBody)))]
async fn get_job(
    InstanceAdmin(_auth): InstanceAdmin,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<JobResponse>, AuthError> {
    let job = db::jobs::find_by_id(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::NotFound)?;

    Ok(Json(job_to_response(job)))
}

fn job_to_response(j: models::Job) -> JobResponse {
    JobResponse {
        id: j.id.to_string(),
        job_type: j.job_type,
        status: j.status,
        error: j.error,
        result: j.result,
        attempts: j.attempts,
        scheduled_at: j.scheduled_at.to_rfc3339(),
        started_at: j.started_at.map(|t| t.to_rfc3339()),
        completed_at: j.completed_at.map(|t| t.to_rfc3339()),
        created_at: j.created_at.to_rfc3339(),
    }
}

