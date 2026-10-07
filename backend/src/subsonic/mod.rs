// SPDX-License-Identifier: AGPL-3.0-or-later
/// OpenSubsonic-compatible API surface (music only) — lets existing
/// Subsonic-ecosystem clients (DSub, Symfonium, play:Sub, ...) browse and
/// stream the user's music library. Mounted at `/rest`, a sibling of
/// `/api/v1`, since Subsonic clients call fixed paths like
/// `/rest/ping.view`.
///
/// Podcasts are served here too: audio2 already had feeds, episodes, refresh,
/// subscribe and download on `/api/v1/podcasts`, so this is the same data in
/// the shape the protocol names rather than new capability. Audiobooks remain
/// out of scope — Subsonic has no concept that fits them, and they stay on
/// audio2's own API.
pub mod annotation;
pub mod auth;
pub mod browsing;
pub mod envelope;
pub mod extract;
pub mod form;
pub mod ids;
pub mod lists;
pub mod playback;
pub mod playlists;
pub mod resolve;
pub mod podcasts;
pub mod system;

use crate::app::AppState;
use axum::Router;
use axum::extract::Query;
use axum::handler::Handler;
use axum::response::Response;
use axum::routing::{MethodRouter, get};
use envelope::{ResponseFormat, SubsonicErrorCode};
use serde::Deserialize;

/// Subsonic clients suffix every endpoint with `.view` (a historical Java
/// servlet convention); register both spellings for each route.
trait ViewAlias {
    fn view(self, name: &str, method_router: MethodRouter<AppState>) -> Self;
}

impl ViewAlias for Router<AppState> {
    fn view(self, name: &str, method_router: MethodRouter<AppState>) -> Self {
        self.route(&format!("/{name}"), method_router.clone())
            .route(&format!("/{name}.view"), method_router)
    }
}

/// Registers a handler for both GET and POST.
///
/// Subsonic is a GET protocol by tradition, but the OpenSubsonic `formPost`
/// extension allows the same parameters in a form body — see `form.rs`, which
/// folds that body into the query so the handler itself is unchanged. Both
/// verbs reach the same code, which is what lets `formPost` be declared
/// honestly.
fn both<H, T>(handler: H) -> MethodRouter<AppState>
where
    H: Handler<T, AppState> + Clone,
    T: 'static,
{
    get(handler.clone()).post(handler)
}

#[derive(Deserialize, Default)]
struct FormatOnly {
    f: Option<String>,
    callback: Option<String>,
}

/// Answers any `/rest` path with no route of its own.
///
/// Without this the request falls through to the SPA's static file service and
/// the client receives a bare HTTP 404 with an empty body. Subsonic clients
/// expect a `subsonic-response` envelope even for refusals, and several treat
/// a non-envelope reply as "this is not a Subsonic server" rather than as one
/// unsupported method — so a single missing endpoint could break the whole
/// connection instead of hiding one button.
///
/// Deliberately unauthenticated: a client probing for a feature should be told
/// the method does not exist whether or not it sent usable credentials.
async fn unknown_method(Query(params): Query<FormatOnly>) -> Response {
    envelope::error(
        ResponseFormat::from_param(params.f.as_deref()),
        params.callback.as_deref(),
        SubsonicErrorCode::NotFound,
        Some("The requested endpoint is not supported by this server"),
    )
}

pub fn router() -> Router<AppState> {
    Router::new()
        // System
        .view("ping", both(browsing::ping))
        .view("getLicense", both(browsing::get_license))
        .view(
            "getOpenSubsonicExtensions",
            both(browsing::get_open_subsonic_extensions),
        )
        .view("getMusicFolders", both(browsing::get_music_folders))
        .view("getUser", both(system::get_user))
        .view("getScanStatus", both(system::get_scan_status))
        .view("startScan", both(system::start_scan))
        .view(
            "getInternetRadioStations",
            both(system::get_internet_radio_stations),
        )
        .view("getNowPlaying", both(system::get_now_playing))
        // Browsing
        .view("getArtists", both(browsing::get_artists))
        .view("getIndexes", both(browsing::get_indexes))
        .view("getArtist", both(browsing::get_artist))
        .view("getMusicDirectory", both(browsing::get_music_directory))
        .view("getGenres", both(browsing::get_genres))
        .view("getAlbumList", both(lists::get_album_list))
        .view("getAlbumList2", both(browsing::get_album_list2))
        .view("getAlbum", both(browsing::get_album))
        .view("getSong", both(browsing::get_song))
        .view("getArtistInfo", both(browsing::get_artist_info))
        .view("getArtistInfo2", both(browsing::get_artist_info2))
        .view("getAlbumInfo", both(browsing::get_album_info))
        .view("getAlbumInfo2", both(browsing::get_album_info2))
        .view("getSimilarSongs", both(browsing::get_similar_songs))
        .view("getSimilarSongs2", both(browsing::get_similar_songs2))
        .view("getTopSongs", both(browsing::get_top_songs))
        // Lists
        .view("getRandomSongs", both(lists::get_random_songs))
        .view("getSongsByGenre", both(lists::get_songs_by_genre))
        .view("getStarred", both(lists::get_starred))
        .view("getStarred2", both(lists::get_starred2))
        // Search
        .view("search2", both(lists::search2))
        .view("search3", both(browsing::search3))
        // Media retrieval
        .view("stream", both(playback::stream))
        .view("download", both(playback::download))
        .view("getCoverArt", both(playback::get_cover_art))
        .view("getLyrics", both(playback::get_lyrics))
        .view("getLyricsBySongId", both(playback::get_lyrics_by_song_id))
        // Annotation
        .view("star", both(annotation::star))
        .view("unstar", both(annotation::unstar))
        .view("setRating", both(annotation::set_rating))
        .view("scrobble", both(playback::scrobble))
        // Bookmarks and play queue
        .view("getBookmarks", both(annotation::get_bookmarks))
        .view("createBookmark", both(annotation::create_bookmark))
        .view("deleteBookmark", both(annotation::delete_bookmark))
        .view("savePlayQueue", both(playback::save_play_queue))
        .view("getPlayQueue", both(playback::get_play_queue))
        // Podcasts
        .view("getPodcasts", both(podcasts::get_podcasts))
        .view("getNewestPodcasts", both(podcasts::get_newest_podcasts))
        .view("refreshPodcasts", both(podcasts::refresh_podcasts))
        .view("createPodcastChannel", both(podcasts::create_podcast_channel))
        .view("deletePodcastChannel", both(podcasts::delete_podcast_channel))
        .view(
            "downloadPodcastEpisode",
            both(podcasts::download_podcast_episode),
        )
        .view("deletePodcastEpisode", both(podcasts::delete_podcast_episode))
        // Playlists
        .view("getPlaylists", both(playlists::get_playlists))
        .view("getPlaylist", both(playlists::get_playlist))
        .view("createPlaylist", both(playlists::create_playlist))
        .view("updatePlaylist", both(playlists::update_playlist))
        .view("deletePlaylist", both(playlists::delete_playlist))
        .fallback(unknown_method)
}
