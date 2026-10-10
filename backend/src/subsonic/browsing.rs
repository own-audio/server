// SPDX-License-Identifier: AGPL-3.0-or-later
/// ID3 browsing endpoints: ping/license, getArtists, getIndexes, getArtist,
/// getAlbumList2, getAlbum, getSong, search3, getMusicFolders.
use crate::app::AppState;
use crate::db;
use crate::db::subsonic::{AlbumRow, ArtistRow};
use crate::music::models::MusicTrack;
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::extract::SubsonicQuery;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
use crate::subsonic::ids::{self, UNKNOWN_ALBUM, UNKNOWN_ARTIST};
use crate::subsonic::resolve;
use axum::extract::State;
use axum::response::Response;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use uuid::Uuid;

fn ok(auth: &SubsonicAuthUser, body: Value) -> Response {
    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), body)
}

fn err(auth: &SubsonicAuthUser, code: SubsonicErrorCode) -> Response {
    envelope::error(auth.format, auth.jsonp_callback.as_deref(), code, None)
}

fn normalize_or<'a>(value: Option<&'a str>, default: &'a str) -> &'a str {
    value.map(str::trim).filter(|s| !s.is_empty()).unwrap_or(default)
}

// ── System ────────────────────────────────────────────────────────────────

pub async fn ping(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({}))
}

pub async fn get_license(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({ "license": { "valid": true } }))
}

/// Declares only what is actually implemented. An empty list was honest while
/// nothing was, but a client reads this to decide which of two spellings of a
/// feature to use — advertising an extension we do not serve is worse than
/// advertising none, because the client stops falling back.
pub async fn get_open_subsonic_extensions(auth: SubsonicAuthUser) -> Response {
    ok(
        &auth,
        json!({
            "openSubsonicExtensions": [
                // `getLyricsBySongId`, timed lines included.
                { "name": "songLyrics", "versions": [1] },
                // Every route is registered for POST as well as GET, and
                // `form::merge_post_form` folds a form body into the query —
                // so a client can send a long play queue in a body instead of
                // a URL a proxy might truncate.
                { "name": "formPost", "versions": [1] }
            ]
        }),
    )
}

/// audio2 has exactly one library, so it advertises exactly one folder. The
/// id is a protocol fiction with nothing behind it, but it has to be stable:
/// clients cache it and send it back.
pub(crate) const MUSIC_FOLDER_ID: i64 = 1;

pub async fn get_music_folders(auth: SubsonicAuthUser) -> Response {
    ok(
        &auth,
        json!({ "musicFolders": { "musicFolder": [{ "id": MUSIC_FOLDER_ID, "name": "Music" }] } }),
    )
}

/// Whether a request's `musicFolderId` addresses the folder this server has.
///
/// An absent id means "all folders", which is the same set. A *different* id
/// addresses a folder that does not exist, and the honest answer is an empty
/// result — the parameter used to be read and discarded, so a client asking
/// for the contents of a folder we never advertised got the whole library.
pub(crate) fn folder_selected(id: Option<i64>) -> bool {
    matches!(id, None | Some(MUSIC_FOLDER_ID))
}

// ── Artists ───────────────────────────────────────────────────────────────

fn artist_json(row: &ArtistRow, user_id: Uuid, groups: &GroupContext) -> Value {
    let mut obj = json!({
        "id": ids::artist_id(user_id, &row.artist).to_string(),
        "name": row.artist,
        "albumCount": row.album_count,
    });
    groups.annotate(&mut obj, &row.artist, "");
    obj
}

/// Leading words a client should ignore when alphabetising, matching the
/// default every Subsonic-derived server ships.
const IGNORED_ARTICLES: &str = "The El La Los Las Le Les";

fn group_by_index(
    rows: &[ArtistRow],
    user_id: Uuid,
    groups: &GroupContext,
    last_modified: i64,
) -> Value {
    let mut by_letter: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    for row in rows {
        let letter = row
            .artist
            .chars()
            .next()
            .map(|c| {
                if c.is_alphabetic() {
                    c.to_uppercase().to_string()
                } else {
                    "#".to_string()
                }
            })
            .unwrap_or_else(|| "#".to_string());
        by_letter.entry(letter).or_default().push(artist_json(row, user_id, groups));
    }
    let index: Vec<Value> = by_letter
        .into_iter()
        .map(|(name, artist)| json!({ "name": name, "artist": artist }))
        .collect();
    // `ignoredArticles` and `lastModified` are both required attributes on
    // `<indexes>`. The articles are what a client strips when sorting, so an
    // empty string files "The Beatles" under T; `lastModified` is what it
    // compares to decide whether its cached index is stale.
    json!({
        "ignoredArticles": IGNORED_ARTICLES,
        "lastModified": last_modified,
        "index": index,
    })
}

#[derive(Deserialize)]
pub struct FolderParam {
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<i64>,
}

impl FolderParam {
    pub(crate) fn music_folder_id(&self) -> Option<i64> {
        self.music_folder_id
    }
}

pub async fn get_artists(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<FolderParam>,
) -> Response {
    if !folder_selected(params.music_folder_id) {
        return ok(&auth, json!({ "artists": { "ignoredArticles": IGNORED_ARTICLES, "index": [] } }));
    }
    let rows = match db::subsonic::list_distinct_artists(state.db(), auth.viewer()).await {
        Ok(rows) => rows,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let last_modified = library_last_modified(&state, &auth).await;
    let body = group_by_index(&rows, auth.user_id, &groups, last_modified);
    ok(&auth, json!({ "artists": body }))
}

/// When this user's visible library last changed, as the epoch milliseconds
/// `<indexes lastModified>` is specified in.
///
/// Derived from the newest track rather than tracked separately: a deletion
/// leaves it unchanged, which costs a client one stale index at worst, and
/// nothing here is expensive enough to justify a counter that can drift.
async fn library_last_modified(state: &AppState, auth: &SubsonicAuthUser) -> i64 {
    db::subsonic::library_last_modified(state.db(), auth.viewer())
        .await
        .ok()
        .flatten()
        .map(|at| at.timestamp_millis())
        .unwrap_or(0)
}

pub async fn get_indexes(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<FolderParam>,
) -> Response {
    if !folder_selected(params.music_folder_id) {
        return ok(
            &auth,
            json!({ "indexes": { "ignoredArticles": IGNORED_ARTICLES, "lastModified": 0, "index": [] } }),
        );
    }
    let rows = match db::subsonic::list_distinct_artists(state.db(), auth.viewer()).await {
        Ok(rows) => rows,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let last_modified = library_last_modified(&state, &auth).await;
    let body = group_by_index(&rows, auth.user_id, &groups, last_modified);
    ok(&auth, json!({ "indexes": body }))
}

#[derive(Deserialize)]
pub struct IdParam {
    id: String,
}

pub async fn get_artist(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    let Ok(target) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let name = match resolve::resolve(state.db(), auth.viewer(), target).await {
        Ok(Some(resolve::Named::Artist(name))) => name,
        Ok(_) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let albums = match db::subsonic::list_artist_albums(state.db(), auth.viewer(), &name).await {
        Ok(rows) => rows,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let album_json: Vec<Value> = albums.iter().map(|a| album_summary_json(a, auth.user_id, &groups)).collect();

    let mut artist = json!({
        "id": target.to_string(),
        "name": name,
        "albumCount": albums.len(),
        "album": album_json,
    });
    groups.annotate(&mut artist, &name, "");

    ok(&auth, json!({ "artist": artist }))
}

// ── Albums ────────────────────────────────────────────────────────────────

fn album_summary_json(row: &AlbumRow, user_id: Uuid, groups: &GroupContext) -> Value {
    let mut obj = json!({
        "id": ids::album_id(user_id, &row.artist, &row.album).to_string(),
        "name": row.album,
        "artist": row.artist,
        "artistId": ids::artist_id(user_id, &row.artist).to_string(),
        "songCount": row.song_count,
        "duration": row.duration_secs.unwrap_or(0),
        "created": row.created_at.to_rfc3339(),
        "coverArt": row.cover_object_id.map(|_| ids::album_id(user_id, &row.artist, &row.album).to_string()),
    });
    groups.annotate(&mut obj, &row.artist, &row.album);
    obj
}

#[derive(Deserialize)]
pub struct AlbumListParams {
    #[serde(rename = "type")]
    list_type: Option<String>,
    size: Option<i64>,
    offset: Option<i64>,
    genre: Option<String>,
    #[serde(rename = "fromYear")]
    from_year: Option<i32>,
    #[serde(rename = "toYear")]
    to_year: Option<i32>,
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<i64>,
}

/// The `type` values `getAlbumList`/`getAlbumList2` accept.
///
/// Checked rather than defaulted: an unrecognized type used to fall through to
/// newest-first, so a client asking for something this server could not do got
/// a plausible-looking wrong answer instead of a refusal it could act on.
const ALBUM_LIST_TYPES: &[&str] = &[
    "random",
    "newest",
    "highest",
    "frequent",
    "recent",
    "alphabeticalByName",
    "alphabeticalByArtist",
    "starred",
    "byYear",
    "byGenre",
];

/// Resolve one page of albums for either spelling of the album-list endpoint.
///
/// Both `getAlbumList` and `getAlbumList2` answer the same question and differ
/// only in how they render the answer, so the selection lives here once. Seven
/// of the ten types were previously ignored — `byGenre`, `byYear`, `starred`,
/// `frequent`, `recent` and `highest` all silently returned the newest albums,
/// which is what made a client's home screen show the same shelf under every
/// heading.
pub(crate) async fn select_albums(
    state: &AppState,
    auth: &SubsonicAuthUser,
    params: &AlbumListParams,
    groups: &GroupContext,
) -> Result<Vec<AlbumRow>, SubsonicErrorCode> {
    let list_type = params.list_type.as_deref().unwrap_or("newest");
    if !ALBUM_LIST_TYPES.contains(&list_type) {
        return Err(SubsonicErrorCode::MissingParam);
    }
    if !folder_selected(params.music_folder_id) {
        return Ok(Vec::new());
    }

    let mut rows = db::subsonic::list_distinct_albums(state.db(), auth.viewer())
        .await
        .map_err(|_| SubsonicErrorCode::Generic)?;

    match list_type {
        // The query already returns newest-first.
        "newest" => {}
        "alphabeticalByName" => rows.sort_by(|a, b| a.album.cmp(&b.album)),
        "alphabeticalByArtist" => {
            rows.sort_by(|a, b| (&a.artist, &a.album).cmp(&(&b.artist, &b.album)))
        }
        "random" => {
            use rand::seq::SliceRandom;
            rows.shuffle(&mut rand::rng());
        }
        "starred" => {
            rows.retain(|r| groups.starred_at(&r.artist, &r.album).is_some());
            rows.sort_by_key(|r| std::cmp::Reverse(groups.starred_at(&r.artist, &r.album)));
        }
        "highest" => {
            rows.retain(|r| groups.rating(&r.artist, &r.album).is_some());
            rows.sort_by_key(|r| std::cmp::Reverse(groups.rating(&r.artist, &r.album)));
        }
        "frequent" | "recent" => {
            let stats = db::subsonic::album_play_stats(state.db(), auth.viewer())
                .await
                .map_err(|_| SubsonicErrorCode::Generic)?;
            let by_key: HashMap<(String, String), db::subsonic::AlbumPlayStats> = stats
                .into_iter()
                .map(|s| ((s.artist.clone(), s.album.clone()), s))
                .collect();
            let key = |r: &AlbumRow| (r.artist.clone(), r.album.clone());
            if list_type == "frequent" {
                // An album never played is absent from the shelf entirely
                // rather than sitting at the bottom of it with a zero.
                rows.retain(|r| by_key.get(&key(r)).is_some_and(|s| s.play_count > 0));
                rows.sort_by_key(|r| {
                    std::cmp::Reverse(by_key.get(&key(r)).map(|s| s.play_count).unwrap_or(0))
                });
            } else {
                rows.retain(|r| by_key.get(&key(r)).is_some_and(|s| s.last_played.is_some()));
                rows.sort_by_key(|r| {
                    std::cmp::Reverse(by_key.get(&key(r)).and_then(|s| s.last_played))
                });
            }
        }
        "byGenre" => {
            let Some(genre) = params.genre.as_deref().filter(|g| !g.is_empty()) else {
                return Err(SubsonicErrorCode::MissingParam);
            };
            let keys: std::collections::HashSet<(String, String)> =
                db::subsonic::album_keys_for_genre(state.db(), auth.viewer(), genre)
                    .await
                    .map_err(|_| SubsonicErrorCode::Generic)?
                    .into_iter()
                    .collect();
            rows.retain(|r| keys.contains(&(r.artist.clone(), r.album.clone())));
        }
        "byYear" => {
            // `fromYear`/`toYear` are required by the protocol, so a client
            // that omits them is told so rather than handed an empty list it
            // would read as "no albums in that decade".
            if params.from_year.is_none() || params.to_year.is_none() {
                return Err(SubsonicErrorCode::MissingParam);
            }
            // audio2 stores no release year for music — `music_tracks` has no
            // such column and the ingest path never captured one. An empty
            // list is the truthful answer; returning newest-first (what this
            // did before) claimed albums belonged to a decade at random.
            rows.clear();
        }
        _ => unreachable!("guarded by ALBUM_LIST_TYPES above"),
    }

    let size = params.size.unwrap_or(10).clamp(1, 500) as usize;
    let offset = params.offset.unwrap_or(0).max(0) as usize;
    Ok(rows.into_iter().skip(offset).take(size).collect())
}

pub async fn get_album_list2(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<AlbumListParams>,
) -> Response {
    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let rows = match select_albums(&state, &auth, &params, &groups).await {
        Ok(rows) => rows,
        Err(code) => return err(&auth, code),
    };
    let page: Vec<Value> = rows
        .iter()
        .map(|row| album_summary_json(row, auth.user_id, &groups))
        .collect();

    ok(&auth, json!({ "albumList2": { "album": page } }))
}

pub async fn get_album(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    let Ok(target) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let Some(matched) = (match find_album_by_id(&auth, &state, target).await {
        Ok(found) => found,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    }) else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let tracks = match db::subsonic::list_album_tracks(state.db(), auth.viewer(), &matched.artist, &matched.album).await {
        Ok(tracks) => tracks,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let songs: Vec<Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();

    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let mut album = album_summary_json(&matched, auth.user_id, &groups);
    album["song"] = json!(songs);

    ok(&auth, json!({ "album": album }))
}

async fn find_album_by_id(
    auth: &SubsonicAuthUser,
    state: &AppState,
    id: Uuid,
) -> anyhow::Result<Option<db::subsonic::AlbumRow>> {
    match resolve::resolve(state.db(), auth.viewer(), id).await? {
        Some(resolve::Named::Album { artist, album }) => db::subsonic::find_album(state.db(), auth.viewer(), &artist, &album).await,
        _ => Ok(None),
    }
}

pub async fn get_song(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    let Ok(track_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };
    let track = match db::music::find_track(state.db(), track_id, auth.viewer()).await {
        Ok(Some(track)) => track,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let ctx = SongContext::load_one(state.db(), auth.user_id, &track).await;
    ok(&auth, json!({ "song": song_json(&track, &ctx) }))
}

// ── Search ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct Search3Params {
    query: Option<String>,
    #[serde(rename = "songCount")]
    song_count: Option<i64>,
    #[serde(rename = "artistCount")]
    artist_count: Option<i64>,
    #[serde(rename = "albumCount")]
    album_count: Option<i64>,
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<i64>,
}

pub async fn search3(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<Search3Params>,
) -> Response {
    if !folder_selected(params.music_folder_id) {
        return ok(&auth, json!({ "searchResult3": {} }));
    }
    let query = params.query.unwrap_or_default();
    let needle = query.trim().to_lowercase();

    let song_limit = params.song_count.unwrap_or(20).clamp(0, 500);
    let songs = if song_limit > 0 {
        match db::subsonic::search_tracks(state.db(), auth.viewer(), &query, song_limit).await {
            Ok(tracks) => tracks,
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        }
    } else {
        Vec::new()
    };

    let ctx = SongContext::load(state.db(), auth.user_id, &songs).await;
    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let song_json_list: Vec<Value> = songs.iter().map(|t| song_json(t, &ctx)).collect();

    let artist_limit = params.artist_count.unwrap_or(20).clamp(0, 500) as usize;
    let album_limit = params.album_count.unwrap_or(20).clamp(0, 500) as usize;

    let artists = if artist_limit > 0 && !needle.is_empty() {
        match db::subsonic::list_distinct_artists(state.db(), auth.viewer()).await {
            Ok(rows) => rows
                .iter()
                .filter(|row| row.artist.to_lowercase().contains(&needle))
                .take(artist_limit)
                .map(|row| artist_json(row, auth.user_id, &groups))
                .collect(),
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        }
    } else {
        Vec::new()
    };

    let albums = if album_limit > 0 && !needle.is_empty() {
        match db::subsonic::list_distinct_albums(state.db(), auth.viewer()).await {
            Ok(rows) => rows
                .iter()
                .filter(|row| row.album.to_lowercase().contains(&needle))
                .take(album_limit)
                .map(|row| album_summary_json(row, auth.user_id, &groups))
                .collect(),
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        }
    } else {
        Vec::new()
    };

    ok(
        &auth,
        json!({
            "searchResult3": {
                "artist": artists,
                "album": albums,
                "song": song_json_list,
            }
        }),
    )
}

// ── Shared song mapping ───────────────────────────────────────────────────

/// Everything a batch of songs needs that does not live on the track row:
/// media details, the viewer's stars, the viewer's ratings.
///
/// Loaded once per response and shared by every song in it. The alternative —
/// each song fetching its own — cost one query per track, so a full album list
/// issued hundreds of round trips to render one page.
pub(crate) struct SongContext {
    media: HashMap<Uuid, (String, Option<i64>)>,
    starred: HashMap<Uuid, DateTime<Utc>>,
    ratings: HashMap<Uuid, i16>,
}

impl SongContext {
    pub(crate) async fn load(pool: &sqlx::PgPool, user_id: Uuid, tracks: &[MusicTrack]) -> Self {
        let object_ids: Vec<Uuid> = tracks.iter().map(|t| t.audio_object_id).collect();
        SongContext {
            media: db::subsonic::media_info_for(pool, &object_ids)
                .await
                .unwrap_or_default(),
            // Annotations are decoration: a failure here should render an
            // unstarred, unrated library rather than fail the whole response.
            starred: db::music::starred_track_ids(pool, user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect(),
            ratings: db::music::track_ratings(pool, user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .collect(),
        }
    }

    /// For the endpoints that render exactly one track.
    pub(crate) async fn load_one(pool: &sqlx::PgPool, user_id: Uuid, track: &MusicTrack) -> Self {
        Self::load(pool, user_id, std::slice::from_ref(track)).await
    }
}

/// Stars and ratings for albums and artists, which are keyed by name rather
/// than by row — the group tables store the names the synthetic ids were
/// derived from. Loaded once per response, like [`SongContext`].
#[derive(Default)]
pub(crate) struct GroupContext {
    /// `(artist, album)`; album is `""` for an artist.
    starred: HashMap<(String, String), DateTime<Utc>>,
    ratings: HashMap<(String, String), i16>,
}

impl GroupContext {
    pub(crate) async fn load(pool: &sqlx::PgPool, user_id: Uuid) -> Self {
        GroupContext {
            starred: db::music::starred_groups(pool, user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(_, artist, album, at)| ((artist, album), at))
                .collect(),
            ratings: db::music::group_ratings(pool, user_id)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|(_, artist, album, rating)| ((artist, album), rating))
                .collect(),
        }
    }

    /// The moment this user starred an album (or artist, with `album` empty),
    /// if they did. Exposed so the album-list endpoints can implement
    /// `type=starred` and `type=highest` against the same data `annotate`
    /// writes, rather than a second, divergent notion of "starred".
    pub(crate) fn starred_at(&self, artist: &str, album: &str) -> Option<DateTime<Utc>> {
        self.starred.get(&(artist.to_string(), album.to_string())).copied()
    }

    pub(crate) fn rating(&self, artist: &str, album: &str) -> Option<i16> {
        self.ratings.get(&(artist.to_string(), album.to_string())).copied()
    }

    /// Writes `starred`/`userRating` onto an album or artist object, so a
    /// client that just rated something reads its own value back rather than
    /// finding the field absent and assuming the write was lost.
    fn annotate(&self, obj: &mut Value, artist: &str, album: &str) {
        let key = (artist.to_string(), album.to_string());
        if let Some(at) = self.starred.get(&key) {
            obj["starred"] = json!(at.to_rfc3339());
        }
        if let Some(rating) = self.ratings.get(&key) {
            obj["userRating"] = json!(rating);
        }
    }
}

pub(crate) fn song_json(track: &MusicTrack, ctx: &SongContext) -> Value {
    let artist_name = normalize_or(track.artist.as_deref(), UNKNOWN_ARTIST).to_string();
    let album_name = normalize_or(track.album.as_deref(), UNKNOWN_ALBUM).to_string();
    let artist_id = ids::artist_id(track.user_id, &artist_name);
    // Filed under the album artist, as the album lists file it: with the track artist, every
    // song on a sampler or a "feat." track would point at an album that doesn't exist.
    let album_artist = track.effective_album_artist().unwrap_or_else(|| UNKNOWN_ARTIST.to_string());
    let album_id = ids::album_id(track.user_id, &album_artist, &album_name);

    let mut obj = json!({
        "id": track.id.to_string(),
        "parent": album_id.to_string(),
        "isDir": false,
        "title": track.title,
        "album": album_name,
        "artist": artist_name,
        "track": track.track_number,
        "discNumber": track.disc_number,
        "duration": track.duration_secs,
        "created": track.created_at.to_rfc3339(),
        "albumId": album_id.to_string(),
        "artistId": artist_id.to_string(),
        "type": "music",
        "mediaType": "song",
    });

    if track.cover_object_id.is_some() {
        obj["coverArt"] = json!(track.id.to_string());
    }
    if let Some(genre) = &track.genre {
        obj["genre"] = json!(genre);
    }
    if let Some(mbid) = &track.musicbrainz_recording_id {
        obj["musicBrainzId"] = json!(mbid);
    }

    if let Some((content_type, size)) = ctx.media.get(&track.audio_object_id) {
        obj["suffix"] = json!(suffix_for(content_type));
        obj["contentType"] = json!(content_type);
        if let Some(bytes) = size {
            obj["size"] = json!(bytes);
            // Averaged over the file rather than read from the stream: audio2
            // stores no per-file bitrate, and clients use this only to size
            // downloads and pick a stream, where an average is honest enough.
            // Omitted entirely when the duration is unknown, since a bitrate
            // derived from a zero duration would be nonsense.
            if let Some(secs) = track.duration_secs.filter(|s| *s > 0) {
                obj["bitRate"] = json!((bytes * 8) / (secs as i64) / 1000);
            }
        }
    }

    if let Some(starred_at) = ctx.starred.get(&track.id) {
        obj["starred"] = json!(starred_at.to_rfc3339());
    }
    if let Some(rating) = ctx.ratings.get(&track.id) {
        obj["userRating"] = json!(rating);
    }

    obj
}

fn suffix_for(content_type: &str) -> &'static str {
    match content_type {
        "audio/flac" | "audio/x-flac" => "flac",
        "audio/ogg" | "audio/vorbis" => "ogg",
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/wav" | "audio/x-wav" => "wav",
        "audio/aac" => "aac",
        _ => "mp3",
    }
}

// ── Folder-era browsing ───────────────────────────────────────────────────

/// `getMusicDirectory` is the other half of `getIndexes`: a client that browses
/// by folder gets artists from one and their contents from the other. audio2
/// has no folders — `music_tracks` stores artist and album as strings — so the
/// two-level artist → album → song shape is synthesised from the same
/// aggregates the ID3 endpoints use.
///
/// Implemented because leaving it out while answering `getIndexes` is the
/// worst of both: the client is told folder browsing works, then finds every
/// directory empty.
pub async fn get_music_directory(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    let Ok(target) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let named = match resolve::resolve(state.db(), auth.viewer(), target).await {
        Ok(Some(named)) => named,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    // An artist directory: its children are that artist's albums.
    if let resolve::Named::Artist(name) = &named {
        let albums = match db::subsonic::list_artist_albums(state.db(), auth.viewer(), name).await {
            Ok(rows) => rows,
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        };
        let children: Vec<Value> = albums
            .iter()
            .map(|a| {
                let album_id = ids::album_id(auth.user_id, &a.artist, &a.album);
                json!({
                    "id": album_id.to_string(),
                    "parent": target.to_string(),
                    "isDir": true,
                    "title": a.album,
                    "album": a.album,
                    "artist": a.artist,
                    "songCount": a.song_count,
                    "duration": a.duration_secs.unwrap_or(0),
                    "created": a.created_at.to_rfc3339(),
                    "coverArt": a.cover_object_id.map(|_| album_id.to_string()),
                })
            })
            .collect();

        return ok(
            &auth,
            json!({
                "directory": {
                    "id": target.to_string(),
                    "name": name,
                    "child": children,
                }
            }),
        );
    }

    // An album directory: its children are the songs.
    let resolve::Named::Album { artist, album } = named else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let tracks = match db::subsonic::list_album_tracks(state.db(), auth.viewer(), &artist, &album).await
    {
        Ok(t) => t,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let children: Vec<Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();

    ok(
        &auth,
        json!({
            "directory": {
                "id": target.to_string(),
                "parent": ids::artist_id(auth.user_id, &artist).to_string(),
                "name": album,
                "child": children,
            }
        }),
    )
}

// ── Genres ────────────────────────────────────────────────────────────────

pub async fn get_genres(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    let rows = match db::subsonic::list_genres_with_counts(state.db(), auth.viewer()).await {
        Ok(rows) => rows,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let genres: Vec<Value> = rows
        .into_iter()
        .map(|(genre, song_count, album_count)| {
            json!({
                // The genre name is the element's text content in XML, which
                // the envelope writer renders from a "value" key.
                "value": genre,
                "songCount": song_count,
                "albumCount": album_count,
            })
        })
        .collect();
    ok(&auth, json!({ "genres": { "genre": genres } }))
}

// ── Artist and album info ─────────────────────────────────────────────────

/// Shared by `getArtistInfo` and `getArtistInfo2`, which differ only in their
/// wrapper name.
async fn artist_info_payload(auth: &SubsonicAuthUser, state: &AppState, id: &str) -> Option<Value> {
    let target = id.parse::<Uuid>().ok()?;
    let resolve::Named::Artist(artist) = resolve::resolve(state.db(), auth.viewer(), target).await.ok()?? else {
        return None;
    };

    let mut info = json!({});

    // The MBID comes from any identified track by this artist; there is no
    // artist table to hold one.
    if let Ok(Some(mbid)) = db::music::find_artist_mbid(state.db(), auth.user_id, &artist).await {
        info["musicBrainzId"] = json!(mbid);
    }

    // The licence credit itself is burned into the photograph
    // (`metadata::watermark`), which is what makes serving it to a client that
    // renders nothing but the image lawful. What goes here is the rest of what
    // CC BY-SA asks for and pixels cannot carry: a link back to the source, and
    // notice that the file was modified — which adding the credit made it.
    if let Ok(Some((_, author, license, _, source_url, is_user_set))) =
        db::music::find_artist_image_attribution(state.db(), auth.user_id, &artist).await
    {
        if !is_user_set {
            if let Some(credit) =
                crate::metadata::watermark::credit_line(author.as_deref(), license.as_deref())
            {
                let mut text = format!("Photo: {credit}. Modified: licence credit added.");
                if let Some(url) = source_url {
                    text.push(' ');
                    text.push_str(&url);
                }
                info["biography"] = json!(text);
            }
        }
    }

    Some(info)
}

pub async fn get_artist_info(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    match artist_info_payload(&auth, &state, &params.id).await {
        Some(info) => ok(&auth, json!({ "artistInfo": info })),
        None => err(&auth, SubsonicErrorCode::NotFound),
    }
}

pub async fn get_artist_info2(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    match artist_info_payload(&auth, &state, &params.id).await {
        Some(info) => ok(&auth, json!({ "artistInfo2": info })),
        None => err(&auth, SubsonicErrorCode::NotFound),
    }
}

/// Album notes come from a catalogue audio2 does not query — there is no
/// review or biography source wired up, and inventing one is worse than
/// admitting there is none. The MBID is real when the album's tracks were
/// identified; the rest of the object is legitimately empty.
async fn album_info_payload(auth: &SubsonicAuthUser, state: &AppState, id: &str) -> Option<Value> {
    let target = id.parse::<Uuid>().ok()?;
    let resolve::Named::Album { artist, album } = resolve::resolve(state.db(), auth.viewer(), target).await.ok()?? else {
        return None;
    };

    let tracks = db::subsonic::list_album_tracks(state.db(), auth.viewer(), &artist, &album)
        .await
        .ok()?;

    let mut info = json!({});
    if let Some(mbid) = tracks.iter().find_map(|t| t.musicbrainz_release_id.as_ref()) {
        info["musicBrainzId"] = json!(mbid);
    }
    Some(info)
}

pub async fn get_album_info(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    match album_info_payload(&auth, &state, &params.id).await {
        Some(info) => ok(&auth, json!({ "albumInfo": info })),
        None => err(&auth, SubsonicErrorCode::NotFound),
    }
}

pub async fn get_album_info2(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    match album_info_payload(&auth, &state, &params.id).await {
        Some(info) => ok(&auth, json!({ "albumInfo2": info })),
        None => err(&auth, SubsonicErrorCode::NotFound),
    }
}

/// "Similar" and "top" tracks need a recommendation catalogue audio2 has no
/// licence to query. Answered as empty rather than left to fail: a client
/// that asks and is told "none" hides the section, where one that gets an
/// error may report the server as broken.
pub async fn get_similar_songs(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({ "similarSongs": {} }))
}

pub async fn get_similar_songs2(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({ "similarSongs2": {} }))
}

pub async fn get_top_songs(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({ "topSongs": {} }))
}
