// SPDX-License-Identifier: AGPL-3.0-or-later
//! The music catalogue (DESIGN §7.2–7.3; docs/api-v2-plan.md, V3): contributors
//! (and the `/artists`, `/authors`, `/narrators`, `/hosts` views of them),
//! albums, release groups, recordings, genres, tracks.
//!
//! Records are normalised: a track carries its album's id and its credits'
//! contributor ids, never their names, so a rename is one changed record.
//! `include=` hands the referenced records back once per answer. Durations are
//! milliseconds. Everything answers only what the caller can see; an id it
//! cannot see is `404`, as an id that does not exist.

use crate::app::AppState;
use crate::db::access::{Viewer, visibility_of};
use crate::db::catalog::{self, AlbumAfter, TrackAfter, TrackOrder};
use crate::families::FamilyContext;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

// ── Records ──────────────────────────────────────────────────────────────────

/// Anyone credited with something: an artist, an author, a narrator, a host. The
/// same person is one contributor whatever they do.
#[derive(Serialize, ToSchema)]
pub struct Contributor {
    pub id: Uuid,
    pub name: String,
    pub sort_name: Option<String>,
    /// `person`, `group`, `orchestra`, `choir`, `organisation` or `unknown`.
    pub kind: String,
    pub musicbrainz_artist_id: Option<String>,
    pub has_image: bool,
    /// What they are credited as on what the caller can see; only on `GET /contributors/{id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<String>>,
}

/// Who did what on an item, in order (MusicBrainz's artist credit).
#[derive(Serialize, ToSchema, Clone)]
pub struct Credit {
    pub contributor_id: Uuid,
    /// `artist`, `featured`, `author`, `narrator`, `host`…
    pub role: String,
    pub position: i16,
    /// The name as printed on this item, when it differs from the contributor's.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub credited_as: Option<String>,
    /// What follows this name in the full credit line: `"; "`, `" feat. "`, or nothing.
    pub join_phrase: String,
}

/// A release: one edition of an album.
#[derive(Serialize, ToSchema)]
pub struct Album {
    pub id: Uuid,
    pub title: String,
    pub credits: Vec<Credit>,
    /// The album as a work: every edition of it shares this.
    pub release_group_id: Option<Uuid>,
    pub release_date: Option<String>,
    pub country: Option<String>,
    pub label: Option<String>,
    pub barcode: Option<String>,
    pub is_compilation: bool,
    pub musicbrainz_release_id: Option<String>,
    /// Of the tracks the caller can see.
    pub track_count: i64,
    pub duration_ms: i64,
    pub disc_count: i32,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct ReleaseGroup {
    pub id: Uuid,
    pub title: String,
    /// `album`, `single`, `ep`, `compilation`, `live`, `soundtrack`…
    pub kind: String,
    pub first_release_date: Option<String>,
    pub musicbrainz_release_group_id: Option<String>,
    /// Its editions the caller can see; only on `GET /release-groups/{id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album_ids: Option<Vec<Uuid>>,
}

/// The performance itself: several tracks on several albums can be one recording.
#[derive(Serialize, ToSchema)]
pub struct Recording {
    pub id: Uuid,
    pub title: String,
    pub musicbrainz_recording_id: String,
    pub isrc: Option<String>,
    pub track_ids: Vec<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct Genre {
    pub id: Uuid,
    pub name: String,
}

/// One appearance of a recording on an album: the file.
#[derive(Serialize, ToSchema)]
pub struct Track {
    pub id: Uuid,
    pub title: String,
    pub album_id: Option<Uuid>,
    pub recording_id: Option<Uuid>,
    pub credits: Vec<Credit>,
    pub genre_ids: Vec<Uuid>,
    pub disc_number: Option<i32>,
    pub track_number: Option<i32>,
    pub duration_ms: Option<i64>,
    /// `private` (its owner's) or `family` (shared with the household).
    pub visibility: String,
    pub owner_id: Uuid,
    /// `upload`, or `folder` for a read-only library folder.
    pub source: String,
    pub content_type: Option<String>,
    pub size_bytes: Option<i64>,
    pub has_cover: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// The records a list's items refer to, each once (`include=`).
#[derive(Serialize, ToSchema, Default)]
pub struct Included {
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub contributors: Vec<Contributor>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub albums: Vec<Album>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub release_groups: Vec<ReleaseGroup>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub genres: Vec<Genre>,
}

pub(crate) fn contributor(row: catalog::ContributorRow) -> Contributor {
    Contributor {
        id: row.id,
        name: row.name,
        sort_name: row.sort_name,
        kind: row.kind,
        musicbrainz_artist_id: row.musicbrainz_artist_id,
        has_image: row.has_image,
        roles: None,
    }
}

fn credit(row: &catalog::CreditRow) -> Credit {
    Credit {
        contributor_id: row.contributor_id,
        role: row.role.clone(),
        position: row.position,
        credited_as: row.credited_as.clone(),
        join_phrase: row.join_phrase.clone(),
    }
}

pub(crate) fn release_group(row: catalog::ReleaseGroupRow) -> ReleaseGroup {
    ReleaseGroup {
        id: row.id,
        title: row.title,
        kind: row.kind,
        first_release_date: row.first_release_date,
        musicbrainz_release_group_id: row.musicbrainz_release_group_id,
        album_ids: None,
    }
}

async fn credits_by_item(state: &AppState, kind: &str, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<Credit>>, Problem> {
    let mut by_item: HashMap<Uuid, Vec<Credit>> = HashMap::new();
    for row in catalog::credits(state.db(), kind, ids).await? {
        by_item.entry(row.item_id).or_default().push(credit(&row));
    }
    Ok(by_item)
}

pub(crate) async fn albums(state: &AppState, viewer: Viewer, rows: Vec<catalog::AlbumRow>) -> Result<Vec<Album>, Problem> {
    let ids: Vec<Uuid> = rows.iter().map(|a| a.id).collect();
    let mut credits = credits_by_item(state, "album", &ids).await?;
    let stats: HashMap<Uuid, (i64, i64, i32)> =
        catalog::album_stats(state.db(), viewer, &ids).await?.into_iter().map(|(id, n, secs, discs)| (id, (n, secs, discs))).collect();
    Ok(rows
        .into_iter()
        .map(|a| {
            let (track_count, secs, disc_count) = stats.get(&a.id).copied().unwrap_or((0, 0, 1));
            Album {
                credits: credits.remove(&a.id).unwrap_or_default(),
                id: a.id,
                title: a.title,
                release_group_id: a.release_group_id,
                release_date: a.release_date,
                country: a.country,
                label: a.label,
                barcode: a.barcode,
                is_compilation: a.is_compilation,
                musicbrainz_release_id: a.musicbrainz_release_id,
                track_count,
                duration_ms: secs * 1000,
                disc_count,
                created_at: a.created_at,
            }
        })
        .collect())
}

pub(crate) async fn tracks(state: &AppState, rows: Vec<catalog::TrackRow>) -> Result<Vec<Track>, Problem> {
    let ids: Vec<Uuid> = rows.iter().map(|t| t.id).collect();
    let mut credits = credits_by_item(state, "track", &ids).await?;
    let mut genres: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for (track, genre) in catalog::track_genres(state.db(), &ids).await? {
        genres.entry(track).or_default().push(genre);
    }
    Ok(rows
        .into_iter()
        .map(|t| Track {
            credits: credits.remove(&t.id).unwrap_or_default(),
            genre_ids: genres.remove(&t.id).unwrap_or_default(),
            id: t.id,
            title: t.title,
            album_id: t.album_id,
            recording_id: t.recording_id,
            disc_number: t.disc_number,
            track_number: t.track_number,
            duration_ms: t.duration_secs.map(|s| i64::from(s) * 1000),
            visibility: visibility_of(t.family_id).to_string(),
            owner_id: t.user_id,
            source: t.source,
            content_type: t.content_type,
            size_bytes: t.size_bytes,
            has_cover: t.has_cover,
            created_at: t.created_at,
            updated_at: t.updated_at,
        })
        .collect())
}

// ── include= ─────────────────────────────────────────────────────────────────

#[derive(Default)]
struct Wanted {
    contributors: bool,
    albums: bool,
    release_groups: bool,
    genres: bool,
}

fn wanted(include: Option<&str>, allowed: &[&str]) -> Result<Wanted, Problem> {
    let mut w = Wanted::default();
    for part in include.unwrap_or("").split(',').map(str::trim).filter(|p| !p.is_empty()) {
        if !allowed.contains(&part) {
            return Err(Problem::bad_request("invalid_include", format!("`{part}` cannot be included here; one of: {}", allowed.join(", ")))
                .field("include", "invalid", part.to_string()));
        }
        match part {
            "contributors" => w.contributors = true,
            "albums" => w.albums = true,
            "release_groups" => w.release_groups = true,
            _ => w.genres = true,
        }
    }
    Ok(w)
}

async fn included(
    state: &AppState,
    viewer: Viewer,
    w: &Wanted,
    credits: impl Iterator<Item = Uuid>,
    album_ids: impl Iterator<Item = Uuid>,
    group_ids: impl Iterator<Item = Uuid>,
    genre_ids: impl Iterator<Item = Uuid>,
) -> Result<Option<Included>, Problem> {
    if !(w.contributors || w.albums || w.release_groups || w.genres) {
        return Ok(None);
    }
    let pool = state.db();
    let mut out = Included::default();
    let mut contributor_ids: BTreeSet<Uuid> = if w.contributors { credits.collect() } else { BTreeSet::new() };
    if w.albums {
        let ids: Vec<Uuid> = album_ids.collect::<BTreeSet<_>>().into_iter().collect();
        out.albums = albums(state, viewer, catalog::albums_by_ids(pool, viewer.family_id, &ids).await?).await?;
        if w.contributors {
            contributor_ids.extend(out.albums.iter().flat_map(|a| a.credits.iter().map(|c| c.contributor_id)));
        }
    }
    if w.contributors {
        let ids: Vec<Uuid> = contributor_ids.into_iter().collect();
        out.contributors = catalog::contributors_by_ids(pool, viewer.family_id, &ids).await?.into_iter().map(contributor).collect();
    }
    if w.release_groups {
        let mut ids: BTreeSet<Uuid> = group_ids.collect();
        ids.extend(out.albums.iter().filter_map(|a| a.release_group_id));
        let ids: Vec<Uuid> = ids.into_iter().collect();
        out.release_groups = catalog::release_groups_by_ids(pool, viewer.family_id, &ids).await?.into_iter().map(release_group).collect();
    }
    if w.genres {
        let ids: Vec<Uuid> = genre_ids.collect::<BTreeSet<_>>().into_iter().collect();
        out.genres = catalog::genres_by_ids(pool, viewer.family_id, &ids).await?.into_iter().map(|g| Genre { id: g.id, name: g.name }).collect();
    }
    Ok(Some(out))
}

// ── Contributors ─────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ContributorsQuery {
    /// Only those credited in these roles, comma-separated: `artist,featured`.
    pub role: Option<String>,
    /// Part of the name, any case.
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

async fn contributor_page(
    state: &AppState,
    family: &FamilyContext,
    roles: Option<Vec<String>>,
    q: Option<&str>,
    page: PageQuery,
) -> Result<Json<Page<Contributor>>, Problem> {
    let limit = page.limit()?;
    let rows = catalog::list_contributors(state.db(), family.viewer(), roles.as_deref(), q, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |r| (r.sort_key.clone(), r.id));
    Ok(Json(Page { items: page.items.into_iter().map(contributor).collect(), next: page.next, included: None }))
}

/// Contributors credited on what the caller can see, by name.
#[utoipa::path(get, path = "/contributors", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(ContributorsQuery),
    responses((status = 200, body = Page<Contributor>)))]
pub async fn list_contributors(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<ContributorsQuery>,
) -> Result<Json<Page<Contributor>>, Problem> {
    let roles = q.role.as_deref().map(|r| r.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect());
    contributor_page(&state, &family, roles, q.q.as_deref(), PageQuery { limit: q.limit, cursor: q.cursor }).await
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ViewQuery {
    /// Part of the name, any case.
    pub q: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

macro_rules! view {
    ($name:ident, $path:literal, $doc:literal, [$($role:literal),+]) => {
        #[doc = $doc]
        #[utoipa::path(get, path = $path, tag = "catalog", security(("bearer" = [])),
            extensions(("x-permission" = json!("member"))),
            params(ViewQuery),
            responses((status = 200, body = Page<Contributor>)))]
        pub async fn $name(
            family: FamilyContext,
            State(state): State<AppState>,
            Query(q): Query<ViewQuery>,
        ) -> Result<Json<Page<Contributor>>, Problem> {
            let roles = vec![$($role.to_string()),+];
            contributor_page(&state, &family, Some(roles), q.q.as_deref(), PageQuery { limit: q.limit, cursor: q.cursor }).await
        }
    };
}

view!(list_artists, "/artists", "Contributors credited as an artist of a track or album the caller can see.", ["artist"]);
view!(list_authors, "/authors", "Contributors credited as an author of a book the caller can see.", ["author", "co_author"]);
view!(list_narrators, "/narrators", "Contributors who narrate a book the caller can see.", ["narrator"]);
view!(list_hosts, "/hosts", "Contributors who host a podcast the caller can see.", ["host", "co_host"]);

/// One contributor, with the roles they have on what the caller can see.
#[utoipa::path(get, path = "/contributors/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Contributor), (status = 404, description = "Unknown, or nothing of theirs is visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_contributor(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Contributor>, Problem> {
    let row = catalog::find_contributor(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let mut answer = contributor(row);
    answer.roles = Some(catalog::contributor_roles(state.db(), family.viewer(), id).await?);
    Ok(Json(answer))
}

// ── Albums ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct AlbumsQuery {
    /// Credited to this contributor, on the album or on one of its tracks.
    pub contributor_id: Option<Uuid>,
    pub release_group_id: Option<Uuid>,
    pub genre_id: Option<Uuid>,
    /// Part of the title, any case.
    pub q: Option<String>,
    /// `title` (default) or `added`, newest first.
    pub sort: Option<String>,
    /// Comma-separated: `contributors`, `release_groups`.
    pub include: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Albums with a track the caller can see.
#[utoipa::path(get, path = "/albums", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(AlbumsQuery),
    responses((status = 200, body = Page<Album>)))]
pub async fn list_albums(family: FamilyContext, State(state): State<AppState>, Query(q): Query<AlbumsQuery>) -> Result<Json<Page<Album>>, Problem> {
    let w = wanted(q.include.as_deref(), &["contributors", "release_groups"])?;
    let by_added = match q.sort.as_deref() {
        None | Some("title") => false,
        Some("added") => true,
        Some(other) => return Err(Problem::bad_request("invalid_sort", format!("`{other}`: sort by `title` or `added`")).field("sort", "invalid", other.to_string())),
    };
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let after = if by_added {
        page.after::<(DateTime<Utc>, Uuid)>()?.map(|(at, id)| AlbumAfter::Added(at, id))
    } else {
        page.after::<(String, Uuid)>()?.map(|(t, id)| AlbumAfter::Title(t, id))
    };
    let filter = catalog::AlbumFilter { contributor_id: q.contributor_id, release_group_id: q.release_group_id, genre_id: q.genre_id, query: q.q };
    let viewer = family.viewer();
    let rows = catalog::list_albums(state.db(), viewer, &filter, by_added, after, limit + 1).await?;
    let page = if by_added {
        let p = Page::from_rows(rows, limit, |a| (a.created_at, a.id));
        (p.items, p.next)
    } else {
        let p = Page::from_rows(rows, limit, |a| (a.sort_key.clone(), a.id));
        (p.items, p.next)
    };
    let items = albums(&state, viewer, page.0).await?;
    let credit_ids: Vec<Uuid> = items.iter().flat_map(|a| a.credits.iter().map(|c| c.contributor_id)).collect();
    let group_ids: Vec<Uuid> = items.iter().filter_map(|a| a.release_group_id).collect();
    let included = included(&state, viewer, &w, credit_ids.into_iter(), std::iter::empty(), group_ids.into_iter(), std::iter::empty()).await?;
    Ok(Json(Page { items, next: page.1, included }))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct IncludeQuery {
    /// Comma-separated records to send along.
    pub include: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct AlbumAnswer {
    #[serde(flatten)]
    pub album: Album,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Included>,
}

/// One album.
#[utoipa::path(get, path = "/albums/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path), IncludeQuery),
    responses((status = 200, body = AlbumAnswer), (status = 404, description = "Unknown, or no track of it is visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_album(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<IncludeQuery>,
) -> Result<Json<AlbumAnswer>, Problem> {
    let w = wanted(q.include.as_deref(), &["contributors", "release_groups"])?;
    let viewer = family.viewer();
    let row = catalog::find_album(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    let album = albums(&state, viewer, vec![row]).await?.pop().ok_or_else(Problem::not_found)?;
    let credit_ids: Vec<Uuid> = album.credits.iter().map(|c| c.contributor_id).collect();
    let included = included(&state, viewer, &w, credit_ids.into_iter(), std::iter::empty(), album.release_group_id.into_iter(), std::iter::empty()).await?;
    Ok(Json(AlbumAnswer { album, included }))
}

/// One release group, with its editions the caller can see.
#[utoipa::path(get, path = "/release-groups/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = ReleaseGroup), (status = 404, description = "Unknown, or no edition is visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_release_group(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<ReleaseGroup>, Problem> {
    let (row, album_ids) = catalog::find_release_group(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let mut group = release_group(row);
    group.album_ids = Some(album_ids);
    Ok(Json(group))
}

/// One recording, with its tracks the caller can see.
#[utoipa::path(get, path = "/recordings/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Recording), (status = 404, description = "Unknown, or no track of it is visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_recording(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Recording>, Problem> {
    let (row, track_ids) = catalog::find_recording(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(Recording { id: row.id, title: row.title, musicbrainz_recording_id: row.musicbrainz_recording_id, isrc: row.isrc, track_ids }))
}

/// Genres of tracks the caller can see, by name.
#[utoipa::path(get, path = "/genres", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<Genre>)))]
pub async fn list_genres(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<Genre>>, Problem> {
    let limit = page.limit()?;
    let rows = catalog::list_genres(state.db(), family.viewer(), page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |g| (g.sort_key.clone(), g.id));
    Ok(Json(Page { items: page.items.into_iter().map(|g| Genre { id: g.id, name: g.name }).collect(), next: page.next, included: None }))
}

// ── Tracks ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct TracksQuery {
    pub album_id: Option<Uuid>,
    /// Credited on the track itself.
    pub contributor_id: Option<Uuid>,
    pub genre_id: Option<Uuid>,
    pub recording_id: Option<Uuid>,
    /// Part of the title, any case.
    pub q: Option<String>,
    /// `album` (disc and number; the default with `album_id`), `title` (otherwise the default) or `added`, newest first.
    pub sort: Option<String>,
    /// Comma-separated: `albums`, `contributors`, `genres`.
    pub include: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

/// Tracks the caller can see.
#[utoipa::path(get, path = "/tracks", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(TracksQuery),
    responses((status = 200, body = Page<Track>)))]
pub async fn list_tracks(family: FamilyContext, State(state): State<AppState>, Query(q): Query<TracksQuery>) -> Result<Json<Page<Track>>, Problem> {
    let w = wanted(q.include.as_deref(), &["albums", "contributors", "genres"])?;
    let order = match (q.sort.as_deref(), q.album_id) {
        (Some("album"), _) | (None, Some(_)) => TrackOrder::InAlbum,
        (Some("title"), _) | (None, None) => TrackOrder::Title,
        (Some("added"), _) => TrackOrder::Added,
        (Some(other), _) => {
            return Err(Problem::bad_request("invalid_sort", format!("`{other}`: sort by `album`, `title` or `added`")).field("sort", "invalid", other.to_string()));
        }
    };
    let page = PageQuery { limit: q.limit, cursor: q.cursor };
    let limit = page.limit()?;
    let after = match order {
        TrackOrder::Title => page.after::<(String, Uuid)>()?.map(|(t, id)| TrackAfter::Title(t, id)),
        TrackOrder::Added => page.after::<(DateTime<Utc>, Uuid)>()?.map(|(at, id)| TrackAfter::Added(at, id)),
        TrackOrder::InAlbum => page.after::<(i32, i32, String, Uuid)>()?.map(|(d, n, t, id)| TrackAfter::InAlbum(d, n, t, id)),
    };
    let filter = catalog::TrackFilter {
        album_id: q.album_id,
        contributor_id: q.contributor_id,
        genre_id: q.genre_id,
        recording_id: q.recording_id,
        query: q.q,
    };
    let viewer = family.viewer();
    let rows = catalog::list_tracks(state.db(), viewer, &filter, order, after, limit + 1).await?;
    let (rows, next) = match order {
        TrackOrder::Title => {
            let p = Page::from_rows(rows, limit, |t| (t.sort_key.clone(), t.id));
            (p.items, p.next)
        }
        TrackOrder::Added => {
            let p = Page::from_rows(rows, limit, |t| (t.created_at, t.id));
            (p.items, p.next)
        }
        TrackOrder::InAlbum => {
            let p = Page::from_rows(rows, limit, |t| (t.disc_number.unwrap_or(1), t.track_number.unwrap_or(0), t.sort_key.clone(), t.id));
            (p.items, p.next)
        }
    };
    let items = tracks(&state, rows).await?;
    let credit_ids: Vec<Uuid> = items.iter().flat_map(|t| t.credits.iter().map(|c| c.contributor_id)).collect();
    let album_ids: Vec<Uuid> = items.iter().filter_map(|t| t.album_id).collect();
    let genre_ids: Vec<Uuid> = items.iter().flat_map(|t| t.genre_ids.iter().copied()).collect();
    let included = included(&state, viewer, &w, credit_ids.into_iter(), album_ids.into_iter(), std::iter::empty(), genre_ids.into_iter()).await?;
    Ok(Json(Page { items, next, included }))
}

#[derive(Serialize, ToSchema)]
pub struct TrackAnswer {
    #[serde(flatten)]
    pub track: Track,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Included>,
}

/// One track.
#[utoipa::path(get, path = "/tracks/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path), IncludeQuery),
    responses((status = 200, body = TrackAnswer), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_track(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<IncludeQuery>,
) -> Result<Json<TrackAnswer>, Problem> {
    let w = wanted(q.include.as_deref(), &["albums", "contributors", "genres"])?;
    let viewer = family.viewer();
    let row = catalog::find_track(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    let track = tracks(&state, vec![row]).await?.pop().ok_or_else(Problem::not_found)?;
    let credit_ids: Vec<Uuid> = track.credits.iter().map(|c| c.contributor_id).collect();
    let genre_ids = track.genre_ids.clone();
    let included = included(&state, viewer, &w, credit_ids.into_iter(), track.album_id.into_iter(), std::iter::empty(), genre_ids.into_iter()).await?;
    Ok(Json(TrackAnswer { track, included }))
}
