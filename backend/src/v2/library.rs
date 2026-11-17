// SPDX-License-Identifier: AGPL-3.0-or-later
//! The library's quarantine: files the scan or the fingerprint job set aside
//! because they are broken (migration 0124) — unreadable, not decoding, a
//! stub, or silent. They are out of every list and player; here an admin sees
//! why, and can let one back.
use crate::app::AppState;
use crate::db;
use crate::families::{FamilyAdmin, FamilyContext};
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

// ── Library folders ─────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct LibraryFolders {
    /// A scan is running now.
    pub scanning: bool,
    /// Files the running (or last) scan has looked at.
    pub files_scanned: u64,
    pub folders: Vec<LibraryFolder>,
}

/// A folder on the server's disk the library is read from, never written to.
#[derive(Serialize, ToSchema)]
pub struct LibraryFolder {
    pub id: Uuid,
    /// Where it is, inside the server's container.
    pub path: String,
    /// `music` or `books`.
    pub media: String,
    /// What is found there: `family` (shared) or `private`.
    pub visibility: String,
    /// The last scan; absent before the first.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_scan: Option<FolderScan>,
    /// Files the library still lists but the folder no longer has.
    pub files_missing: i64,
}

#[derive(Serialize, ToSchema)]
pub struct FolderScan {
    pub started_at: DateTime<Utc>,
    /// Absent while it runs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<DateTime<Utc>>,
    /// Why it stopped, when it failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub files_seen: i32,
    pub files_added: i32,
}

pub(crate) fn folder_media(kind: &str) -> &'static str {
    if kind == "audiobooks" { "books" } else { "music" }
}

/// The household's library folders and how their last scan went.
#[utoipa::path(get, path = "/library/folders", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = LibraryFolders)))]
pub async fn list_folders(family: FamilyContext, State(state): State<AppState>) -> Result<Json<LibraryFolders>, Problem> {
    let folders = db::admin::library_folders(state.db(), family.family_id)
        .await?
        .into_iter()
        .map(|f| LibraryFolder {
            id: f.id,
            path: f.path,
            media: folder_media(&f.kind).to_string(),
            visibility: f.visibility,
            last_scan: f.scan_started_at.map(|started_at| FolderScan {
                started_at,
                finished_at: f.scan_finished_at,
                error: f.scan_error,
                files_seen: f.files_seen,
                files_added: f.files_added,
            }),
            files_missing: f.files_missing,
        })
        .collect();
    let (scanning, files_scanned) = crate::library_folders::scan_status();
    Ok(Json(LibraryFolders { scanning, files_scanned, folders }))
}

/// Scan the library folders now rather than at the scanner's next look. Answers at once; the
/// list above says when it is done.
#[utoipa::path(post, path = "/library/folders/scan", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    responses((status = 202, description = "A scan is on its way")))]
pub async fn scan_folders(_admin: FamilyAdmin) -> StatusCode {
    crate::library_folders::request_scan();
    StatusCode::ACCEPTED
}

#[cfg(test)]
mod tests {
    #[test]
    fn folder_kinds_are_v2_media() {
        assert_eq!(super::folder_media("audiobooks"), "books");
        assert_eq!(super::folder_media("music"), "music");
    }
}
