// SPDX-License-Identifier: AGPL-3.0-or-later
//! Corrections on the catalogue (DESIGN §7.2: "a wrong match is fixed once, on
//! the entity, never by retagging files"; docs/api-v2-plan.md, V3c).
//!
//! - An item's credits, by hand: a role set here stays as set whatever the tags
//!   say. That is how two people of one name are told apart, or a credit the
//!   tags lack is added. The item's owner sets a track's, book's or podcast's;
//!   any member who sees an album sets its credits, as it is the household's.
//! - Contributors: rename, sort name and kind (any member who sees them);
//!   merge and split (a family admin's, as they rewrite everyone's credits).
//! - Albums: title and release details, release group (any member who sees
//!   them); merge (a family admin's).

use crate::app::AppState;
use crate::db;
use crate::db::catalog;
use crate::db::entities::{AlbumDetails, CreditInput, Renamed};
use crate::families::{FamilyAdmin, FamilyContext};
use crate::v2::music::{Contributor, Credit};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// Absent, `null` and a value are three different things in a PATCH.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

const TRACK_ROLES: &[&str] = &["artist", "featured", "composer", "lyricist", "producer", "conductor", "performer", "remixer"];
const BOOK_ROLES: &[&str] = &["author", "co_author", "narrator", "translator", "editor", "illustrator"];
const PODCAST_ROLES: &[&str] = &["host", "co_host", "producer"];
const CONTRIBUTOR_KINDS: &[&str] = &["unknown", "person", "group", "orchestra", "choir", "organisation"];

#[derive(Deserialize, ToSchema)]
pub struct CreditEntry {
    /// An existing contributor of the household…
    pub contributor_id: Option<Uuid>,
    /// …or a name: the household's contributor of that name, made if new.
    pub name: Option<String>,
    pub role: String,
    /// The name as printed on this item, when it differs.
    pub credited_as: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreditsRequest {
    /// The corrected roles, complete and in order. A role corrected before and
    /// left out goes back to what the tags say; `[]` gives every role back.
    pub credits: Vec<CreditEntry>,
}

async fn resolve_credits(state: &AppState, family: &FamilyContext, roles: &[&str], body: CreditsRequest) -> Result<Vec<CreditInput>, Problem> {
    let mut out = Vec::new();
    for (i, entry) in body.credits.into_iter().enumerate() {
        if !roles.contains(&entry.role.as_str()) {
            return Err(Problem::bad_request("invalid_role", format!("`{}` is not a role here; one of: {}", entry.role, roles.join(", ")))
                .field(format!("/credits/{i}/role"), "invalid", entry.role.clone()));
        }
        let contributor_id = match (entry.contributor_id, entry.name.as_deref().map(str::trim).filter(|n| !n.is_empty())) {
            (Some(id), _) => id,
            (None, Some(name)) => db::entities::contributor_named(state.db(), family.family_id, name).await?,
            (None, None) => {
                return Err(Problem::bad_request("invalid_body", "each credit needs a contributor_id or a name")
                    .field(format!("/credits/{i}"), "required", "contributor_id or name"));
            }
        };
        out.push(CreditInput { contributor_id, role: entry.role, credited_as: entry.credited_as });
    }
    let ids: Vec<Uuid> = out.iter().map(|c| c.contributor_id).collect();
    if !db::entities::all_in_household(state.db(), family.family_id, &ids).await? {
        return Err(Problem::bad_request("unknown_contributor", "a contributor_id is not one of this household's"));
    }
    Ok(out)
}

async fn answer_credits(state: &AppState, kind: &str, id: Uuid) -> Result<Json<Vec<Credit>>, Problem> {
    let mut rows = catalog::credits(state.db(), kind, &[id]).await?;
    Ok(Json(
        rows.drain(..)
            .map(|r| Credit { contributor_id: r.contributor_id, role: r.role, position: r.position, credited_as: r.credited_as, join_phrase: r.join_phrase })
            .collect(),
    ))
}

/// The caller owns it (`403`), or cannot see it (`404`).
fn owner(owner_id: Uuid, family: &FamilyContext) -> Result<(), Problem> {
    if owner_id == family.user_id { Ok(()) } else { Err(Problem::forbidden().detail("only its owner corrects this item")) }
}

/// Correct a track's credits.
#[utoipa::path(put, path = "/tracks/{id}/credits", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = CreditsRequest,
    responses((status = 200, description = "The track's credits now", body = Vec<Credit>),
        (status = 400, description = "`invalid_role`, `unknown_contributor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the track's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn set_track_credits(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<CreditsRequest>) -> Result<Json<Vec<Credit>>, Problem> {
    let track = catalog::find_track(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    owner(track.user_id, &family)?;
    let credits = resolve_credits(&state, &family, TRACK_ROLES, body).await?;
    db::entities::set_credits(state.db(), "track", id, &credits).await?;
    answer_credits(&state, "track", id).await
}

/// Correct a book's credits.
#[utoipa::path(put, path = "/books/{id}/credits", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = CreditsRequest,
    responses((status = 200, description = "The book's credits now", body = Vec<Credit>),
        (status = 400, description = "`invalid_role`, `unknown_contributor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the book's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn set_book_credits(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<CreditsRequest>) -> Result<Json<Vec<Credit>>, Problem> {
    let book = catalog::find_book(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    owner(book.user_id, &family)?;
    let credits = resolve_credits(&state, &family, BOOK_ROLES, body).await?;
    db::entities::set_credits(state.db(), "book", id, &credits).await?;
    answer_credits(&state, "book", id).await
}

/// Correct a podcast's credits.
#[utoipa::path(put, path = "/podcasts/{id}/credits", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = CreditsRequest,
    responses((status = 200, description = "The podcast's credits now", body = Vec<Credit>),
        (status = 400, description = "`invalid_role`, `unknown_contributor`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the podcast's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn set_podcast_credits(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<CreditsRequest>) -> Result<Json<Vec<Credit>>, Problem> {
    let podcast = catalog::find_podcast(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    owner(podcast.user_id, &family)?;
    let credits = resolve_credits(&state, &family, PODCAST_ROLES, body).await?;
    db::entities::set_credits(state.db(), "podcast", id, &credits).await?;
    answer_credits(&state, "podcast", id).await
}

/// Correct an album's credits; the album artist line follows.
#[utoipa::path(put, path = "/albums/{id}/credits", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)), request_body = CreditsRequest,
    responses((status = 200, description = "The album's credits now", body = Vec<Credit>),
        (status = 400, description = "`invalid_role`, `unknown_contributor`, or no credits", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn set_album_credits(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<CreditsRequest>) -> Result<Json<Vec<Credit>>, Problem> {
    catalog::find_album(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    if body.credits.is_empty() {
        return Err(Problem::bad_request("invalid_body", "an album keeps at least one credit"));
    }
    let credits = resolve_credits(&state, &family, TRACK_ROLES, body).await?;
    db::entities::set_credits(state.db(), "album", id, &credits).await?;
    answer_credits(&state, "album", id).await
}

// ── Contributors ─────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ContributorPatch {
    /// Shown everywhere they are credited; the old spelling keeps matching tags.
    pub name: Option<String>,
    /// `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub sort_name: Option<Option<String>>,
    pub kind: Option<String>,
}

async fn visible_contributor(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<Contributor, Problem> {
    let row = catalog::find_contributor(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    Ok(Contributor {
        id: row.id,
        name: row.name,
        sort_name: row.sort_name,
        kind: row.kind,
        musicbrainz_artist_id: row.musicbrainz_artist_id,
        has_image: row.has_image,
        roles: None,
    })
}

/// Correct a contributor: name, sort name, kind.
#[utoipa::path(patch, path = "/contributors/{id}", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)), request_body = ContributorPatch,
    responses((status = 200, body = Contributor),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`name_taken`: another contributor has the name; merge them instead", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_contributor(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<ContributorPatch>) -> Result<Json<Contributor>, Problem> {
    visible_contributor(&state, &family, id).await?;
    if let Some(kind) = body.kind.as_deref() {
        if !CONTRIBUTOR_KINDS.contains(&kind) {
            return Err(Problem::bad_request("invalid_body", format!("kind is one of: {}", CONTRIBUTOR_KINDS.join(", "))).field("/kind", "invalid", kind.to_string()));
        }
    }
    if let Some(name) = body.name.as_deref() {
        if name.trim().is_empty() {
            return Err(Problem::bad_request("invalid_body", "a name cannot be empty").field("/name", "required", "a name"));
        }
        let mut tx = state.db().begin().await.map_err(anyhow::Error::from)?;
        match db::entities::rename_contributor(&mut tx, family.family_id, id, name).await? {
            Renamed::Done => {}
            Renamed::NotFound => return Err(Problem::not_found()),
            Renamed::Taken(other) => {
                return Err(Problem::new(StatusCode::CONFLICT, "name_taken").detail(crate::audiobooks::authors::NAME_TAKEN).with("contributor_id", other.to_string()));
            }
        }
        tx.commit().await.map_err(anyhow::Error::from)?;
    }
    if body.sort_name.is_some() || body.kind.is_some() {
        let sort = body.sort_name.as_ref().map(|s| s.as_deref());
        db::entities::describe_contributor(state.db(), family.family_id, id, sort, body.kind.as_deref()).await?;
    }
    Ok(Json(visible_contributor(&state, &family, id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct MergeRequest {
    /// The one that stays.
    pub into: Uuid,
}

/// Two contributors are one: everything `id` has goes to `into`, and it goes.
#[utoipa::path(post, path = "/contributors/{id}/merge", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("id" = Uuid, Path, description = "The one that goes")), request_body = MergeRequest,
    responses((status = 200, description = "The one that stays", body = Contributor),
        (status = 404, description = "Either is not the household's", body = crate::v2::problem::ProblemBody)))]
pub async fn merge_contributor(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<MergeRequest>) -> Result<Json<Contributor>, Problem> {
    if body.into == id {
        return Err(Problem::bad_request("invalid_body", "a contributor cannot be merged into itself"));
    }
    if !db::entities::merge_contributors(state.db(), family.family_id, id, body.into).await? {
        return Err(Problem::not_found());
    }
    Ok(Json(visible_contributor(&state, &family, body.into).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct SplitRequest {
    /// Who it really is, in credit order; at least two.
    pub names: Vec<String>,
}

/// One contributor is several ("A & B" read as one name): every credit, now
/// and at every later scan, becomes one per name.
#[utoipa::path(post, path = "/contributors/{id}/split", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("id" = Uuid, Path, description = "The one that goes")), request_body = SplitRequest,
    responses((status = 200, description = "The contributors it became, in order", body = Vec<Contributor>),
        (status = 404, description = "Not the household's", body = crate::v2::problem::ProblemBody)))]
pub async fn split_contributor(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<SplitRequest>) -> Result<Json<Vec<Contributor>>, Problem> {
    let mut names: Vec<String> = Vec::new();
    for name in body.names.iter().map(|n| n.trim()).filter(|n| !n.is_empty()) {
        if !names.iter().any(|seen| seen.to_lowercase() == name.to_lowercase()) {
            names.push(name.to_string());
        }
    }
    if names.len() < 2 {
        return Err(Problem::bad_request("invalid_body", "a split needs at least two different names").field("/names", "too_few", "two or more"));
    }
    let parts = db::entities::split_contributor(state.db(), family.family_id, id, &names).await?.ok_or_else(Problem::not_found)?;
    let ids: Vec<Uuid> = parts.iter().map(|(id, _)| *id).collect();
    let rows = catalog::contributors_by_ids(state.db(), family.family_id, &ids).await?;
    Ok(Json(
        ids.iter()
            .filter_map(|id| rows.iter().find(|r| r.id == *id))
            .map(|r| Contributor {
                id: r.id,
                name: r.name.clone(),
                sort_name: r.sort_name.clone(),
                kind: r.kind.clone(),
                musicbrainz_artist_id: r.musicbrainz_artist_id.clone(),
                has_image: r.has_image,
                roles: None,
            })
            .collect(),
    ))
}

// ── Albums ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct AlbumPatch {
    pub title: Option<String>,
    /// `null` clears any of these.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub release_date: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub country: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub label: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub barcode: Option<Option<String>>,
}

#[derive(Serialize, ToSchema)]
pub struct Updated {
    pub id: Uuid,
}

/// Correct an album: title, release date, country, label, barcode. Its tracks
/// stay on it whatever their tags say.
#[utoipa::path(patch, path = "/albums/{id}", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)), request_body = AlbumPatch,
    responses((status = 200, description = "Corrected; read it back with `GET /albums/{id}`", body = Updated),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_album(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<AlbumPatch>) -> Result<Json<Updated>, Problem> {
    catalog::find_album(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    if body.title.as_deref().is_some_and(|t| t.trim().is_empty()) {
        return Err(Problem::bad_request("invalid_body", "a title cannot be empty").field("/title", "required", "a title"));
    }
    let details = AlbumDetails { title: body.title, release_date: body.release_date, country: body.country, label: body.label, barcode: body.barcode };
    if !db::entities::update_album(state.db(), family.family_id, id, &details).await? {
        return Err(Problem::not_found());
    }
    Ok(Json(Updated { id }))
}

#[derive(Deserialize, ToSchema)]
pub struct ReleaseGroupRequest {
    /// Another album's `release_group_id`, so the two are editions of one
    /// release; `null` for a group of its own.
    pub release_group_id: Option<Uuid>,
}

/// Put an album in a release group. Tags no longer move it.
#[utoipa::path(put, path = "/albums/{id}/release-group", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)), request_body = ReleaseGroupRequest,
    responses((status = 200, description = "The album's release group now", body = Updated),
        (status = 404, description = "The album or group is not visible or not the household's", body = crate::v2::problem::ProblemBody)))]
pub async fn set_release_group(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<ReleaseGroupRequest>) -> Result<Json<Updated>, Problem> {
    catalog::find_album(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let group = db::entities::set_album_release_group(state.db(), family.family_id, id, body.release_group_id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(Updated { id: group }))
}

/// Two albums are one release tagged two ways: `id`'s tracks, stars and
/// ratings go to `into`, and so will the same tags at every later scan.
#[utoipa::path(post, path = "/albums/{id}/merge", tag = "corrections", security(("bearer" = [])),
    extensions(("x-permission" = json!("family_admin"))),
    params(("id" = Uuid, Path, description = "The one that goes")), request_body = MergeRequest,
    responses((status = 200, description = "The album that stays", body = Updated),
        (status = 404, description = "Either is not the household's", body = crate::v2::problem::ProblemBody)))]
pub async fn merge_album(FamilyAdmin(family): FamilyAdmin, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<MergeRequest>) -> Result<Json<Updated>, Problem> {
    if body.into == id {
        return Err(Problem::bad_request("invalid_body", "an album cannot be merged into itself"));
    }
    if !db::entities::merge_albums(state.db(), family.family_id, id, body.into).await? {
        return Err(Problem::not_found());
    }
    Ok(Json(Updated { id: body.into }))
}
