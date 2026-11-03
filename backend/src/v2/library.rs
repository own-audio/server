// SPDX-License-Identifier: AGPL-3.0-or-later
//! The library's quarantine: files the scan or the fingerprint job set aside
//! because they are broken (migration 0124) — unreadable, not decoding, a
//! stub, or silent. They are out of every list and player; here an admin sees
//! why, and can let one back.
use crate::app::AppState;
use crate::db;
use crate::families::FamilyAdmin;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
pub struct QuarantinedTrack {
    pub track_id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    /// The file as the server knows it: `folder/<folder id>/<path in the folder>` or an upload's key.
    pub file: String,
    pub size_bytes: Option<i64>,
    /// Why, in words: what the scan or the fingerprint job found.
    pub reason: String,
    pub since: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct QuarantineList {
    pub items: Vec<QuarantinedTrack>,
}

/// The household's quarantined tracks and why each is there.
#[utoipa::path(get, path = "/library/quarantine", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    responses((status = 200, body = QuarantineList)))]
pub async fn list_quarantine(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>) -> Result<Json<QuarantineList>, Problem> {
    let viewer = family.viewer();
    let items = db::quarantine::list(state.db(), viewer.family_id, viewer.user_id)
        .await?
        .into_iter()
        .map(|q| QuarantinedTrack {
            track_id: q.track_id,
            title: q.title,
            artist: q.artist,
            album: q.album,
            file: q.file,
            size_bytes: q.size_bytes,
            reason: q.reason,
            since: q.since,
        })
        .collect();
    Ok(Json(QuarantineList { items }))
}

/// Lets a track back into the library. The file is not read again: it comes
/// back as it is, until it changes on disk and the scan reads it anew.
#[utoipa::path(delete, path = "/library/quarantine/{track_id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("track_id" = Uuid, Path)),
    responses((status = 204, description = "Back in the library"), (status = 404, description = "Not the household's, or not in quarantine", body = crate::v2::problem::ProblemBody)))]
pub async fn release_quarantine(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Path(track_id): Path<Uuid>) -> Result<StatusCode, Problem> {
    let viewer = family.viewer();
    if !db::quarantine::belongs(state.db(), track_id, viewer.family_id, viewer.user_id).await? {
        return Err(Problem::not_found());
    }
    if !db::quarantine::release(state.db(), track_id).await? {
        return Err(Problem::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}
