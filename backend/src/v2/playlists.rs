// SPDX-License-Identifier: AGPL-3.0-or-later
//! Playlists (docs/api-v2-plan.md, V10): a member's own, private or shared
//! with the household. Anyone who can see one reads it; only its owner changes
//! it, and its owner or a family admin deletes it (to the trash, as on v1).
//!
//! Entries have ids of their own, since a track may be in a playlist twice;
//! removing and reordering name entries, not tracks. A reader is shown only
//! the entries whose tracks they can see, and counts and durations are of
//! those. Playlists are not in the change feed yet: a client reads them when
//! it shows them.

use crate::app::AppState;
use crate::db::access::{self, visibility_of};
use crate::db::catalog::{self, PlaylistRow};
use crate::db::trash::Kind as TrashKind;
use crate::families::FamilyContext;
use crate::v2::music::{Included, Track, included, tracks, wanted};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The most tracks one request adds.
const MAX_ADD: usize = 1000;

/// An entry without its track: id, position, when added.
type EntryHead = (Uuid, i32, DateTime<Utc>);

#[derive(Serialize, ToSchema)]
pub struct Playlist {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// `private` or `family`.
    pub visibility: String,
    pub owner_id: Uuid,
    /// Of the entries the caller can see.
    pub track_count: i64,
    pub duration_ms: i64,
    /// Its cover, or its first track's (`GET /images/{id}`; add `w` for the size).
    pub image: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn playlist(state: &AppState, row: PlaylistRow) -> Playlist {
    Playlist {
        id: row.id,
        name: row.name,
        description: row.description,
        visibility: visibility_of(row.family_id).to_string(),
        owner_id: row.user_id,
        track_count: row.track_count,
        duration_ms: row.duration_secs * 1000,
        image: crate::v2::images::maybe(state, row.cover_object_id.or(row.first_cover)),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

/// One place in a playlist.
#[derive(Serialize, ToSchema)]
pub struct Entry {
    /// The entry's own id, for removing or moving it.
    pub id: Uuid,
    /// 1-based, in order; positions of entries the caller cannot see are skipped.
    pub position: i32,
    pub added_at: DateTime<Utc>,
    pub track: Track,
}

async fn find(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<PlaylistRow, Problem> {
    catalog::find_playlist(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)
}

/// Seen (`404` otherwise) and the caller's (`403` otherwise).
async fn owned(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<PlaylistRow, Problem> {
    let row = find(state, family, id).await?;
    if row.user_id != family.user_id {
        return Err(Problem::forbidden().detail("only its owner changes a playlist"));
    }
    Ok(row)
}

fn name_of(name: &str) -> Result<String, Problem> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(Problem::bad_request("invalid_body", "a name is 1 to 200 characters").field("name", "invalid", "1 to 200 characters"));
    }
    Ok(name.to_string())
}

fn description_of(description: Option<&str>) -> Result<Option<String>, Problem> {
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    if description.is_some_and(|d| d.chars().count() > 2000) {
        return Err(Problem::bad_request("invalid_body", "a description is at most 2000 characters").field("description", "invalid", "at most 2000 characters"));
    }
    Ok(description.map(str::to_string))
}

fn family_for(visibility: Option<&str>, family: &FamilyContext) -> Result<Option<Uuid>, Problem> {
    access::family_id_for_visibility(visibility, family.family_id)
        .map_err(|e| Problem::bad_request("invalid_body", e).field("visibility", "invalid", "private or family"))
}

fn check_added(track_ids: &[Uuid]) -> Result<(), Problem> {
    if track_ids.len() > MAX_ADD {
        return Err(Problem::bad_request("too_many", format!("at most {MAX_ADD} tracks at a time")));
    }
    Ok(())
}

// ── Reading ──────────────────────────────────────────────────────────────────

/// Playlists the caller can see: their own and those shared with them, by name.
#[utoipa::path(get, path = "/playlists", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<Playlist>)))]
pub async fn list_playlists(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<Playlist>>, Problem> {
    let limit = page.limit()?;
    let rows = catalog::list_playlists(state.db(), family.viewer(), None, page.after::<(String, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |p| (p.sort_key.clone(), p.id));
    Ok(Json(Page { items: page.items.into_iter().map(|p| playlist(&state, p)).collect(), next: page.next, included: None }))
}

/// One playlist.
#[utoipa::path(get, path = "/playlists/{id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Playlist), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_playlist(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Playlist>, Problem> {
    Ok(Json(playlist(&state, find(&state, &family, id).await?)))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EntriesQuery {
    /// `albums`, `contributors`, `genres`, comma-separated.
    pub include: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// A playlist's entries in order, each with its track.
#[utoipa::path(get, path = "/playlists/{id}/entries", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path), EntriesQuery),
    responses((status = 200, body = Page<Entry>), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn list_entries(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<EntriesQuery>,
) -> Result<Json<Page<Entry>>, Problem> {
    let w = wanted(q.include.as_deref(), &["albums", "contributors", "genres"])?;
    find(&state, &family, id).await?;
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let viewer = family.viewer();
    let rows = catalog::playlist_entries(state.db(), viewer, id, page.after::<i32>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |(_, position, _, _)| *position);
    let (heads, track_rows): (Vec<EntryHead>, Vec<catalog::TrackRow>) =
        page.items.into_iter().map(|(entry, position, added, track)| ((entry, position, added), track)).unzip();
    let records = tracks(&state, track_rows).await?;
    let credit_ids: Vec<Uuid> = records.iter().flat_map(|t| t.credits.iter().map(|c| c.contributor_id)).collect();
    let album_ids: Vec<Uuid> = records.iter().filter_map(|t| t.album_id).collect();
    let genre_ids: Vec<Uuid> = records.iter().flat_map(|t| t.genre_ids.iter().copied()).collect();
    let included: Option<Included> =
        included(&state, viewer, &w, credit_ids.into_iter(), album_ids.into_iter(), std::iter::empty(), genre_ids.into_iter()).await?;
    let items = heads.into_iter().zip(records).map(|((id, position, added_at), track)| Entry { id, position, added_at, track }).collect();
    Ok(Json(Page { items, next: page.next, included }))
}

// ── Changing ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct CreatePlaylist {
    pub name: String,
    pub description: Option<String>,
    /// `private` (the default) or `family`.
    pub visibility: Option<String>,
    /// Tracks to start with, in order; those the caller cannot see are left out.
    #[serde(default)]
    pub track_ids: Vec<Uuid>,
}

/// Make a playlist.
#[utoipa::path(post, path = "/playlists", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = CreatePlaylist,
    responses((status = 201, body = Playlist), (status = 400, description = "`invalid_body`, `too_many`", body = crate::v2::problem::ProblemBody)))]
pub async fn create_playlist(family: FamilyContext, State(state): State<AppState>, Json(body): Json<CreatePlaylist>) -> Result<(StatusCode, Json<Playlist>), Problem> {
    let name = name_of(&body.name)?;
    let description = description_of(body.description.as_deref())?;
    let family_id = family_for(body.visibility.as_deref(), &family)?;
    check_added(&body.track_ids)?;
    let made = crate::db::music::insert_playlist(state.db(), family.user_id, family_id, &name, description.as_deref(), false).await?;
    if !body.track_ids.is_empty() {
        crate::db::music::add_tracks_to_playlist(state.db(), made.id, &body.track_ids, family.viewer()).await?;
    }
    Ok((StatusCode::CREATED, Json(playlist(&state, find(&state, &family, made.id).await?))))
}

/// `null` sets a field to nothing; leaving it out keeps it.
fn nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Deserialize, ToSchema)]
pub struct PatchPlaylist {
    pub name: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// `private` or `family`.
    pub visibility: Option<String>,
}

/// Rename a playlist, describe it, share it or stop sharing it.
#[utoipa::path(patch, path = "/playlists/{id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = PatchPlaylist,
    responses((status = 200, body = Playlist),
        (status = 400, description = "`invalid_body`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_playlist(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchPlaylist>,
) -> Result<Json<Playlist>, Problem> {
    let current = owned(&state, &family, id).await?;
    let name = match body.name.as_deref() {
        Some(n) => name_of(n)?,
        None => current.name,
    };
    let description = match body.description {
        Some(d) => description_of(d.as_deref())?,
        None => current.description,
    };
    let family_id = match body.visibility.as_deref() {
        Some(v) => family_for(Some(v), &family)?,
        None => current.family_id,
    };
    catalog::update_playlist(state.db(), id, family.user_id, &name, description.as_deref(), family_id).await?;
    Ok(Json(playlist(&state, find(&state, &family, id).await?)))
}

/// Delete a playlist: it goes to the trash, as on v1. Its owner may, and a
/// family admin for one shared with the household.
#[utoipa::path(delete, path = "/playlists/{id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "In the trash"),
        (status = 403, description = "Neither its owner nor, for a shared one, a family admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_playlist(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    find(&state, &family, id).await?;
    crate::trash::move_to_trash(&state, family.viewer(), None, TrashKind::Playlist, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct AddEntries {
    /// Appended in this order; a track the caller cannot see is left out.
    pub track_ids: Vec<Uuid>,
}

/// Add tracks to the end of a playlist.
#[utoipa::path(post, path = "/playlists/{id}/entries", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = AddEntries,
    responses((status = 200, description = "The playlist now", body = Playlist),
        (status = 400, description = "`too_many`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn add_entries(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AddEntries>,
) -> Result<Json<Playlist>, Problem> {
    owned(&state, &family, id).await?;
    check_added(&body.track_ids)?;
    if crate::db::music::add_tracks_to_playlist(state.db(), id, &body.track_ids, family.viewer()).await? > 0 {
        catalog::touch_playlist(state.db(), id).await?;
    }
    Ok(Json(playlist(&state, find(&state, &family, id).await?)))
}

/// Take one entry out of a playlist.
#[utoipa::path(delete, path = "/playlists/{id}/entries/{entry_id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path), ("entry_id" = Uuid, Path)),
    responses((status = 204, description = "Removed"),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown playlist or entry", body = crate::v2::problem::ProblemBody)))]
pub async fn remove_entry(family: FamilyContext, State(state): State<AppState>, Path((id, entry_id)): Path<(Uuid, Uuid)>) -> Result<StatusCode, Problem> {
    owned(&state, &family, id).await?;
    if !catalog::remove_playlist_entry(state.db(), id, entry_id).await? {
        return Err(Problem::not_found());
    }
    catalog::touch_playlist(state.db(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct OrderEntries {
    /// Entry ids in their new order. Entries left out follow them in the order
    /// they had, so a client that sees only some entries can still reorder those.
    pub entry_ids: Vec<Uuid>,
}

/// Reorder a playlist.
#[utoipa::path(put, path = "/playlists/{id}/entries/order", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = OrderEntries,
    responses((status = 204, description = "Reordered"),
        (status = 400, description = "`invalid_entry`: an id twice, or not this playlist's", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn order_entries(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<OrderEntries>,
) -> Result<StatusCode, Problem> {
    owned(&state, &family, id).await?;
    if !catalog::order_playlist(state.db(), id, &body.entry_ids).await? {
        return Err(Problem::bad_request("invalid_entry", "every id once, each an entry of this playlist").field("entry_ids", "invalid", "unknown or repeated entry"));
    }
    Ok(StatusCode::NO_CONTENT)
}
