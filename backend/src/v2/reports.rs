// SPDX-License-Identifier: AGPL-3.0-or-later
//! Content reports (docs/api-v2-completion-plan.md, phase 7): anyone in a household may report
//! something in its library, and its admins read the reports and close them. Nobody outside the
//! household moderates a family library, so reporting is not a privilege (v1's rule, kept).

use crate::app::AppState;
use crate::db::{self, access, catalog};
use crate::db::user_state::{self, Kind};
use crate::families::{FamilyAdmin, FamilyContext};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// Long enough to say what was wrong, short enough that the column is not an essay store (v1's).
const MAX_NOTE_CHARS: usize = 1000;

#[derive(Serialize, ToSchema)]
pub struct Report {
    pub id: Uuid,
    /// `track`, `book` or `podcast`.
    pub kind: String,
    pub item_id: Uuid,
    /// `offensive`, `inaccurate`, `not_for_children` or `other`.
    pub reason: String,
    pub note: Option<String>,
    /// Who reported it; absent once their account is deleted (the report stays).
    pub reporter_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub resolved: bool,
    pub resolved_at: Option<DateTime<Utc>>,
    pub resolved_by: Option<Uuid>,
    /// What the admin who closed it said.
    pub resolution_note: Option<String>,
}

/// v1 stores the medium; v2 names the item's kind.
fn v2_kind(media_kind: &str) -> &'static str {
    match media_kind {
        access::MUSIC => "track",
        access::AUDIOBOOK => "book",
        _ => "podcast",
    }
}

pub(crate) fn v1_kind(kind: &str) -> Option<&'static str> {
    match kind {
        "track" => Some(access::MUSIC),
        "book" => Some(access::AUDIOBOOK),
        "podcast" => Some(access::PODCAST),
        _ => None,
    }
}

fn report(row: access::ReportRow) -> Report {
    Report {
        id: row.id,
        kind: v2_kind(&row.media_kind).to_string(),
        item_id: row.item_id,
        reason: row.reason,
        note: row.note,
        reporter_id: row.reporter_user_id,
        created_at: row.created_at,
        resolved: row.resolved_at.is_some(),
        resolved_at: row.resolved_at,
        resolved_by: row.resolved_by,
        resolution_note: row.resolution_note,
    }
}

/// Free text cut to its length, blank as none.
pub(crate) fn note(text: Option<&str>) -> Option<String> {
    text.map(str::trim).filter(|n| !n.is_empty()).map(|n| n.chars().take(MAX_NOTE_CHARS).collect())
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ReportsQuery {
    /// `open` (the default), `resolved` or `all`.
    pub state: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

pub(crate) fn report_state(state: Option<&str>) -> Result<Option<bool>, Problem> {
    match state.map(str::trim) {
        None | Some("") | Some("open") => Ok(Some(false)),
        Some("resolved") => Ok(Some(true)),
        Some("all") => Ok(None),
        Some(other) => Err(Problem::bad_request("invalid_state", format!("state is `open`, `resolved` or `all`, not `{other}`"))
            .field("state", "invalid", other.to_string())),
    }
}

/// The household's reports, newest first: what is open, unless `state` says otherwise.
#[utoipa::path(get, path = "/family/reports", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(ReportsQuery),
    responses((status = 200, body = Page<Report>),
        (status = 400, description = "`invalid_state`, `invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody)))]
pub async fn list_reports(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Query(q): Query<ReportsQuery>) -> Result<Json<Page<Report>>, Problem> {
    let resolved = report_state(q.state.as_deref())?;
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let rows = access::page_reports(state.db(), family.family_id, resolved, page.after::<(DateTime<Utc>, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |r| (r.created_at, r.id));
    Ok(Json(Page { items: page.items.into_iter().map(report).collect(), next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct NewReport {
    /// `track`, `book` or `podcast`.
    pub kind: String,
    pub item_id: Uuid,
    /// `offensive`, `inaccurate`, `not_for_children` or `other`.
    pub reason: String,
    /// What is wrong, in a few words (up to 1,000 characters).
    pub note: Option<String>,
}

/// Report something in the household's library to its admins. Anyone in the household may,
/// including someone who may not add to it. Send an `Idempotency-Key` to retry safely.
#[utoipa::path(post, path = "/family/reports", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = NewReport,
    responses((status = 201, body = Report),
        (status = 400, description = "`unsupported_kind`, `invalid_reason`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn create_report(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewReport>) -> Result<(StatusCode, Json<Report>), Problem> {
    let media_kind = v1_kind(&body.kind).ok_or_else(|| {
        Problem::bad_request("unsupported_kind", "kind is `track`, `book` or `podcast`").field("/kind", "invalid", body.kind.clone())
    })?;
    if !access::is_valid_report_reason(&body.reason) {
        return Err(Problem::bad_request("invalid_reason", "reason is `offensive`, `inaccurate`, `not_for_children` or `other`")
            .field("/reason", "invalid", body.reason.clone()));
    }
    let visible = match body.kind.as_str() {
        "track" => user_state::is_visible(state.db(), family.viewer(), Kind::Track, body.item_id).await?,
        "book" => user_state::is_visible(state.db(), family.viewer(), Kind::Book, body.item_id).await?,
        _ => catalog::find_podcast(state.db(), family.viewer(), body.item_id).await?.is_some(),
    };
    if !visible {
        return Err(Problem::not_found());
    }
    let id = access::insert_report(state.db(), family.family_id, family.user_id, media_kind, body.item_id, &body.reason, note(body.note.as_deref()).as_deref()).await?;
    let row = access::find_report(state.db(), family.family_id, id).await?.ok_or_else(Problem::not_found)?;
    Ok((StatusCode::CREATED, Json(report(row))))
}

#[derive(Deserialize, ToSchema)]
pub struct ReportPatch {
    /// `true` closes the report, `false` opens it again.
    pub resolved: bool,
    /// What was done about it, kept with the closed report.
    pub note: Option<String>,
}

/// Close a report, with a note of what was done, or open it again. Closing a closed report keeps
/// when and by whom it was closed.
#[utoipa::path(patch, path = "/family/reports/{id}", tag = "household", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("id" = Uuid, Path)),
    request_body = ReportPatch,
    responses((status = 200, body = Report), (status = 404, body = crate::v2::problem::ProblemBody)))]
pub async fn patch_report(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<ReportPatch>) -> Result<Json<Report>, Problem> {
    let row = db::access::set_report_resolved(state.db(), family.family_id, id, body.resolved, family.user_id, note(body.note.as_deref()).as_deref())
        .await?
        .ok_or_else(Problem::not_found)?;
    Ok(Json(report(row)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_go_both_ways() {
        for kind in ["track", "book", "podcast"] {
            assert_eq!(v2_kind(v1_kind(kind).unwrap()), kind);
        }
        assert_eq!(v1_kind("episode"), None);
        assert_eq!(v1_kind("music"), None, "v1's words are not v2's");
    }

    #[test]
    fn the_list_reads_open_reports_unless_asked() {
        assert_eq!(report_state(None).unwrap(), Some(false));
        assert_eq!(report_state(Some("open")).unwrap(), Some(false));
        assert_eq!(report_state(Some("resolved")).unwrap(), Some(true));
        assert_eq!(report_state(Some("all")).unwrap(), None);
        assert_eq!(report_state(Some("closed")).unwrap_err().code(), "invalid_state");
    }

    #[test]
    fn a_note_is_trimmed_cut_and_blank_is_none() {
        assert_eq!(note(Some("  ")), None);
        assert_eq!(note(Some(" rude ")).as_deref(), Some("rude"));
        assert_eq!(note(Some(&"x".repeat(1500))).unwrap().chars().count(), MAX_NOTE_CHARS);
    }
}
