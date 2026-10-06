// SPDX-License-Identifier: AGPL-3.0-or-later
/// The list endpoints a client's home screen is built from: the pre-ID3
/// `getAlbumList`, shuffle, genre listings, favourites, and the older
/// `search2`.
///
/// These are mostly re-shapings of queries `browsing` already uses. They exist
/// separately because the protocol kept both a folder-era and an ID3-era
/// spelling of nearly everything, and clients pick between them by age rather
/// than by preference — a server that answers only the newer spelling looks
/// empty to an older client rather than looking old.
use crate::app::AppState;
use crate::db;
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::browsing::{self, AlbumListParams, GroupContext, SongContext, song_json};
use crate::subsonic::extract::SubsonicQuery;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
use crate::subsonic::ids;
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};

fn ok(auth: &SubsonicAuthUser, body: Value) -> Response {
    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), body)
}

fn err(auth: &SubsonicAuthUser, code: SubsonicErrorCode) -> Response {
    envelope::error(auth.format, auth.jsonp_callback.as_deref(), code, None)
}

/// The folder-era `getAlbumList`, whose entries are directories rather than
/// ID3 albums. Same data and the same selection as `getAlbumList2`, rendered
/// in the older shape — see [`browsing::select_albums`], which both share so
/// the two spellings cannot answer the same question differently.
pub async fn get_album_list(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<AlbumListParams>,
) -> Response {
    let groups = GroupContext::load(state.db(), auth.user_id).await;
    let rows = match browsing::select_albums(&state, &auth, &params, &groups).await {
        Ok(rows) => rows,
        Err(code) => return err(&auth, code),
    };

    let albums: Vec<Value> = rows
        .iter()
        .map(|row| {
            let album_id = ids::album_id(auth.user_id, &row.artist, &row.album);
            json!({
                "id": album_id.to_string(),
                "parent": ids::artist_id(auth.user_id, &row.artist).to_string(),
                "isDir": true,
                "title": row.album,
                "album": row.album,
                "artist": row.artist,
                "songCount": row.song_count,
                "duration": row.duration_secs.unwrap_or(0),
                "created": row.created_at.to_rfc3339(),
                "coverArt": row.cover_object_id.map(|_| album_id.to_string()),
            })
        })
        .collect();

    ok(&auth, json!({ "albumList": { "album": albums } }))
}

#[derive(Deserialize)]
pub struct RandomSongsParams {
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<i64>,
    size: Option<i64>,
    genre: Option<String>,
    #[serde(rename = "fromYear")]
    from_year: Option<i32>,
    #[serde(rename = "toYear")]
    to_year: Option<i32>,
}

pub async fn get_random_songs(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<RandomSongsParams>,
) -> Response {
    if !browsing::folder_selected(params.music_folder_id) {
        return ok(&auth, json!({ "randomSongs": {} }));
    }
    let size = params.size.unwrap_or(10).clamp(1, 500);
    let genre = params.genre.as_deref().filter(|g| !g.is_empty());

    let tracks = match db::music::list_random_tracks(
        state.db(),
        auth.viewer(),
        genre,
        params.from_year,
        params.to_year,
        size,
    )
    .await
    {
        Ok(t) => t,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let songs: Vec<Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();
    ok(&auth, json!({ "randomSongs": { "song": songs } }))
}

#[derive(Deserialize)]
pub struct SongsByGenreParams {
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<i64>,
    genre: Option<String>,
    count: Option<i64>,
    offset: Option<i64>,
}

pub async fn get_songs_by_genre(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<SongsByGenreParams>,
) -> Response {
    let Some(genre) = params.genre.as_deref().filter(|g| !g.is_empty()) else {
        return err(&auth, SubsonicErrorCode::MissingParam);
    };
    if !browsing::folder_selected(params.music_folder_id) {
        return ok(&auth, json!({ "songsByGenre": {} }));
    }
    let count = params.count.unwrap_or(10).clamp(1, 500);
    let offset = params.offset.unwrap_or(0).max(0);

    let tracks = match db::music::list_tracks_by_genre(state.db(), auth.viewer(), genre, count, offset).await
    {
        Ok(t) => t,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let songs: Vec<Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();
    ok(&auth, json!({ "songsByGenre": { "song": songs } }))
}

// ── Favourites ────────────────────────────────────────────────────────────

/// Shared by `getStarred` and `getStarred2`, which differ only in whether the
/// artist and album entries are directory-shaped or ID3-shaped.
async fn starred_payload(auth: &SubsonicAuthUser, state: &AppState, id3: bool) -> Result<Value, ()> {
    let starred_tracks = db::music::starred_track_ids(state.db(), auth.user_id)
        .await
        .map_err(|_| ())?;
    let groups = db::music::starred_groups(state.db(), auth.user_id)
        .await
        .map_err(|_| ())?;

    let mut tracks = Vec::new();
    for (track_id, _) in &starred_tracks {
        // Re-checked rather than trusted: a star can outlive the viewer's
        // access to the track it points at.
        if let Ok(Some(track)) = db::music::find_track(state.db(), *track_id, auth.viewer()).await {
            tracks.push(track);
        }
    }
    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let songs: Vec<Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();

    let albums = db::subsonic::list_distinct_albums(state.db(), auth.viewer())
        .await
        .map_err(|_| ())?;
    let artists = db::subsonic::list_distinct_artists(state.db(), auth.viewer())
        .await
        .map_err(|_| ())?;

    let mut album_json = Vec::new();
    let mut artist_json = Vec::new();
    for (kind, artist_name, album_name, created) in &groups {
        match kind.as_str() {
            "album" => {
                if let Some(row) = albums
                    .iter()
                    .find(|a| &a.artist == artist_name && &a.album == album_name)
                {
                    let id = ids::album_id(auth.user_id, &row.artist, &row.album);
                    album_json.push(json!({
                        "id": id.to_string(),
                        "name": row.album,
                        "title": row.album,
                        "album": row.album,
                        "artist": row.artist,
                        "artistId": ids::artist_id(auth.user_id, &row.artist).to_string(),
                        "songCount": row.song_count,
                        "duration": row.duration_secs.unwrap_or(0),
                        "created": row.created_at.to_rfc3339(),
                        "starred": created.to_rfc3339(),
                        "isDir": !id3,
                        "coverArt": row.cover_object_id.map(|_| id.to_string()),
                    }));
                }
            }
            "artist" => {
                if let Some(row) = artists.iter().find(|a| &a.artist == artist_name) {
                    artist_json.push(json!({
                        "id": ids::artist_id(auth.user_id, &row.artist).to_string(),
                        "name": row.artist,
                        "albumCount": row.album_count,
                        "starred": created.to_rfc3339(),
                    }));
                }
            }
            _ => {}
        }
    }

    Ok(json!({ "artist": artist_json, "album": album_json, "song": songs }))
}

pub async fn get_starred(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<browsing::FolderParam>,
) -> Response {
    if !browsing::folder_selected(params.music_folder_id()) {
        return ok(&auth, json!({ "starred": {} }));
    }
    match starred_payload(&auth, &state, false).await {
        Ok(body) => ok(&auth, json!({ "starred": body })),
        Err(()) => err(&auth, SubsonicErrorCode::Generic),
    }
}

pub async fn get_starred2(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<browsing::FolderParam>,
) -> Response {
    if !browsing::folder_selected(params.music_folder_id()) {
        return ok(&auth, json!({ "starred2": {} }));
    }
    match starred_payload(&auth, &state, true).await {
        Ok(body) => ok(&auth, json!({ "starred2": body })),
        Err(()) => err(&auth, SubsonicErrorCode::Generic),
    }
}

// ── Search ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct Search2Params {
    #[serde(rename = "musicFolderId")]
    music_folder_id: Option<i64>,
    query: Option<String>,
    #[serde(rename = "songCount")]
    song_count: Option<i64>,
    #[serde(rename = "artistCount")]
    artist_count: Option<i64>,
    #[serde(rename = "albumCount")]
    album_count: Option<i64>,
}

/// The folder-era search. Identical results to `search3`, wrapped in
/// `searchResult2` with directory-shaped artist and album entries.
pub async fn search2(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<Search2Params>,
) -> Response {
    if !browsing::folder_selected(params.music_folder_id) {
        return ok(&auth, json!({ "searchResult2": {} }));
    }
    let query = params.query.unwrap_or_default();
    let needle = query.trim().to_lowercase();

    let song_limit = params.song_count.unwrap_or(20).clamp(0, 500);
    let tracks = if song_limit > 0 {
        match db::subsonic::search_tracks(state.db(), auth.viewer(), &query, song_limit).await {
            Ok(t) => t,
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        }
    } else {
        Vec::new()
    };
    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let songs: Vec<Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();

    let artist_limit = params.artist_count.unwrap_or(20).clamp(0, 500) as usize;
    let album_limit = params.album_count.unwrap_or(20).clamp(0, 500) as usize;

    let artists: Vec<Value> = if artist_limit > 0 && !needle.is_empty() {
        match db::subsonic::list_distinct_artists(state.db(), auth.viewer()).await {
            Ok(rows) => rows
                .iter()
                .filter(|a| a.artist.to_lowercase().contains(&needle))
                .take(artist_limit)
                .map(|a| {
                    json!({
                        "id": ids::artist_id(auth.user_id, &a.artist).to_string(),
                        "name": a.artist,
                        "isDir": true,
                    })
                })
                .collect(),
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        }
    } else {
        Vec::new()
    };

    let albums: Vec<Value> = if album_limit > 0 && !needle.is_empty() {
        match db::subsonic::list_distinct_albums(state.db(), auth.viewer()).await {
            Ok(rows) => rows
                .iter()
                .filter(|a| a.album.to_lowercase().contains(&needle))
                .take(album_limit)
                .map(|a| {
                    let id = ids::album_id(auth.user_id, &a.artist, &a.album);
                    json!({
                        "id": id.to_string(),
                        "parent": ids::artist_id(auth.user_id, &a.artist).to_string(),
                        "title": a.album,
                        "album": a.album,
                        "artist": a.artist,
                        "isDir": true,
                        "songCount": a.song_count,
                        "duration": a.duration_secs.unwrap_or(0),
                        "created": a.created_at.to_rfc3339(),
                        "coverArt": a.cover_object_id.map(|_| id.to_string()),
                    })
                })
                .collect(),
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        }
    } else {
        Vec::new()
    };

    ok(
        &auth,
        json!({
            "searchResult2": {
                "artist": artists,
                "album": albums,
                "song": songs,
            }
        }),
    )
}
