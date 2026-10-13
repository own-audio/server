// SPDX-License-Identifier: AGPL-3.0-or-later
//! The history of metadata changes (docs/metadata-organizer-plan.md, O0): what
//! was changed, by whom, how, and undoing it. A member sees the household's
//! albums and contributors, their own songs, and what they changed themselves;
//! a song's change is undone by its owner, an album's or a contributor's by any
//! member.

use crate::app::AppState;
use crate::db;
use crate::db::edits::{EditRow, Filter, UndoOutcome};
use crate::families::FamilyContext;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
pub struct MetadataEdit {
    pub id: Uuid,
    /// `track`, `album` or `contributor`.
    pub item_kind: String,
    pub item_id: Uuid,
    /// How the item is called now.
    pub item_label: Option<String>,
    pub user_id: Option<Uuid>,
    /// `manual`, `bulk`, `identify`, `run`, `revert` or `undo`.
    pub source: String,
    pub batch_id: Uuid,
    /// The fields that changed, as they were and as they became.
    pub before: Value,
    pub after: Value,
    pub at: DateTime<Utc>,
    pub undone_at: Option<DateTime<Utc>>,
}

fn edit_out(e: EditRow) -> MetadataEdit {
    MetadataEdit {
        id: e.id,
        item_kind: e.item_kind,
        item_id: e.item_id,
        item_label: e.item_label,
        user_id: e.user_id,
        source: e.source,
        batch_id: e.batch_id,
        before: e.before,
        after: e.after,
        at: e.at,
        undone_at: e.undone_at,
    }
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EditFilter {
    /// With `item_id`: one song's, album's or contributor's history.
    pub item_kind: Option<String>,
    pub item_id: Option<Uuid>,
    /// One save's changes.
    pub batch_id: Option<Uuid>,
    /// Only the caller's own changes.
    #[serde(default)]
    pub mine: bool,
}

/// Changes, newest first.
#[utoipa::path(get, path = "/edits", tag = "edits", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    params(EditFilter, PageQuery),
    responses((status = 200, body = Page<MetadataEdit>), (status = 400, body = crate::v2::problem::ProblemBody)))]
pub async fn list_edits(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(filter): Query<EditFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<MetadataEdit>>, Problem> {
    let item = match (filter.item_kind, filter.item_id) {
        (Some(kind), Some(id)) if ["track", "album", "contributor"].contains(&kind.as_str()) => Some((kind, id)),
        (None, None) => None,
        _ => {
            return Err(Problem::bad_request("invalid_item", "item_kind (track, album, contributor) and item_id go together")
                .field("/item_kind", "invalid", "track, album or contributor, with item_id"));
        }
    };
    let f = Filter { item, batch: filter.batch_id, by: filter.mine.then_some(family.user_id) };
    let limit = page.limit()?;
    let after: Option<(DateTime<Utc>, Uuid)> = page.after()?;
    let rows = db::edits::list(state.db(), family.family_id, family.user_id, &f, after, limit + 1).await?;
    let keys: Vec<(DateTime<Utc>, Uuid)> = rows.iter().map(|r| (r.at, r.id)).collect();
    let mut out = Page::from_rows(rows.into_iter().map(edit_out).collect::<Vec<_>>(), limit, |e| e.id);
    out.next = out.next.as_ref().and_then(|_| keys.get(out.items.len().saturating_sub(1)).map(crate::v2::page::encode));
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
pub struct EditBatch {
    pub batch_id: Uuid,
    pub user_id: Option<Uuid>,
    pub source: String,
    pub at: DateTime<Utc>,
    /// Songs, albums and contributors it changed.
    pub items: i64,
    /// One of them, by name.
    pub first_label: Option<String>,
    pub undone: bool,
}

/// Saves, newest first: one row per batch, for a history list.
#[utoipa::path(get, path = "/edits/batches", tag = "edits", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    params(PageQuery),
    responses((status = 200, body = Page<EditBatch>)))]
pub async fn list_batches(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<EditBatch>>, Problem> {
    let limit = page.limit()?;
    let after: Option<(DateTime<Utc>, Uuid)> = page.after()?;
    let rows = db::edits::batches(state.db(), family.family_id, family.user_id, after, limit + 1).await?;
    let keys: Vec<(DateTime<Utc>, Uuid)> = rows.iter().map(|r| (r.at, r.batch_id)).collect();
    let items = rows
        .into_iter()
        .map(|b| EditBatch {
            batch_id: b.batch_id,
            user_id: b.user_id,
            source: b.source,
            at: b.at,
            items: b.items,
            first_label: b.first_label,
            undone: b.undone,
        })
        .collect::<Vec<_>>();
    let mut out = Page::from_rows(items, limit, |b| b.batch_id);
    out.next = out.next.as_ref().and_then(|_| keys.get(out.items.len().saturating_sub(1)).map(crate::v2::page::encode));
    Ok(Json(out))
}

#[derive(Serialize, ToSchema)]
pub struct Undone {
    /// Songs, albums and contributors put back.
    pub items: usize,
    /// The undo's own batch, itself in the history (and undoable).
    pub batch_id: Uuid,
}

async fn undo_rows(state: &AppState, family: &FamilyContext, rows: Vec<EditRow>) -> Result<Json<Undone>, Problem> {
    if rows.is_empty() {
        return Err(Problem::not_found());
    }
    if rows.iter().any(|e| e.item_kind == "track" && e.item_owner.is_some_and(|o| o != family.user_id)) {
        return Err(Problem::forbidden().detail("only a song's owner undoes its changes"));
    }
    match db::edits::undo(state.db(), &rows, family.user_id).await? {
        UndoOutcome::Undone { items, batch_id } => Ok(Json(Undone { items, batch_id })),
        UndoOutcome::AlreadyUndone => Err(Problem::new(StatusCode::CONFLICT, "already_undone").detail("this change was already undone")),
        UndoOutcome::Gone => Err(Problem::new(StatusCode::CONFLICT, "gone").detail("the song, album or contributor is gone")),
        UndoOutcome::ChangedSince { item_kind, item_id, fields } => Err(Problem::new(StatusCode::CONFLICT, "changed_since")
            .detail(format!("changed again since: {}", fields.join(", ")))
            .with("item_kind", item_kind)
            .with("item_id", item_id.to_string())
            .with("fields", fields.join(","))),
    }
}

/// Undo one change: its fields go back to what they were, while they still
/// hold what it wrote (else `409 changed_since`, naming them).
#[utoipa::path(post, path = "/edits/{id}/undo", tag = "edits", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Undone), (status = 403, body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody), (status = 409, body = crate::v2::problem::ProblemBody)))]
pub async fn undo_edit(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Undone>, Problem> {
    let rows = db::edits::find(state.db(), family.family_id, family.user_id, id, false).await?;
    undo_rows(&state, &family, rows).await
}

/// Undo a whole save — every change in the batch, all or nothing.
#[utoipa::path(post, path = "/edits/batches/{id}/undo", tag = "edits", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Undone), (status = 403, body = crate::v2::problem::ProblemBody),
        (status = 404, body = crate::v2::problem::ProblemBody), (status = 409, body = crate::v2::problem::ProblemBody)))]
pub async fn undo_batch(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Undone>, Problem> {
    let rows = db::edits::find(state.db(), family.family_id, family.user_id, id, true).await?;
    undo_rows(&state, &family, rows).await
}
