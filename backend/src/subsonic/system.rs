// SPDX-License-Identifier: AGPL-3.0-or-later
/// Endpoints clients call while connecting, rather than to browse anything:
/// capability discovery (`getUser`), library-scan state, and internet radio.
///
/// These matter out of proportion to their content. A client that cannot read
/// a user's roles at login may disable downloading, playlist editing or
/// scrobbling for the whole session, or refuse the connection outright — so
/// answering them plainly is what makes audio2 look like a normal server.
use crate::app::AppState;
use crate::db;
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::extract::SubsonicQuery;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
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

#[derive(Deserialize)]
pub struct UsernameParam {
    username: Option<String>,
}

/// The roles reported here describe what `/rest` itself will actually do, not
/// what the audio2 account can do elsewhere. Upload, sharing, podcast and
/// jukebox are all false because this surface implements none of them —
/// claiming otherwise would make a client offer buttons that then fail.
pub async fn get_user(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<UsernameParam>,
) -> Response {
    // Asking about somebody else is an administrative act; audio2 exposes no
    // user administration over Subsonic, so only self-lookup is answered.
    if let Some(requested) = params.username.as_deref() {
        if !requested.eq_ignore_ascii_case(&auth.username) {
            return err(&auth, SubsonicErrorCode::NotAuthorized);
        }
    }

    let _ = state;
    ok(
        &auth,
        json!({
            "user": {
                "username": auth.username,
                "email": auth.username,
                "scrobblingEnabled": true,
                "adminRole": false,
                "settingsRole": false,
                "downloadRole": true,
                "uploadRole": false,
                "playlistRole": true,
                "coverArtRole": false,
                "commentRole": false,
                "podcastRole": false,
                "streamRole": true,
                "jukeboxRole": false,
                "shareRole": false,
                "videoConversionRole": false,
                "folder": [1],
            }
        }),
    )
}

/// Uploads are in the library the moment they exist; read-only library
/// folders (`crate::library_folders`) are scanned. `scanning` reports that
/// scan, `count` the tracks the caller can see.
pub async fn get_scan_status(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    let count = db::music::count_tracks(state.db(), auth.viewer()).await.unwrap_or(0);
    let (scanning, _) = crate::library_folders::scan_status();
    ok(
        &auth,
        json!({ "scanStatus": { "scanning": scanning, "count": count } }),
    )
}

/// Starts a library-folder scan when there are folders; without any it is
/// still answered, so a client's "rescan" button never shows a fault.
pub async fn start_scan(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    crate::library_folders::request_scan();
    get_scan_status(auth, State(state)).await
}

/// Always empty: audio2 stores a family's own recordings and has no stream
/// directory. An empty list is a valid answer and keeps the client's radio
/// tab quiet, where an error would look like a broken server.
pub async fn get_internet_radio_stations(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({ "internetRadioStations": {} }))
}

/// Reported per-response rather than tracked: audio2 records listening
/// sessions for its own statistics, but "who is playing what right now"
/// across a family is not something this surface exposes.
pub async fn get_now_playing(auth: SubsonicAuthUser) -> Response {
    ok(&auth, json!({ "nowPlaying": {} }))
}
