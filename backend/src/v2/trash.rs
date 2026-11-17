// SPDX-License-Identifier: AGPL-3.0-or-later
//! Deleting and the 30-day trash (docs/api-v2-completion-plan.md, phase 4):
//! delete a track or a book, delete many things as one batch, list the trash
//! page by page, restore one item or a whole batch, purge one, empty it.
//!
//! The work is v1's (`crate::trash`), called rather than copied: who may
//! delete, restore and purge (the owner, or a household admin for what is
//! shared with the household), the notification to an owner whose item an
//! admin deleted, the edition's restore charge, deleting the stored objects.
//! What v2 changes: the kinds are v2's words, one request is one batch (no
//! `X-Trash-Batch` header), the list is cursor-paged, and every answer is v2's.
//!
//! The change feed needs nothing from here: the triggers on the `…_all`
//! tables log a deletion, a restore and a purge, with the album, recording and
//! contributors the item makes visible (migrations 0109, 0126).

use crate::app::AppState;
use crate::db::trash::{Kind, Scope, Trashed};
use crate::db;
use crate::families::FamilyContext;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use crate::v2::state::ItemResult;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The most items one `POST /trash` takes: discovery's `limits.batch_max`.
pub const BATCH_MAX: usize = 500;

/// Rows read at a time while emptying the trash.
const EMPTY_CHUNK: i64 = 200;

/// The v2 word for each kind the trash holds.
pub(crate) fn kind_word(kind: Kind) -> &'static str {
    match kind {
        Kind::MusicTrack => "track",
        Kind::Audiobook => "book",
        Kind::Playlist => "playlist",
        Kind::PodcastEpisode => "episode",
        Kind::CompanionFile => "file",
        Kind::Podcast => "podcast",
    }
}

pub(crate) fn parse_kind(word: &str) -> Option<Kind> {
    match word {
        "track" => Some(Kind::MusicTrack),
        "book" => Some(Kind::Audiobook),
        "playlist" => Some(Kind::Playlist),
        "episode" => Some(Kind::PodcastEpisode),
        "file" => Some(Kind::CompanionFile),
        "podcast" => Some(Kind::Podcast),
        _ => None,
    }
}

fn invalid_kind(word: &str) -> Problem {
    Problem::bad_request("invalid_kind", "kind is `track`, `book`, `playlist`, `episode`, `file` or `podcast`").field("kind", "invalid", word.to_string())
}

// ── Deleting ────────────────────────────────────────────────────────────────

async fn delete_one(state: &AppState, family: &FamilyContext, kind: Kind, id: Uuid) -> Result<StatusCode, Problem> {
    crate::trash::move_to_trash_in(state, family.viewer(), Uuid::new_v4(), kind, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Deletes a track: into the trash for 30 days, restorable until then; it leaves every list,
/// and the change feed sends its removal. Its owner may, and a household admin for one shared
/// with the household. A household admin deleting someone else's track is told to its owner.
#[utoipa::path(delete, path = "/tracks/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "In the trash"),
        (status = 403, description = "Visible, but neither its owner nor, for a shared one, a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown, not visible, or already in the trash", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_track(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    delete_one(&state, &family, Kind::MusicTrack, id).await
}

/// Deletes a book with its files: into the trash for 30 days, as a track.
#[utoipa::path(delete, path = "/books/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "In the trash"),
        (status = 403, description = "Visible, but neither its owner nor, for a shared one, a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown, not visible, or already in the trash", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_book(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    delete_one(&state, &family, Kind::Audiobook, id).await
}

#[derive(Deserialize, ToSchema, Debug, Clone)]
pub struct TrashRef {
    /// `track`, `book`, `playlist`, `episode` (the server's stored copy of the episode; the
    /// episode stays listed), `file` (a companion file of the own.audio folder) or `podcast` (an
    /// uploaded show with its episodes; a followed one is unfollowed, not trashed).
    pub kind: String,
    pub id: Uuid,
}

#[derive(Deserialize, ToSchema)]
pub struct TrashRequest {
    /// Up to `limits.batch_max` (500).
    pub items: Vec<TrashRef>,
}

#[derive(Serialize, ToSchema)]
pub struct TrashBatch {
    /// What restores all of it at once: `POST /trash/batches/{batch}/restore`.
    pub batch: Uuid,
    /// One per item, in order: `client_id` is the item's id; `status` is `ok`, `duplicate`
    /// (named twice in the request), or `not_found`, `forbidden`, `invalid_kind`.
    pub items: Vec<ItemResult>,
}

fn result(id: Uuid, status: &str) -> ItemResult {
    ItemResult { client_id: id.to_string(), status: status.to_string(), detail: None }
}

/// An item of the request: its kind and id, or the id of one whose kind is unknown.
pub(crate) type Checked = Result<(Kind, Uuid), Uuid>;

/// The request as kinds, before anything is asked of the database: a known kind each, at most
/// [`BATCH_MAX`]. An unknown kind is that item's answer, not the request's.
pub(crate) fn check_request(items: &[TrashRef]) -> Result<Vec<Checked>, Problem> {
    if items.is_empty() {
        return Err(Problem::bad_request("invalid_body", "items names at least one thing to delete").field("/items", "empty", ""));
    }
    if items.len() > BATCH_MAX {
        return Err(Problem::bad_request("too_many", format!("at most {BATCH_MAX} items at a time")).field("/items", "too_many", format!("at most {BATCH_MAX}")));
    }
    Ok(items.iter().map(|i| parse_kind(&i.kind).map(|k| (k, i.id)).ok_or(i.id)).collect())
}

/// Deletes several things as one batch: each into the trash for 30 days, all restorable at
/// once with the batch. Each item answers on its own, by the rule of a single delete (its
/// owner, or a household admin for what is shared). Send an `Idempotency-Key` to retry safely:
/// a retry answers the first answer, the same batch.
#[utoipa::path(post, path = "/trash", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = TrashRequest,
    responses((status = 200, description = "One result per item, in order", body = TrashBatch),
        (status = 400, description = "`invalid_body` (no items) or `too_many`", body = crate::v2::problem::ProblemBody),
        (status = 422, description = "`idempotency_key_reused`: the key came with another request", body = crate::v2::problem::ProblemBody)))]
pub async fn trash_many(family: FamilyContext, State(state): State<AppState>, Json(body): Json<TrashRequest>) -> Result<Json<TrashBatch>, Problem> {
    let wanted = check_request(&body.items)?;
    let batch = Uuid::new_v4();
    let mut seen = std::collections::HashSet::new();
    let mut items = Vec::with_capacity(wanted.len());
    for item in wanted {
        let (kind, id) = match item {
            Ok(k) => k,
            Err(id) => {
                items.push(result(id, "invalid_kind"));
                continue;
            }
        };
        if !seen.insert((kind, id)) {
            items.push(result(id, "duplicate"));
            continue;
        }
        let outcome = crate::trash::move_to_trash_in(&state, family.viewer(), batch, kind, id).await.map_err(Problem::from);
        items.push(match outcome {
            Ok(()) => result(id, "ok"),
            Err(p) if p.status().is_client_error() => result(id, p.code()),
            Err(p) => return Err(p),
        });
    }
    Ok(Json(TrashBatch { batch, items }))
}

// ── Listing ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TrashQuery {
    /// `mine` (the default): what the caller owns, whoever deleted it. `family`: everything
    /// shared with the household, for a household admin.
    pub scope: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

fn scope_of(family: &FamilyContext, raw: Option<&str>) -> Result<Scope, Problem> {
    match raw {
        None | Some("mine") => Ok(Scope::Owner(family.user_id)),
        Some("family") => {
            family.require_family_admin()?;
            Ok(Scope::Family(family.family_id))
        }
        Some(other) => Err(Problem::bad_request("invalid_scope", "scope is `mine` or `family`").field("scope", "invalid", other.to_string())),
    }
}

#[derive(Serialize, ToSchema)]
pub struct TrashItem {
    /// `track`, `book`, `playlist`, `episode` or `file`.
    pub kind: String,
    pub id: Uuid,
    pub title: String,
    /// Whose it is: a `member` record.
    pub owner_id: Uuid,
    /// Who deleted it, when that account still exists.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub deleted_by: Option<Uuid>,
    /// Deleted together with the other items of this batch.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub batch: Option<Uuid>,
    pub deleted_at: DateTime<Utc>,
    /// When the server deletes it for good, 30 days on.
    pub purge_at: DateTime<Utc>,
    /// What it keeps in storage.
    pub size_bytes: i64,
    /// What restoring it now would charge, in millionths of the household's credit currency,
    /// so a client can say so before the restore. Always 0 where restoring is free (an
    /// open-source server).
    pub restore_charge_micro: i64,
}

async fn item(state: &AppState, t: Trashed) -> Result<TrashItem, Problem> {
    let restore_charge_micro = crate::trash::restore_quote(state, t.size_bytes, t.trashed_at).await?;
    Ok(TrashItem {
        kind: kind_word(t.kind()).to_string(),
        id: t.id,
        purge_at: t.purge_at(),
        title: t.title,
        owner_id: t.owner_id,
        deleted_by: t.trashed_by,
        batch: t.trash_batch,
        deleted_at: t.trashed_at,
        size_bytes: t.size_bytes,
        restore_charge_micro,
    })
}

/// What is in the trash, newest first: the caller's own things (`scope=mine`), or everything
/// shared with the household (`scope=family`, a household admin's view).
#[utoipa::path(get, path = "/trash", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(TrashQuery),
    responses((status = 200, body = Page<TrashItem>),
        (status = 400, description = "`invalid_scope`, `invalid_limit` or `invalid_cursor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`scope=family` without being a household admin", body = crate::v2::problem::ProblemBody)))]
pub async fn list_trash(family: FamilyContext, State(state): State<AppState>, Query(q): Query<TrashQuery>) -> Result<Json<Page<TrashItem>>, Problem> {
    let scope = scope_of(&family, q.scope.as_deref())?;
    let page = PageQuery { limit: q.limit, cursor: q.cursor.clone() };
    let limit = page.limit()?;
    let rows = db::trash::page(state.db(), scope, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |t| (t.trashed_at, t.kind.clone(), t.id));
    let mut items = Vec::with_capacity(page.items.len());
    for t in page.items {
        items.push(item(&state, t).await?);
    }
    Ok(Json(Page { items, next: page.next, included: None }))
}

// ── Restoring ───────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct TrashRestored {
    pub batch: Uuid,
    /// One per item of the batch the caller may restore: `client_id` is the item's id, `status`
    /// `ok` (or `not_found` when it left the trash meanwhile).
    pub items: Vec<ItemResult>,
    /// What the restore charged, in millionths of the household's credit currency; 0 where
    /// restoring is free.
    pub charged_micro: i64,
}

/// Restores everything deleted in one batch that the caller may restore (their own, and what
/// is shared with the household for a household admin). Each comes back as it was, and the
/// change feed sends it again.
#[utoipa::path(post, path = "/trash/batches/{batch}/restore", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("batch" = Uuid, Path)),
    responses((status = 200, body = TrashRestored),
        (status = 404, description = "Nothing in the batch the caller may restore", body = crate::v2::problem::ProblemBody)))]
pub async fn restore_batch(family: FamilyContext, State(state): State<AppState>, Path(batch): Path<Uuid>) -> Result<Json<TrashRestored>, Problem> {
    // A batch is what one request deleted: at most `batch_max` through v2, a client's own
    // gesture through v1's `X-Trash-Batch`.
    let rows = db::trash::list_batch(state.db(), batch).await?;
    let mut items = Vec::new();
    let mut charged_micro = 0;
    let viewer = family.viewer();
    for row in rows.iter().filter(|r| crate::trash::may_manage(viewer, r.owner_id, r.family_id)) {
        match crate::trash::restore_item(&state, &family, row).await? {
            Some(charged) => {
                charged_micro += charged;
                items.push(result(row.id, "ok"));
            }
            None => items.push(result(row.id, "not_found")),
        }
    }
    if !items.iter().any(|i| i.status == "ok") {
        return Err(Problem::not_found());
    }
    Ok(Json(TrashRestored { batch, items, charged_micro }))
}

/// Restores one item, as it was; the change feed sends it again.
#[utoipa::path(post, path = "/trash/{kind}/{id}/restore", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("kind" = String, Path, description = "`track`, `book`, `playlist`, `episode` or `file`"), ("id" = Uuid, Path)),
    responses((status = 204, description = "Restored"),
        (status = 400, description = "`invalid_kind`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not in the trash, or not the caller's to restore", body = crate::v2::problem::ProblemBody)))]
pub async fn restore_one(family: FamilyContext, State(state): State<AppState>, Path((kind, id)): Path<(String, Uuid)>) -> Result<StatusCode, Problem> {
    let kind = parse_kind(&kind).ok_or_else(|| invalid_kind(&kind))?;
    let row = crate::trash::managed_trashed(&state, &family, kind, id).await?;
    crate::trash::restore_item(&state, &family, &row).await?.ok_or_else(Problem::not_found)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Deleting for good ───────────────────────────────────────────────────────

/// Deletes one item of the trash for good, now, with what it keeps in storage. Its owner, or a
/// household admin for something that was shared with the household.
#[utoipa::path(delete, path = "/trash/{kind}/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("kind" = String, Path, description = "`track`, `book`, `playlist`, `episode` or `file`"), ("id" = Uuid, Path)),
    responses((status = 204, description = "Deleted for good"),
        (status = 400, description = "`invalid_kind`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Not in the trash, or not the caller's to delete", body = crate::v2::problem::ProblemBody)))]
pub async fn purge_one(family: FamilyContext, State(state): State<AppState>, Path((kind, id)): Path<(String, Uuid)>) -> Result<StatusCode, Problem> {
    let kind = parse_kind(&kind).ok_or_else(|| invalid_kind(&kind))?;
    crate::trash::managed_trashed(&state, &family, kind, id).await?;
    if !crate::trash::purge(state.db(), state.storage(), kind, id).await? {
        return Err(Problem::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EmptyQuery {
    /// `mine` (the default) or `family` (everything shared with the household, for a household
    /// admin), as in `GET /trash`.
    pub scope: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TrashEmptied {
    /// How many items were deleted for good.
    pub purged: i64,
}

/// Empties the trash: everything in it the caller owns (`scope=mine`), or everything shared
/// with the household (`scope=family`, a household admin), deleted for good now.
#[utoipa::path(delete, path = "/trash", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(EmptyQuery),
    responses((status = 200, body = TrashEmptied),
        (status = 400, description = "`invalid_scope`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "`scope=family` without being a household admin", body = crate::v2::problem::ProblemBody)))]
pub async fn empty_trash(family: FamilyContext, State(state): State<AppState>, Query(q): Query<EmptyQuery>) -> Result<Json<TrashEmptied>, Problem> {
    let scope = scope_of(&family, q.scope.as_deref())?;
    let viewer = family.viewer();
    let mut purged = 0;
    let mut after = None;
    // Page by page, never the whole trash in memory; a purged row is gone, so the walk only
    // moves forward.
    loop {
        let rows = db::trash::page(state.db(), scope, after.take(), EMPTY_CHUNK).await?;
        let Some(last) = rows.last() else { break };
        after = Some((last.trashed_at, last.kind.clone(), last.id));
        for row in rows.iter().filter(|r| crate::trash::may_manage(viewer, r.owner_id, r.family_id)) {
            if crate::trash::purge(state.db(), state.storage(), row.kind(), row.id).await? {
                purged += 1;
            }
        }
        if (rows.len() as i64) < EMPTY_CHUNK {
            break;
        }
    }
    Ok(Json(TrashEmptied { purged }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_trash_kind_has_a_v2_word_and_back() {
        for k in Kind::ALL {
            assert_eq!(parse_kind(kind_word(k)), Some(k));
        }
        assert_eq!(parse_kind("music_track"), None, "v1's word is not v2's");
        assert_eq!(parse_kind("podcast"), Some(Kind::Podcast), "an uploaded show is, since phase 11");
        assert_eq!(parse_kind("show"), None);
    }

    fn r(kind: &str) -> TrashRef {
        TrashRef { kind: kind.into(), id: Uuid::new_v4() }
    }

    #[test]
    fn a_request_names_one_to_batch_max_items() {
        assert_eq!(check_request(&[]).unwrap_err().code(), "invalid_body");
        let many: Vec<TrashRef> = (0..=BATCH_MAX).map(|_| r("track")).collect();
        assert_eq!(check_request(&many).unwrap_err().code(), "too_many");
        assert_eq!(check_request(&many[..BATCH_MAX]).unwrap().len(), BATCH_MAX);
    }

    #[test]
    fn an_unknown_kind_is_that_items_answer() {
        let items = [r("track"), r("teleport"), r("episode")];
        let checked = check_request(&items).unwrap();
        assert_eq!(checked[0], Ok((Kind::MusicTrack, items[0].id)));
        assert_eq!(checked[1], Err(items[1].id));
        assert_eq!(checked[2], Ok((Kind::PodcastEpisode, items[2].id)));
    }

    #[test]
    fn batch_max_is_what_discovery_says() {
        assert_eq!(BATCH_MAX, 500);
    }
}
