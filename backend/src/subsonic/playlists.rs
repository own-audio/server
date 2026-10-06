// SPDX-License-Identifier: AGPL-3.0-or-later
/// Playlist endpoints — thin wrappers over the existing `db::music` playlist
/// queries used by the app's own `/api/v1/music/playlists` routes.
use crate::app::AppState;
use crate::db;
use crate::music::models::{MusicPlaylist, MusicTrack};
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::browsing::{SongContext, song_json};
use crate::subsonic::extract::SubsonicQuery;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
use axum::extract::State;
use axum::response::Response;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use uuid::Uuid;

fn ok(auth: &SubsonicAuthUser, body: Value) -> Response {
    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), body)
}

fn err(auth: &SubsonicAuthUser, code: SubsonicErrorCode) -> Response {
    envelope::error(auth.format, auth.jsonp_callback.as_deref(), code, None)
}

/// Account emails for the owners of these playlists, keyed by user id.
///
/// A family-shared playlist can belong to another member, so the caller's own
/// name is not enough.
async fn owner_names(state: &AppState, playlists: &[MusicPlaylist]) -> HashMap<Uuid, String> {
    let mut ids: Vec<Uuid> = playlists.iter().map(|p| p.user_id).collect();
    ids.sort_unstable();
    ids.dedup();
    db::users::emails_for(state.db(), &ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect()
}

/// Falls back to the id only if the owner row has vanished — better a stable
/// opaque string than an absent required attribute.
fn owner_name(owners: &HashMap<Uuid, String>, playlist: &MusicPlaylist) -> String {
    owners
        .get(&playlist.user_id)
        .cloned()
        .unwrap_or_else(|| playlist.user_id.to_string())
}

/// `owner` is a **username**, not an id. Clients display it, and several
/// compare it to the name they logged in with to decide whether a playlist is
/// theirs to edit — a raw UUID fails that test against every playlist,
/// including the user's own. audio2 has no separate username, so the account
/// email is the name, exactly as `getUser` reports it.
fn playlist_json(
    playlist: &MusicPlaylist,
    owner: &str,
    song_count: i64,
    duration_secs: i64,
) -> Value {
    json!({
        "id": playlist.id.to_string(),
        "name": playlist.name,
        "comment": playlist.description,
        "owner": owner,
        "public": false,
        "songCount": song_count,
        "duration": duration_secs,
        "created": playlist.created_at.to_rfc3339(),
        "changed": playlist.updated_at.to_rfc3339(),
        "coverArt": playlist.cover_object_id.map(|_| playlist.id.to_string()),
    })
}

pub async fn get_playlists(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    let playlists = match db::music::list_playlists(state.db(), auth.viewer()).await {
        Ok(p) => p,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let ids: Vec<Uuid> = playlists.iter().map(|p| p.id).collect();
    let stats: HashMap<Uuid, (i64, i64)> =
        db::subsonic::playlist_track_stats(state.db(), &ids)
            .await
            .unwrap_or_default()
            .into_iter()
            .map(|(id, count, duration)| (id, (count, duration)))
            .collect();
    let owners = owner_names(&state, &playlists).await;

    let items: Vec<Value> = playlists
        .iter()
        .map(|playlist| {
            let (count, duration) = stats.get(&playlist.id).copied().unwrap_or((0, 0));
            playlist_json(playlist, &owner_name(&owners, playlist), count, duration)
        })
        .collect();

    ok(&auth, json!({ "playlists": { "playlist": items } }))
}

#[derive(Deserialize)]
pub struct PlaylistIdParam {
    id: String,
}

pub async fn get_playlist(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<PlaylistIdParam>,
) -> Response {
    let Ok(playlist_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let playlist = match db::music::find_playlist(state.db(), playlist_id, auth.viewer()).await {
        Ok(Some(p)) => p,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let entries = match db::music::list_playlist_tracks(state.db(), playlist_id).await {
        Ok(e) => e,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let entry_ids: Vec<Uuid> = entries.iter().map(|e| e.track_id).collect();
    let fetched = db::subsonic::tracks_by_ids(state.db(), auth.viewer(), &entry_ids)
        .await
        .unwrap_or_default();
    let by_id: HashMap<Uuid, &MusicTrack> = fetched.iter().map(|t| (t.id, t)).collect();
    // Playlist order is `music_playlist_tracks.position`, which the bulk fetch
    // does not preserve.
    let tracks: Vec<MusicTrack> = entry_ids
        .iter()
        .filter_map(|id| by_id.get(id).map(|t| (*t).clone()))
        .collect();

    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;
    let songs: Vec<serde_json::Value> = tracks.iter().map(|t| song_json(t, &ctx)).collect();

    let duration: i64 = tracks.iter().filter_map(|t| t.duration_secs).map(i64::from).sum();
    let owners = owner_names(&state, std::slice::from_ref(&playlist)).await;
    let mut body = playlist_json(
        &playlist,
        &owner_name(&owners, &playlist),
        songs.len() as i64,
        duration,
    );
    body["entry"] = json!(songs);

    ok(&auth, json!({ "playlist": body }))
}

#[derive(Deserialize)]
pub struct CreatePlaylistParams {
    name: Option<String>,
    #[serde(default)]
    #[serde(rename = "songId")]
    song_id: Vec<String>,
}

pub async fn create_playlist(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<CreatePlaylistParams>,
) -> Response {
    let Some(name) = params.name.as_deref().map(str::trim).filter(|s| !s.is_empty()) else {
        return err(&auth, SubsonicErrorCode::MissingParam);
    };

    let playlist = match db::music::insert_playlist(state.db(), auth.user_id, None, name, None, false).await {
        Ok(p) => p,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    for song_id in &params.song_id {
        if let Ok(track_id) = song_id.parse::<Uuid>() {
            let _ = db::music::add_track_to_playlist(state.db(), playlist.id, track_id).await;
        }
    }

    let (count, duration) = db::subsonic::playlist_track_stats(state.db(), &[playlist.id])
        .await
        .unwrap_or_default()
        .first()
        .map(|(_, c, d)| (*c, *d))
        .unwrap_or((0, 0));
    let owners = owner_names(&state, std::slice::from_ref(&playlist)).await;
    ok(
        &auth,
        json!({ "playlist": playlist_json(&playlist, &owner_name(&owners, &playlist), count, duration) }),
    )
}

#[derive(Deserialize)]
pub struct UpdatePlaylistParams {
    #[serde(rename = "playlistId")]
    playlist_id: String,
    name: Option<String>,
    comment: Option<String>,
    #[serde(default)]
    #[serde(rename = "songIdToAdd")]
    song_id_to_add: Vec<String>,
    #[serde(default)]
    #[serde(rename = "songIndexToRemove")]
    song_index_to_remove: Vec<usize>,
}

pub async fn update_playlist(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<UpdatePlaylistParams>,
) -> Response {
    let Ok(playlist_id) = params.playlist_id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let playlist = match db::music::find_playlist(state.db(), playlist_id, auth.viewer()).await {
        Ok(Some(p)) => p,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    if params.name.is_some() || params.comment.is_some() {
        let name = params.name.as_deref().unwrap_or(&playlist.name);
        let description = params.comment.as_deref().or(playlist.description.as_deref());
        if db::music::update_playlist(state.db(), playlist_id, auth.user_id, name, description)
            .await
            .is_err()
        {
            return err(&auth, SubsonicErrorCode::Generic);
        }
    }

    for song_id in &params.song_id_to_add {
        if let Ok(track_id) = song_id.parse::<Uuid>() {
            let _ = db::music::add_track_to_playlist(state.db(), playlist_id, track_id).await;
        }
    }

    if !params.song_index_to_remove.is_empty() {
        if let Ok(entries) = db::music::list_playlist_tracks(state.db(), playlist_id).await {
            for &index in &params.song_index_to_remove {
                if let Some(entry) = entries.get(index) {
                    let _ = db::music::remove_track_from_playlist(state.db(), playlist_id, entry.id).await;
                }
            }
        }
    }

    ok(&auth, json!({}))
}

pub async fn delete_playlist(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<PlaylistIdParam>,
) -> Response {
    let Ok(playlist_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    // Same trash as the REST route, so a playlist deleted from a Subsonic app
    // is restorable for 30 days and every client learns it is gone.
    match crate::trash::move_to_trash(&state, auth.viewer(), None, crate::db::trash::Kind::Playlist, playlist_id).await {
        Ok(()) => ok(&auth, json!({})),
        Err(crate::auth::error::AuthError::ItemNotFound) => err(&auth, SubsonicErrorCode::NotFound),
        Err(crate::auth::error::AuthError::Forbidden) => err(&auth, SubsonicErrorCode::NotAuthorized),
        Err(_) => err(&auth, SubsonicErrorCode::Generic),
    }
}
