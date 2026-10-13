// SPDX-License-Identifier: AGPL-3.0-or-later
//! A public demo's shared account is read-only.
//!
//! Its email and password are published (`GET /api/v1/server` → `demo`), so anyone could
//! otherwise change the password, rotate the Subsonic key or delete the music, and break the
//! demo for everyone else until the next reset. Listening is still allowed — playing, progress,
//! the play queue, scrobbles — and so are reads that happen to be POSTs (searches, a smart
//! playlist's preview). Everything else that writes is refused.
use crate::app::AppState;
use axum::http::Method;
use uuid::Uuid;

/// Whether `email` is a demo account that may only read. `SERVER__DEMO__READ_ONLY=false` lifts
/// it, for the seed script that fills the demo after a reset.
pub fn is_read_only_account(state: &AppState, email: &str) -> bool {
    state
        .config()
        .server
        .demo
        .as_ref()
        .is_some_and(|d| d.read_only && d.email.trim().eq_ignore_ascii_case(email.trim()))
}

/// The same for a REST request, where only the user id is known: looked up only for a write,
/// and only on a server with a read-only demo, so other requests cost nothing.
pub async fn refuses(state: &AppState, method: &Method, path: &str, user_id: Uuid) -> bool {
    let demo_is_read_only = state.config().server.demo.as_ref().is_some_and(|d| d.read_only);
    if !demo_is_read_only || !is_write(method) || rest_allows(method, path) {
        return false;
    }
    is_read_only_user(state, user_id).await
}

/// Whether this user is the read-only demo account; for a request that mixes listening with
/// edits (`POST /api/v2/sync/push`) and refuses only the edits.
pub async fn is_read_only_user(state: &AppState, user_id: Uuid) -> bool {
    if !state.config().server.demo.as_ref().is_some_and(|d| d.read_only) {
        return false;
    }
    match crate::db::users::find_by_id(state.db(), user_id).await {
        Ok(Some(user)) => is_read_only_account(state, &user.email),
        _ => false,
    }
}

fn is_write(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS)
}

/// REST writes the demo account may still make.
fn rest_allows(method: &Method, path: &str) -> bool {
    let segments: Vec<&str> = path.trim_end_matches('/').split('/').skip_while(|s| s.is_empty()).collect();
    let rest = match segments.as_slice() {
        ["api", "v1", rest @ ..] => rest,
        ["api", "v2", rest @ ..] => return v2_allows(method, rest),
        _ => return false,
    };
    match (method, rest) {
        // The session itself.
        (_, ["auth", "logout" | "refresh" | "fork"]) => true,
        (_, ["auth", "device", "start" | "poll"]) => true,
        (_, ["devices", "push-token"]) | (_, ["devices", "notifications", "ack"]) => true,
        // Listening.
        (&Method::POST, ["playback", "sessions"]) => true,
        (&Method::PUT, ["playback", "queue"]) => true,
        (&Method::PUT, ["music", "tracks", _, "progress"]) => true,
        (&Method::PUT, ["playback", "books", _, "progress"]) => true,
        (&Method::PUT, ["playback", "episodes", _, "progress"]) => true,
        (&Method::POST, ["playback", "episodes", "progress", "bulk"]) => true,
        // Reads sent as POST.
        (&Method::POST, ["podcasts", "search"]) => true,
        (&Method::POST, ["music", "intent"]) => true,
        (&Method::POST, ["music", "smart-playlists", "resolve"]) => true,
        (&Method::POST, ["music", "smart-playlists", _, "resolve"]) => true,
        _ => false,
    }
}

/// The same on `/api/v2`. `/sync/push` carries plays and progress with favourites and
/// ratings; it is let through and refuses those two item by item.
fn v2_allows(method: &Method, rest: &[&str]) -> bool {
    matches!(
        (method, rest),
        (&Method::POST, ["auth", "revoke"])
            | (&Method::POST, ["playback-info"])
            | (&Method::PUT, ["me", "progress", _, _])
            | (&Method::POST, ["me", "plays"])
            | (&Method::POST, ["sync", "push"])
            // Reads sent as POST: what a smart playlist or a rule holds now.
            | (&Method::POST, ["smart-playlists", "resolve"])
            | (&Method::POST, ["smart-playlists", _, "resolve"])
    )
}

/// Subsonic endpoints that change something other than listening state.
pub fn subsonic_refuses(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("");
    let name = name.strip_suffix(".view").unwrap_or(name);
    matches!(
        name,
        "star" | "unstar" | "setRating"
            | "createPlaylist" | "updatePlaylist" | "deletePlaylist"
            | "createShare" | "updateShare" | "deleteShare"
            | "changePassword" | "createUser" | "updateUser" | "deleteUser"
            | "createInternetRadioStation" | "updateInternetRadioStation" | "deleteInternetRadioStation"
            | "createPodcastChannel" | "deletePodcastChannel" | "deletePodcastEpisode"
            | "downloadPodcastEpisode" | "refreshPodcasts"
            | "startScan" | "jukeboxControl"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn listening_and_reads_are_allowed() {
        assert!(rest_allows(&Method::POST, "/api/v1/playback/sessions"));
        assert!(rest_allows(&Method::PUT, "/api/v1/music/tracks/0b9c/progress"));
        assert!(rest_allows(&Method::POST, "/api/v1/music/smart-playlists/abc/resolve"));
        assert!(rest_allows(&Method::POST, "/api/v1/auth/logout"));
        assert!(rest_allows(&Method::POST, "/api/v2/playback-info"));
        assert!(rest_allows(&Method::PUT, "/api/v2/me/progress/track/0b9c"));
        assert!(rest_allows(&Method::POST, "/api/v2/me/plays"));
        assert!(rest_allows(&Method::POST, "/api/v2/sync/push"));
        assert!(rest_allows(&Method::POST, "/api/v2/smart-playlists/resolve"));
        assert!(rest_allows(&Method::POST, "/api/v2/smart-playlists/0b9c/resolve"));
        assert!(!rest_allows(&Method::POST, "/api/v2/smart-playlists/0b9c/freeze"));
    }

    #[test]
    fn v2_edits_are_refused() {
        assert!(!rest_allows(&Method::PUT, "/api/v2/me/favourites/track/0b9c"));
        assert!(!rest_allows(&Method::POST, "/api/v2/playlists"));
        assert!(!rest_allows(&Method::PATCH, "/api/v2/albums/0b9c"));
        assert!(!rest_allows(&Method::DELETE, "/api/v2/me/progress/track/0b9c"));
    }

    #[test]
    fn edits_are_refused() {
        assert!(!rest_allows(&Method::POST, "/api/v1/auth/password"));
        assert!(!rest_allows(&Method::POST, "/api/v1/users/me/subsonic-key/regenerate"));
        assert!(!rest_allows(&Method::DELETE, "/api/v1/music/tracks/0b9c"));
        assert!(!rest_allows(&Method::POST, "/api/v1/music/playlists"));
        // A resolve that writes, under another path.
        assert!(!rest_allows(&Method::POST, "/api/v1/family/content-reports/abc/resolve"));
        assert!(is_write(&Method::PATCH) && !is_write(&Method::GET));
    }

    #[test]
    fn subsonic_writes_with_or_without_view() {
        assert!(subsonic_refuses("/rest/star.view"));
        assert!(subsonic_refuses("/rest/changePassword"));
        assert!(!subsonic_refuses("/rest/scrobble.view"));
        assert!(!subsonic_refuses("/rest/savePlayQueue"));
        assert!(!subsonic_refuses("/rest/getAlbum.view"));
    }
}
