// SPDX-License-Identifier: AGPL-3.0-or-later
//! Smart playlists (docs/api-v2-plan.md, V11): a saved rule, not a list of
//! tracks (docs/music-signals-and-smart-playlists-plan.md §4). The rule is v1's,
//! unchanged (`music::rules::Rule`).
//!
//! What a rule holds is worked out each time it is played, **from the caller's
//! own history**: a shared "not played for a year" is a different list for each
//! member, and a client should say so. Freezing makes the other kind of
//! sharing — an ordinary playlist, fixed and editable. Only the owner changes or
//! deletes a smart playlist; deleting one is final (there is nothing to restore
//! but a rule).

use crate::app::AppState;
use crate::db::access::visibility_of;
use crate::db::{self, catalog};
use crate::families::FamilyContext;
use crate::music::rules::Rule;
use crate::v2::music::{Included, Track, included, tracks, wanted};
use crate::v2::page::Page;
use crate::v2::playlists::Playlist;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// How many matching tracks may be considered before sequencing: room for the
/// per-artist and energy-arc constraints to still fill the requested length.
const CANDIDATE_POOL: i64 = 4_000;

#[derive(Serialize, ToSchema)]
pub struct SmartPlaylist {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// The rule: `filters`, `limit`, `arc`, `sequence`, `sort` (as v1's).
    #[schema(value_type = Object)]
    pub rule: serde_json::Value,
    /// `dynamic`, or `frozen` once made into an ordinary playlist.
    pub mode: String,
    /// `private` or `family`.
    pub visibility: String,
    pub owner_id: Uuid,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn smart(row: db::music::SmartPlaylist) -> SmartPlaylist {
    SmartPlaylist {
        id: row.id,
        name: row.name,
        description: row.description,
        rule: row.rule,
        mode: row.mode,
        visibility: visibility_of(row.family_id).to_string(),
        owner_id: row.user_id,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn find(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<db::music::SmartPlaylist, Problem> {
    db::music::find_smart_playlist(state.db(), id, family.user_id, family.family_id).await?.ok_or_else(Problem::not_found)
}

async fn owned(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<db::music::SmartPlaylist, Problem> {
    let row = find(state, family, id).await?;
    if row.user_id != family.user_id {
        return Err(Problem::forbidden().detail("only its owner changes a smart playlist"));
    }
    Ok(row)
}

fn checked(rule: &Rule) -> Result<serde_json::Value, Problem> {
    rule.validate().map_err(|e| Problem::bad_request("invalid_rule", e.to_string()).field("rule", "invalid", e.to_string()))?;
    serde_json::to_value(rule).map_err(Problem::internal)
}

fn name_of(name: &str) -> Result<String, Problem> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(Problem::bad_request("invalid_body", "a name is 1 to 200 characters").field("name", "invalid", "1 to 200 characters"));
    }
    Ok(name.to_string())
}

fn family_for(visibility: Option<&str>, family: &FamilyContext) -> Result<Option<Uuid>, Problem> {
    db::access::family_id_for_visibility(visibility, family.family_id)
        .map_err(|e| Problem::bad_request("invalid_body", e).field("visibility", "invalid", "private or family"))
}

// ── Reading ──────────────────────────────────────────────────────────────────

/// The caller's smart playlists and those shared with the household, by name.
#[utoipa::path(get, path = "/smart-playlists", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, description = "All of them: a short list, so one page", body = Page<SmartPlaylist>)))]
pub async fn list_smart(family: FamilyContext, State(state): State<AppState>) -> Result<Json<Page<SmartPlaylist>>, Problem> {
    let mut rows = db::music::list_smart_playlists(state.db(), family.user_id, family.family_id).await?;
    rows.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()).then(a.id.cmp(&b.id)));
    Ok(Json(Page { items: rows.into_iter().map(smart).collect(), next: None, included: None }))
}

/// One smart playlist.
#[utoipa::path(get, path = "/smart-playlists/{id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = SmartPlaylist), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_smart(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<SmartPlaylist>, Problem> {
    Ok(Json(smart(find(&state, &family, id).await?)))
}

#[derive(Serialize, ToSchema)]
pub struct Preset {
    pub slug: String,
    pub name: String,
    #[schema(value_type = Object)]
    pub rule: serde_json::Value,
}

/// Ready-made rules, the same for everyone: to play as they are, or to save
/// and change.
#[utoipa::path(get, path = "/smart-playlists/presets", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Vec<Preset>)))]
pub async fn presets(_family: FamilyContext) -> Result<Json<Vec<Preset>>, Problem> {
    crate::music::rules::presets()
        .into_iter()
        .map(|(slug, name, rule)| Ok(Preset { slug: slug.into(), name: name.into(), rule: serde_json::to_value(rule).map_err(Problem::internal)? }))
        .collect::<Result<Vec<_>, Problem>>()
        .map(Json)
}

// ── Playing ──────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Resolved {
    /// In the order to play them.
    pub items: Vec<Track>,
    pub duration_ms: i64,
    /// How many tracks matched before sequencing: few, with a short list, means
    /// the rule is narrow rather than that something failed.
    pub candidates: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Included>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ResolveQuery {
    /// `albums`, `contributors`, `genres`.
    pub include: Option<String>,
}

async fn resolve(state: &AppState, family: &FamilyContext, rule: &Rule, include: Option<&str>) -> Result<Resolved, Problem> {
    let w = wanted(include, &["albums", "contributors", "genres"])?;
    let viewer = family.viewer();
    let candidates = db::music::smart_playlist_candidates(state.db(), viewer, rule, CANDIDATE_POOL).await?;
    let ids = crate::music::sequence::build(&candidates, rule, rand::random::<u64>());
    // One lookup, then back into the sequence: the order is the point.
    let mut found: std::collections::HashMap<Uuid, catalog::TrackRow> =
        catalog::tracks_by_ids(state.db(), viewer, &ids).await?.into_iter().map(|t| (t.id, t)).collect();
    let rows: Vec<catalog::TrackRow> = ids.iter().filter_map(|id| found.remove(id)).collect();
    let items = tracks(state, rows).await?;
    let duration_ms = items.iter().filter_map(|t| t.duration_ms).sum();
    let credit_ids: Vec<Uuid> = items.iter().flat_map(|t| t.credits.iter().map(|c| c.contributor_id)).collect();
    let album_ids: Vec<Uuid> = items.iter().filter_map(|t| t.album_id).collect();
    let genre_ids: Vec<Uuid> = items.iter().flat_map(|t| t.genre_ids.iter().copied()).collect();
    let included = included(state, viewer, &w, credit_ids.into_iter(), album_ids.into_iter(), std::iter::empty(), genre_ids.into_iter()).await?;
    Ok(Resolved { items, duration_ms, candidates: candidates.len(), included })
}

/// What a smart playlist holds now, for the caller, in the order to play it.
/// A new draw each time (the default order is weighted random).
#[utoipa::path(post, path = "/smart-playlists/{id}/resolve", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("library:read"))),
    params(("id" = Uuid, Path), ResolveQuery),
    responses((status = 200, body = Resolved), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn resolve_smart(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<ResolveQuery>,
) -> Result<Json<Resolved>, Problem> {
    let row = find(&state, &family, id).await?;
    let rule: Rule = serde_json::from_value(row.rule).map_err(|e| Problem::internal(anyhow::anyhow!("stored rule is not valid: {e}")))?;
    resolve(&state, &family, &rule, q.include.as_deref()).await.map(Json)
}

#[derive(Deserialize, ToSchema)]
pub struct RuleRequest {
    #[schema(value_type = Object)]
    pub rule: Rule,
}

/// Try a rule without saving it: what an editor shows while it is changed.
#[utoipa::path(post, path = "/smart-playlists/resolve", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member")), ("x-scope" = json!("library:read"))),
    params(ResolveQuery), request_body = RuleRequest,
    responses((status = 200, body = Resolved), (status = 400, description = "`invalid_rule`", body = crate::v2::problem::ProblemBody)))]
pub async fn resolve_rule(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<ResolveQuery>,
    Json(body): Json<RuleRequest>,
) -> Result<Json<Resolved>, Problem> {
    checked(&body.rule)?;
    resolve(&state, &family, &body.rule, q.include.as_deref()).await.map(Json)
}

// ── Changing ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct CreateSmart {
    pub name: String,
    pub description: Option<String>,
    #[schema(value_type = Object)]
    pub rule: Rule,
    /// `private` (the default) or `family`: each member then gets their own
    /// result from it.
    pub visibility: Option<String>,
}

/// Save a rule as a smart playlist.
#[utoipa::path(post, path = "/smart-playlists", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = CreateSmart,
    responses((status = 201, body = SmartPlaylist), (status = 400, description = "`invalid_rule`, `invalid_body`", body = crate::v2::problem::ProblemBody)))]
pub async fn create_smart(family: FamilyContext, State(state): State<AppState>, Json(body): Json<CreateSmart>) -> Result<(StatusCode, Json<SmartPlaylist>), Problem> {
    let name = name_of(&body.name)?;
    let rule = checked(&body.rule)?;
    let family_id = family_for(body.visibility.as_deref(), &family)?;
    let description = body.description.as_deref().map(str::trim).filter(|d| !d.is_empty());
    let row = db::music::create_smart_playlist(state.db(), family.user_id, family_id, &name, description, &rule).await?;
    Ok((StatusCode::CREATED, Json(smart(row))))
}

fn nullable<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(d).map(Some)
}

#[derive(Deserialize, ToSchema)]
pub struct PatchSmart {
    pub name: Option<String>,
    /// `null` clears it; leaving it out keeps it.
    #[serde(default, deserialize_with = "nullable")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[schema(value_type = Option<Object>)]
    pub rule: Option<Rule>,
    pub visibility: Option<String>,
}

/// Rename, describe, change the rule, share or stop sharing.
#[utoipa::path(patch, path = "/smart-playlists/{id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = PatchSmart,
    responses((status = 200, body = SmartPlaylist),
        (status = 400, description = "`invalid_rule`, `invalid_body`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_smart(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<PatchSmart>,
) -> Result<Json<SmartPlaylist>, Problem> {
    let current = owned(&state, &family, id).await?;
    let name = match body.name.as_deref() {
        Some(n) => name_of(n)?,
        None => current.name,
    };
    let description = match body.description {
        Some(d) => d.map(|d| d.trim().to_string()).filter(|d| !d.is_empty()),
        None => current.description,
    };
    let rule = match body.rule.as_ref() {
        Some(r) => checked(r)?,
        None => current.rule,
    };
    let family_id = match body.visibility.as_deref() {
        Some(v) => family_for(Some(v), &family)?,
        None => current.family_id,
    };
    let row = catalog::update_smart_playlist(state.db(), id, family.user_id, &name, description.as_deref(), &rule, family_id)
        .await?
        .ok_or_else(Problem::not_found)?;
    Ok(Json(smart(row)))
}

/// Delete a smart playlist. Its rule is all it held, so this is final.
#[utoipa::path(delete, path = "/smart-playlists/{id}", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Deleted"),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_smart(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    owned(&state, &family, id).await?;
    db::music::delete_smart_playlist(state.db(), id, family.user_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn yes() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
pub struct Freeze {
    /// The new playlist's name; the smart playlist's by default.
    pub name: Option<String>,
    /// `private` (the default) or `family`.
    pub visibility: Option<String>,
    /// Mark the smart playlist frozen (the default). False keeps it living and
    /// makes this a copy. Only the owner's own is ever marked.
    #[serde(default = "yes")]
    pub mark_frozen: bool,
}

/// Make what the rule holds now into an ordinary playlist: fixed, editable,
/// the same for everyone it is shared with.
#[utoipa::path(post, path = "/smart-playlists/{id}/freeze", tag = "playlists", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)), request_body = Freeze,
    responses((status = 201, description = "The new playlist", body = Playlist),
        (status = 400, description = "`empty_result`: the rule matches nothing now", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn freeze_smart(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<Freeze>,
) -> Result<(StatusCode, Json<Playlist>), Problem> {
    let row = find(&state, &family, id).await?;
    let rule: Rule = serde_json::from_value(row.rule.clone()).map_err(|e| Problem::internal(anyhow::anyhow!("stored rule is not valid: {e}")))?;
    let resolved = resolve(&state, &family, &rule, None).await?;
    if resolved.items.is_empty() {
        return Err(Problem::bad_request("empty_result", "the rule matches nothing right now, so there is nothing to freeze"));
    }
    let name = match body.name.as_deref() {
        Some(n) => name_of(n)?,
        None => row.name.clone(),
    };
    let family_id = family_for(body.visibility.as_deref(), &family)?;
    let made = db::music::insert_playlist(state.db(), family.user_id, family_id, &name, row.description.as_deref(), true).await?;
    let ids: Vec<Uuid> = resolved.items.iter().map(|t| t.id).collect();
    db::music::add_tracks_to_playlist(state.db(), made.id, &ids, family.viewer()).await?;
    // A member who froze a shared rule gets their copy; the owner's stays as it was.
    if body.mark_frozen && row.user_id == family.user_id {
        db::music::mark_smart_playlist_frozen(state.db(), id, family.user_id, made.id).await?;
    }
    let playlist = crate::v2::playlists::find_for(&state, &family, made.id).await?;
    Ok((StatusCode::CREATED, Json(playlist)))
}
