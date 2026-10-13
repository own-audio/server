// SPDX-License-Identifier: AGPL-3.0-or-later
//! The metadata organizer (docs/metadata-organizer-plan.md, O1): a member's own
//! songs as a table — every editable field, the file, whether it is identified —
//! with filters and facets, and saving many songs' changes as one batch of the
//! edit history. Nothing here touches a file.

use crate::app::AppState;
use crate::db;
use crate::db::organize::{Filter, OrganizeRow, TrackChange};
use crate::families::FamilyContext;
use crate::v2::page::PageQuery;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Deserializer, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The most songs one save may change.
const MAX_SAVE: usize = 500;

#[derive(Serialize, ToSchema)]
pub struct OrganizeTrack {
    pub id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub disc_total: Option<i32>,
    pub is_compilation: bool,
    pub duration_secs: Option<i32>,
    pub album_id: Option<Uuid>,
    /// The album's year.
    pub year: Option<String>,
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    /// `identify` (matched by this server), `mbid_tag` (the file carries the
    /// ids), or absent.
    pub identity: Option<String>,
    pub has_cover: bool,
    /// The library folder the file is in; absent for an upload.
    pub folder_id: Option<Uuid>,
    /// The file's path inside its folder, or an upload's file name.
    pub path: String,
    pub content_type: String,
    pub size_bytes: Option<i64>,
    /// It has changes in the history.
    pub edited: bool,
    pub updated_at: DateTime<Utc>,
}

fn track_out(r: OrganizeRow) -> OrganizeTrack {
    OrganizeTrack {
        id: r.id,
        title: r.title,
        artist: r.artist,
        album: r.album,
        album_artist: r.album_artist,
        genre: r.genre,
        track_number: r.track_number,
        disc_number: r.disc_number,
        disc_total: r.disc_total,
        is_compilation: r.is_compilation,
        duration_secs: r.duration_secs,
        album_id: r.album_id,
        year: r.year,
        musicbrainz_recording_id: r.musicbrainz_recording_id,
        musicbrainz_release_id: r.musicbrainz_release_id,
        identity: r.identity,
        has_cover: r.has_cover,
        folder_id: r.folder_id,
        path: r.path,
        content_type: r.content_type,
        size_bytes: r.size_bytes,
        edited: r.edited,
        updated_at: r.updated_at,
    }
}

#[derive(Deserialize, IntoParams, Default)]
#[into_params(parameter_in = Query)]
pub struct OrganizeFilter {
    /// Words in the title, artist, album or file path.
    pub q: Option<String>,
    /// A library folder's id.
    pub folder: Option<Uuid>,
    /// A path prefix inside `folder`, e.g. `Artist/Album/`.
    pub path: Option<String>,
    /// An artist as `facets` names it.
    pub artist: Option<String>,
    pub album_id: Option<Uuid>,
    #[serde(default)]
    pub unidentified: bool,
    #[serde(default)]
    pub no_cover: bool,
    #[serde(default)]
    pub no_year: bool,
    #[serde(default)]
    pub no_genre: bool,
    /// Songs with changes in the history.
    #[serde(default)]
    pub edited: bool,
    /// Songs whose title and artist another of yours has too.
    #[serde(default)]
    pub duplicates: bool,
    /// `title`, `artist`, `album`, `album_artist`, `genre`, `track`, `year`,
    /// `path`, `duration`, `updated`; by album and track by default.
    pub sort: Option<String>,
    #[serde(default)]
    pub desc: bool,
}

#[derive(Serialize, ToSchema)]
pub struct OrganizePage {
    pub items: Vec<OrganizeTrack>,
    pub next: Option<String>,
    /// Songs the filter matches, all pages.
    pub total: i64,
}

/// Your songs as the organizer shows them, filtered and sorted. Up to 500 a
/// page.
#[utoipa::path(get, path = "/organize/tracks", tag = "organize", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    params(OrganizeFilter, PageQuery),
    responses((status = 200, body = OrganizePage), (status = 400, body = crate::v2::problem::ProblemBody)))]
pub async fn list_tracks(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<OrganizeFilter>,
    Query(page): Query<PageQuery>,
) -> Result<Json<OrganizePage>, Problem> {
    let order = match q.sort.as_deref() {
        None => db::organize::DEFAULT_SORT,
        Some(s) => db::organize::sort_sql(s).ok_or_else(|| {
            Problem::bad_request("invalid_sort", "unknown sort column").field("/sort", "invalid", "a column name")
        })?,
    };
    let limit = page.limit.unwrap_or(200).clamp(1, 500);
    let offset: i64 = page.after()?.unwrap_or(0);
    let f = Filter {
        q: q.q,
        folder: q.folder,
        path: q.path,
        artist: q.artist,
        album_id: q.album_id,
        unidentified: q.unidentified,
        no_cover: q.no_cover,
        no_year: q.no_year,
        no_genre: q.no_genre,
        edited: q.edited,
        duplicates: q.duplicates,
    };
    let (rows, total) = db::organize::list(state.db(), family.user_id, &f, order, q.desc, offset, limit).await?;
    let more = offset + (rows.len() as i64) < total;
    let next = more.then(|| crate::v2::page::encode(&(offset + rows.len() as i64)));
    Ok(Json(OrganizePage { items: rows.into_iter().map(track_out).collect(), next, total }))
}

#[derive(Serialize, ToSchema)]
pub struct FolderFacet {
    /// Absent: uploaded songs.
    pub id: Option<Uuid>,
    pub path: Option<String>,
    pub tracks: i64,
}

#[derive(Serialize, ToSchema)]
pub struct ArtistFacet {
    /// The album artist as the library groups by it; pass as `artist`.
    pub name: String,
    pub tracks: i64,
}

#[derive(Serialize, ToSchema)]
pub struct AlbumFacet {
    pub id: Uuid,
    pub title: String,
    pub artist: String,
    pub tracks: i64,
}

#[derive(Serialize, ToSchema)]
pub struct FilterCounts {
    pub all: i64,
    pub unidentified: i64,
    pub no_cover: i64,
    pub no_year: i64,
    pub no_genre: i64,
    pub edited: i64,
}

#[derive(Serialize, ToSchema)]
pub struct Facets {
    pub folders: Vec<FolderFacet>,
    pub artists: Vec<ArtistFacet>,
    pub albums: Vec<AlbumFacet>,
    pub counts: FilterCounts,
}

/// What your songs can be narrowed to, with counts: folders, artists, albums
/// and the quick filters.
#[utoipa::path(get, path = "/organize/facets", tag = "organize", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    responses((status = 200, body = Facets)))]
pub async fn facets(family: FamilyContext, State(state): State<AppState>) -> Result<Json<Facets>, Problem> {
    let (folders, artists, albums, c) = db::organize::facets(state.db(), family.user_id).await?;
    Ok(Json(Facets {
        folders: folders.into_iter().map(|f| FolderFacet { id: f.id, path: f.path, tracks: f.tracks }).collect(),
        artists: artists.into_iter().map(|a| ArtistFacet { name: a.key, tracks: a.tracks }).collect(),
        albums: albums.into_iter().map(|a| AlbumFacet { id: a.id, title: a.title, artist: a.artist, tracks: a.tracks }).collect(),
        counts: FilterCounts {
            all: c.all,
            unidentified: c.unidentified,
            no_cover: c.no_cover,
            no_year: c.no_year,
            no_genre: c.no_genre,
            edited: c.edited,
        },
    }))
}

/// Absent, `null` and a value are three different things.
fn present<'de, D: Deserializer<'de>, T: Deserialize<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Deserialize, ToSchema)]
pub struct TrackPatch {
    pub id: Uuid,
    /// Never empty.
    pub title: Option<String>,
    /// A field left out stays; `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub artist: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub album: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub album_artist: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub genre: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>)]
    pub track_number: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>)]
    pub disc_number: Option<Option<i32>>,
    pub is_compilation: Option<bool>,
}

#[derive(Deserialize, ToSchema)]
pub struct TracksPatch {
    /// Each song with its own values — the same values for a bulk edit.
    pub tracks: Vec<TrackPatch>,
}

#[derive(Serialize, ToSchema)]
pub struct Saved {
    /// Songs that changed.
    pub updated: u64,
    /// The save's batch in the history: `POST /edits/batches/{id}/undo`.
    pub batch_id: Uuid,
}

/// Save changes to many of your songs as one batch of the history (`bulk`, or
/// `manual` for one song). Songs that are not yours are left out.
#[utoipa::path(patch, path = "/tracks", tag = "organize", security(("bearer" = [])),
    extensions(("x-permission" = json!("user"))),
    request_body = TracksPatch,
    responses((status = 200, body = Saved), (status = 400, body = crate::v2::problem::ProblemBody)))]
pub async fn save_tracks(family: FamilyContext, State(state): State<AppState>, Json(body): Json<TracksPatch>) -> Result<Json<Saved>, Problem> {
    if body.tracks.is_empty() || body.tracks.len() > MAX_SAVE {
        return Err(Problem::bad_request("invalid_body", format!("1 to {MAX_SAVE} songs a save")).field("/tracks", "invalid", "1–500 songs"));
    }
    if let Some(i) = body.tracks.iter().position(|t| t.title.as_deref().is_some_and(|s| s.trim().is_empty())) {
        return Err(Problem::bad_request("invalid_body", "a title cannot be empty").field(&format!("/tracks/{i}/title"), "required", "a title"));
    }
    let changes: Vec<TrackChange> = body
        .tracks
        .into_iter()
        .map(|t| TrackChange {
            id: t.id,
            title: t.title,
            artist: t.artist,
            album: t.album,
            album_artist: t.album_artist,
            genre: t.genre,
            track_number: t.track_number,
            disc_number: t.disc_number,
            is_compilation: t.is_compilation,
        })
        .collect();
    let (batch_id, updated) = db::organize::save(state.db(), family.user_id, &changes).await?;
    Ok(Json(Saved { updated, batch_id }))
}
