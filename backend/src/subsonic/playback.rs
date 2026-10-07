// SPDX-License-Identifier: AGPL-3.0-or-later
/// Media retrieval: stream/download (redirect to a presigned Garage URL),
/// getCoverArt (proxied bytes, like `music::get_cover`), and scrobble.
use crate::app::AppState;
use crate::db;
use crate::music::models::MusicTrack;
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::browsing::{SongContext, song_json};
use crate::subsonic::extract::SubsonicQuery;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
use crate::subsonic::resolve;
use axum::body::Body;
use axum::extract::State;
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Redirect, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::io::Cursor;
use uuid::Uuid;

const STREAM_EXPIRY_SECS: u64 = 4 * 3600;

fn err(auth: &SubsonicAuthUser, code: SubsonicErrorCode) -> Response {
    envelope::error(auth.format, auth.jsonp_callback.as_deref(), code, None)
}

/// A refusal from an endpoint whose success case is raw bytes.
///
/// The envelope is still returned, because a client that inspects the body
/// should find a real Subsonic error code in it — but the HTTP status is no
/// longer 200. A media player does not inspect the body: it hands whatever
/// arrives to the audio decoder, and a 200 response containing XML produced
/// "the specified file type is not supported" rather than a clean failure.
///
/// Only `stream`, `download` and `getCoverArt` use this. Every other endpoint
/// returns 200 with an envelope, which is what the protocol expects and what
/// clients parse.
fn binary_err(auth: &SubsonicAuthUser, code: SubsonicErrorCode) -> Response {
    let status = match code {
        SubsonicErrorCode::NotFound => StatusCode::NOT_FOUND,
        SubsonicErrorCode::NotAuthorized | SubsonicErrorCode::WrongCredentials => {
            StatusCode::FORBIDDEN
        }
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let mut response = err(auth, code);
    *response.status_mut() = status;
    response
}

#[derive(Deserialize)]
pub struct IdParam {
    id: String,
}

pub async fn stream(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    let Ok(id) = params.id.parse::<Uuid>() else {
        return binary_err(&auth, SubsonicErrorCode::NotFound);
    };

    let object_id = match resolve_audio_source(&auth, &state, id).await {
        Ok(Some(AudioSource::Stored(object_id))) => object_id,
        // Not downloaded: hand the client the origin, which also gives it the
        // range support seeking needs.
        Ok(Some(AudioSource::Remote(url))) => return Redirect::temporary(&url).into_response(),
        Ok(None) => return binary_err(&auth, SubsonicErrorCode::NotFound),
        Err(()) => return binary_err(&auth, SubsonicErrorCode::Generic),
    };

    let key = match db::subsonic::media_object_key(state.db(), object_id).await {
        Ok(Some(key)) => key,
        Ok(None) => return binary_err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return binary_err(&auth, SubsonicErrorCode::Generic),
    };

    match state.storage().presigned_get(&key, STREAM_EXPIRY_SECS).await {
        Ok(url) => Redirect::temporary(&url).into_response(),
        Err(_) => binary_err(&auth, SubsonicErrorCode::Generic),
    }
}

/// Where a playable item's audio actually lives.
pub(crate) enum AudioSource {
    /// An object in audio2's own storage.
    Stored(Uuid),
    /// A podcast episode that has not been downloaded, still at its origin.
    Remote(String),
}

/// The audio for a music track *or* a podcast episode.
///
/// One id space, two kinds of media: `stream` is handed whatever the client is
/// playing, and a podcast episode reaching a music-only lookup was the reason
/// episodes could be listed but never played.
///
/// **The rule is: serve it if own.audio has it, redirect if not.** Refusing an
/// undownloaded episode was a mistake carried over from music, where audio2 is
/// the only place a file exists. A podcast episode is public and already hosted
/// elsewhere; downloading it is a convenience, not a precondition, and on a
/// real library almost nothing is downloaded — so refusing made the whole
/// podcast tab unplayable.
///
/// **Proxying the origin through this server was considered and declined**
/// (2026-08-22). It would hide the listener's IP from the podcast host, which
/// the redirect does not, but at roughly 56MB in and 56MB out per play on this
/// library's average 22-minute episode, and only after implementing `Range`
/// forwarding — without which seeking breaks and players that probe with a
/// range request before starting refuse the stream outright. Redirecting hands
/// the client the origin's own range support for free.
///
/// Downloading an episode is the answer for anyone who wants their own copy;
/// it is then `Stored` and never leaves own.audio again.
async fn resolve_audio_source(
    auth: &SubsonicAuthUser,
    state: &AppState,
    id: Uuid,
) -> Result<Option<AudioSource>, ()> {
    if let Some(track) = db::music::find_track(state.db(), id, auth.viewer())
        .await
        .map_err(|_| ())?
    {
        return Ok(Some(AudioSource::Stored(track.audio_object_id)));
    }

    let Some(episode) = db::podcasts::find_episode(state.db(), id).await.map_err(|_| ())? else {
        return Ok(None);
    };
    // Episodes carry no owner; the feed they belong to is the access check.
    if db::podcasts::find_feed(state.db(), episode.feed_id, auth.viewer())
        .await
        .map_err(|_| ())?
        .is_none()
    {
        return Ok(None);
    }

    Ok(match (episode.audio_object_id, episode.audio_url) {
        (Some(object_id), _) => Some(AudioSource::Stored(object_id)),
        (None, Some(url)) => Some(AudioSource::Remote(url)),
        (None, None) => None,
    })
}

/// `download` behaves the same as `stream` for this server — both hand back
/// a presigned redirect to the original file.
pub async fn download(
    auth: SubsonicAuthUser,
    state: State<AppState>,
    params: SubsonicQuery<IdParam>,
) -> Response {
    stream(auth, state, params).await
}

#[derive(Deserialize)]
pub struct CoverArtParams {
    id: String,
    size: Option<u32>,
}

/// How long a client may reuse a cover without asking again.
///
/// Cover art is addressed by the id of an immutable stored object, so a URL's
/// bytes never change; only replacing the art changes the id. Without this
/// header a grid of 100 albums re-fetched every full-size image on every
/// scroll back to the top.
const COVER_CACHE_CONTROL: &str = "private, max-age=604800, immutable";

pub async fn get_cover_art(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<CoverArtParams>,
) -> Response {
    let Ok(target) = params.id.parse::<Uuid>() else {
        return binary_err(&auth, SubsonicErrorCode::NotFound);
    };

    let cover_object_id = match resolve_cover_object(&state, auth.viewer(), target).await {
        Ok(Some(id)) => id,
        Ok(None) => return binary_err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return binary_err(&auth, SubsonicErrorCode::Generic),
    };

    let (key, content_type) = match db::subsonic::media_object_key_and_type(state.db(), cover_object_id).await {
        Ok(Some(pair)) => pair,
        Ok(None) => return binary_err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return binary_err(&auth, SubsonicErrorCode::Generic),
    };

    let Ok(bytes) = state.storage().get(&key).await else {
        return binary_err(&auth, SubsonicErrorCode::Generic);
    };

    // Re-encoded as JPEG whatever went in: the protocol only promises an
    // image, and a client asking for a 100 px thumbnail wants the small one,
    // not the format it happened to be stored in.
    let scaled = params.size.and_then(|size| scale_cover(&bytes, size));
    let (body, content_type) = match scaled {
        Some(scaled) => (Body::from(scaled), "image/jpeg".to_string()),
        None => (Body::from(bytes), content_type),
    };

    (
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, COVER_CACHE_CONTROL.to_string()),
        ],
        body,
    )
        .into_response()
}

/// Scale a cover to fit `size` on its longest edge, preserving aspect ratio.
///
/// `size` was previously ignored, so a client drawing a wall of thumbnails
/// pulled the full-resolution original for every tile — on this library, 94 KB
/// per 100 px square.
///
/// Returns `None` — meaning "send the original" — when the bytes are not a
/// decodable image, or when the request is for something at least as large as
/// what is stored. Upscaling would cost CPU to deliver a worse picture.
fn scale_cover(bytes: &[u8], size: u32) -> Option<Vec<u8>> {
    // A zero or absurd size is a client bug; treat it as "no preference"
    // rather than trying to honour it.
    if !(1..=2000).contains(&size) {
        return None;
    }
    let image = image::load_from_memory(bytes).ok()?;
    if image.width().max(image.height()) <= size {
        return None;
    }
    let scaled = image.thumbnail(size, size);
    let mut out = Cursor::new(Vec::new());
    scaled
        .into_rgb8()
        .write_to(&mut out, image::ImageFormat::Jpeg)
        .ok()?;
    Some(out.into_inner())
}

async fn resolve_cover_object(
    state: &AppState,
    viewer: crate::db::access::Viewer,
    id: Uuid,
) -> anyhow::Result<Option<Uuid>> {
    let user_id = viewer.user_id;
    // 1. A real track id with its own cover.
    if let Some(track) = db::music::find_track(state.db(), id, viewer).await? {
        return Ok(track.cover_object_id);
    }

    // 2. A playlist id.
    if let Some(playlist) = db::music::find_playlist(state.db(), id, viewer).await? {
        return Ok(playlist.cover_object_id);
    }

    // 3. A podcast feed, or an episode — an episode falls back to its
    //    channel's artwork, which is what a listener expects against an
    //    episode that ships none of its own.
    if let Some(feed) = db::podcasts::find_feed(state.db(), id, viewer).await? {
        return Ok(feed.image_object_id);
    }
    if let Some(episode) = db::podcasts::find_episode(state.db(), id).await? {
        if let Some(feed) = db::podcasts::find_feed(state.db(), episode.feed_id, viewer).await? {
            return Ok(episode.image_object_id.or(feed.image_object_id));
        }
    }

    // 4. A synthetic album or artist id. Last, because resolving one that is
    //    not known yet costs a pass over the whole catalog.
    match resolve::resolve(state.db(), viewer, id).await? {
        Some(resolve::Named::Album { artist, album }) => {
            return db::subsonic::representative_cover(state.db(), viewer, &artist, Some(&album)).await;
        }
        //    An artist: their own photo when there is one, falling back to a
        //    representative album cover — an artist tile showing one of their
        //    sleeves is a better answer than an empty frame. Commons-sourced
        //    photos carry an attribution requirement, which `getArtistInfo2`
        //    satisfies by returning the credit in `biography`.
        Some(resolve::Named::Artist(artist)) => {
            if let Some(image) = db::music::find_artist_image_object(state.db(), user_id, &artist).await? {
                return Ok(Some(image));
            }
            return db::subsonic::representative_cover(state.db(), viewer, &artist, None).await;
        }
        None => {}
    }

    Ok(None)
}

#[derive(Deserialize)]
pub struct ScrobbleParams {
    id: String,
    submission: Option<bool>,
}

pub async fn scrobble(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<ScrobbleParams>,
) -> Response {
    let Ok(track_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    let track = match db::music::find_track(state.db(), track_id, auth.viewer()).await {
        Ok(Some(track)) => track,
        // A scrobble for a podcast episode goes to `podcast_progress`, so the
        // position a Subsonic client reaches is the one audio2's own clients
        // resume from — the two share a listening position, not just a library.
        Ok(None) => return scrobble_episode(&auth, &state, track_id, &params).await,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    // `submission=false` is a "now playing" notice, not a completed play —
    // only record progress for actual scrobbles (the default).
    if params.submission.unwrap_or(true) {
        let duration = track.duration_secs.unwrap_or(0) as f64;
        if db::music::upsert_track_progress(state.db(), auth.user_id, track_id, duration, true)
            .await
            .is_err()
        {
            return err(&auth, SubsonicErrorCode::Generic);
        }

        // A scrobble means the track was played through, so the whole
        // duration counts as listened.
        let _ = db::stats::derive_from_progress(
            state.db(),
            auth.user_id,
            crate::db::access::MUSIC,
            track_id,
            None,
            duration,
            "subsonic",
        )
        .await;
    }

    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), json!({}))
}

// ── Play queue ────────────────────────────────────────────────────────────
// Backed by the same `play_queues` row as `/api/v1/playback/queue`, so a
// Subsonic app and the native clients share one queue.

#[derive(Deserialize)]
pub struct SavePlayQueueParams {
    #[serde(default)]
    id: Vec<String>,
    current: Option<String>,
    /// Milliseconds, per the Subsonic spec.
    position: Option<i64>,
}

pub async fn save_play_queue(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<SavePlayQueueParams>,
) -> Response {
    let items: Vec<serde_json::Value> = params
        .id
        .iter()
        .filter_map(|raw| raw.parse::<Uuid>().ok())
        .map(|id| json!({ "media_kind": "music", "item_id": id.to_string() }))
        .collect();

    let current_index = params
        .current
        .as_deref()
        .and_then(|c| params.id.iter().position(|id| id == c))
        .unwrap_or(0) as i32;

    let position_secs = params.position.unwrap_or(0) as f64 / 1000.0;
    let items = serde_json::Value::Array(items);

    match db::sync::put_queue(
        state.db(),
        auth.user_id,
        &items,
        current_index,
        position_secs,
        Some("subsonic"),
        None,
    )
    .await
    {
        Ok(_) => envelope::ok(auth.format, auth.jsonp_callback.as_deref(), json!({})),
        Err(_) => err(&auth, SubsonicErrorCode::Generic),
    }
}

pub async fn get_play_queue(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    let queue = match db::sync::get_queue(state.db(), auth.user_id).await {
        Ok(Some(queue)) => queue,
        // No saved queue is a normal state, not an error.
        Ok(None) => return envelope::ok(auth.format, auth.jsonp_callback.as_deref(), json!({})),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    // The stored queue holds bare ids; the protocol wants each entry rendered
    // as a full song. Emitting `<entry id="..."/>` alone left a client
    // restoring its queue with a list of blank rows it could not label.
    let queued_ids: Vec<Uuid> = queue
        .items
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter(|i| i.get("media_kind").and_then(|k| k.as_str()) == Some("music"))
                .filter_map(|i| i.get("item_id").and_then(|v| v.as_str()))
                .filter_map(|id| id.parse::<Uuid>().ok())
                .collect()
        })
        .unwrap_or_default();

    let tracks = db::subsonic::tracks_by_ids(state.db(), auth.viewer(), &queued_ids)
        .await
        .unwrap_or_default();
    let by_id: std::collections::HashMap<Uuid, &MusicTrack> =
        tracks.iter().map(|t| (t.id, t)).collect();
    let ctx = SongContext::load(state.db(), auth.user_id, &tracks).await;

    // Re-ordered to the saved queue, not to whatever order the rows came back
    // in — a queue whose order is lost is not a restored queue. A track that
    // has since been deleted, or that the viewer may no longer see, simply
    // drops out.
    let entries: Vec<serde_json::Value> = queued_ids
        .iter()
        .filter_map(|id| by_id.get(id).map(|t| song_json(t, &ctx)))
        .collect();

    let current = queue
        .items
        .as_array()
        .and_then(|items| items.get(queue.current_index.max(0) as usize))
        .and_then(|i| i.get("item_id").and_then(|v| v.as_str()))
        .map(|s| s.to_string());

    envelope::ok(
        auth.format,
        auth.jsonp_callback.as_deref(),
        json!({
            "playQueue": {
                "entry": entries,
                "current": current,
                "position": (queue.position_secs * 1000.0) as i64,
                "changed": queue.updated_at.to_rfc3339(),
                "changedBy": queue.updated_by_device.unwrap_or_else(|| "audio2".to_string()),
            }
        }),
    )
}

// ── Lyrics ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LyricsParams {
    artist: Option<String>,
    title: Option<String>,
}

/// The original `getLyrics`, which searches by artist and title rather than by
/// id. audio2 fetches lyrics from nowhere, so this only ever finds words the
/// owner typed or that were already embedded in their own file — matched
/// against the library rather than any external service.
pub async fn get_lyrics(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<LyricsParams>,
) -> Response {
    let title = params.title.unwrap_or_default();
    let artist = params.artist.unwrap_or_default();
    if title.trim().is_empty() {
        return envelope::ok(
            auth.format,
            auth.jsonp_callback.as_deref(),
            json!({ "lyrics": {} }),
        );
    }

    let tracks = match db::subsonic::search_tracks(state.db(), auth.viewer(), title.trim(), 25).await {
        Ok(t) => t,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let matched = tracks.into_iter().find(|t| {
        let title_matches = t.title.eq_ignore_ascii_case(title.trim());
        let artist_matches = artist.trim().is_empty()
            || t.artist
                .as_deref()
                .is_some_and(|a| a.eq_ignore_ascii_case(artist.trim()));
        title_matches && artist_matches
    });

    let body = match matched {
        Some(track) => {
            let text = track.lyrics.unwrap_or_default();
            json!({
                "lyrics": {
                    "artist": track.artist,
                    "title": track.title,
                    "value": text,
                }
            })
        }
        None => json!({ "lyrics": {} }),
    };

    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), body)
}

/// OpenSubsonic's `getLyricsBySongId`, which carries structure the original
/// cannot: a timed lyric comes back as individual lines with millisecond
/// offsets rather than one blob of text.
///
/// Worth having because audio2 already stores whatever the owner pasted
/// verbatim, `.lrc` timestamps included — the same raw text the Mac player
/// parses to follow the playhead. Without this endpoint a Subsonic client
/// would only ever see the timestamps as literal characters.
pub async fn get_lyrics_by_song_id(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<IdParam>,
) -> Response {
    let Ok(track_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };
    let track = match db::music::find_track(state.db(), track_id, auth.viewer()).await {
        Ok(Some(t)) => t,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    let raw = track.lyrics.clone().unwrap_or_default();
    if raw.trim().is_empty() {
        return envelope::ok(
            auth.format,
            auth.jsonp_callback.as_deref(),
            json!({ "lyricsList": {} }),
        );
    }

    let lines = parse_lrc(&raw);
    let synced = !lines.is_empty();
    let structured: Vec<Value> = if synced {
        lines
            .into_iter()
            .map(|(ms, text)| json!({ "start": ms, "value": text }))
            .collect()
    } else {
        raw.lines().map(|l| json!({ "value": l })).collect()
    };

    envelope::ok(
        auth.format,
        auth.jsonp_callback.as_deref(),
        json!({
            "lyricsList": {
                "structuredLyrics": [{
                    "lang": "xxx",
                    "synced": synced,
                    "displayArtist": track.artist,
                    "displayTitle": track.title,
                    "line": structured,
                }]
            }
        }),
    )
}

/// Returns `(offset_ms, text)` per timed line, or empty when the text carries
/// no timestamps at all.
///
/// A bracket only counts when it holds a parseable time: lyric sheets marked
/// up with `[Verse 1]` and `[Chorus]` are extremely common, and reading those
/// as timestamps would collapse a whole song onto one mistimed line — worse
/// than serving it as plain text.
fn parse_lrc(raw: &str) -> Vec<(i64, String)> {
    let mut out: Vec<(i64, String)> = Vec::new();

    for line in raw.lines() {
        let mut remainder = line.trim_start();
        let mut stamps: Vec<i64> = Vec::new();

        while remainder.starts_with('[') {
            let Some(close) = remainder.find(']') else { break };
            let Some(ms) = parse_lrc_timestamp(&remainder[1..close]) else {
                break;
            };
            stamps.push(ms);
            remainder = &remainder[close + 1..];
        }

        // One line may carry several timestamps — LRC's way of writing a
        // repeated chorus once. Each becomes its own cue.
        for ms in stamps {
            out.push((ms, remainder.trim().to_string()));
        }
    }

    out.sort_by_key(|(ms, _)| *ms);
    out
}

/// `mm:ss`, `mm:ss.xx`, and the legacy `mm:ss:xx` older tools emit.
fn parse_lrc_timestamp(value: &str) -> Option<i64> {
    let parts: Vec<&str> = value.split(':').collect();
    if parts.len() < 2 {
        return None;
    }
    let minutes: f64 = parts[0].trim().parse().ok()?;
    if parts.len() == 2 {
        let seconds: f64 = parts[1].trim().parse().ok()?;
        return Some(((minutes * 60.0 + seconds) * 1000.0) as i64);
    }
    let seconds: f64 = parts[1].trim().parse().ok()?;
    let hundredths: f64 = parts[2].trim().parse().ok()?;
    Some(((minutes * 60.0 + seconds + hundredths / 100.0) * 1000.0) as i64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_timed_lyric_into_cues() {
        let lines = parse_lrc("[00:17.62] All I wanna do\n[00:22.19] Rosanna, Rosanna");
        assert_eq!(
            lines,
            vec![
                (17620, "All I wanna do".to_string()),
                (22190, "Rosanna, Rosanna".to_string()),
            ]
        );
    }

    /// A lyric sheet marked up with section headings is not an LRC file.
    /// Without a real timestamp check "[Chorus]" parses as a time and the
    /// whole song collapses onto one mistimed line — worse than plain text.
    #[test]
    fn section_headings_are_not_timestamps() {
        assert!(parse_lrc("[Verse 1]\nCloser to the sun\n\n[Chorus]\nRosanna").is_empty());
    }

    #[test]
    fn plain_text_yields_no_cues() {
        assert!(parse_lrc("Closing time\nOpen all the doors").is_empty());
    }

    /// LRC's way of writing a repeated chorus once.
    #[test]
    fn several_timestamps_on_one_line_become_several_cues() {
        let lines = parse_lrc("[00:30.00][01:20.50] Rosanna");
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0], (30000, "Rosanna".to_string()));
        assert_eq!(lines[1], (80500, "Rosanna".to_string()));
    }

    /// `[ar:...]` must contribute nothing, not a line at time zero.
    #[test]
    fn metadata_headers_are_skipped() {
        let lines = parse_lrc("[ar:Toto]\n[ti:Rosanna]\n[00:17.62] All I wanna do");
        assert_eq!(lines.len(), 1);
        assert_eq!(lines[0].1, "All I wanna do");
    }

    /// Older tools write `mm:ss:xx` where the spec says `mm:ss.xx`.
    #[test]
    fn the_legacy_colon_form_parses() {
        assert_eq!(parse_lrc("[01:02:50] Line")[0].0, 62500);
    }

    #[test]
    fn cues_come_back_in_time_order_however_they_were_written() {
        let lines = parse_lrc("[00:30.00] Second\n[00:10.00] First");
        assert_eq!(lines[0].1, "First");
        assert_eq!(lines[1].1, "Second");
    }
}

/// Records a completed play of a podcast episode.
///
/// Separate from the music path because the two store progress in different
/// tables; sharing one would mean a podcast position lost the moment audio2's
/// own clients looked for it.
async fn scrobble_episode(
    auth: &SubsonicAuthUser,
    state: &AppState,
    episode_id: Uuid,
    params: &ScrobbleParams,
) -> Response {
    let episode = match db::podcasts::find_episode(state.db(), episode_id).await {
        Ok(Some(episode)) => episode,
        Ok(None) => return err(auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(auth, SubsonicErrorCode::Generic),
    };

    match db::podcasts::find_feed(state.db(), episode.feed_id, auth.viewer()).await {
        Ok(Some(_)) => {}
        Ok(None) => return err(auth, SubsonicErrorCode::NotFound),
        Err(_) => return err(auth, SubsonicErrorCode::Generic),
    }

    // `submission=false` is a "now playing" notice rather than a finished
    // play, exactly as on the music path.
    if params.submission.unwrap_or(true) {
        let duration = episode.duration_secs.unwrap_or(0) as f64;
        if db::playback::upsert_podcast_progress(state.db(), auth.user_id, episode_id, duration, true)
            .await
            .is_err()
        {
            return err(auth, SubsonicErrorCode::Generic);
        }

        let _ = db::stats::derive_from_progress(
            state.db(),
            auth.user_id,
            crate::db::access::PODCAST,
            episode_id,
            None,
            duration,
            "subsonic",
        )
        .await;
    }

    envelope::ok(auth.format, auth.jsonp_callback.as_deref(), json!({}))
}

#[cfg(test)]
mod binary_error_tests {
    use super::*;
    use crate::subsonic::envelope::ResponseFormat;

    fn auth() -> SubsonicAuthUser {
        SubsonicAuthUser {
            user_id: Uuid::new_v4(),
            family_id: Uuid::new_v4(),
            is_family_admin: false,
            username: "someone@example.com".into(),
            format: ResponseFormat::Json,
            jsonp_callback: None,
        }
    }

    /// The bug this guards: `stream` refused with HTTP 200 and an XML/JSON
    /// envelope, and the player handed that body to its audio decoder —
    /// surfacing as "the specified file type is not supported" instead of a
    /// clean failure.
    #[test]
    fn a_binary_refusal_is_not_http_200() {
        let response = binary_err(&auth(), SubsonicErrorCode::NotFound);
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn access_failures_are_forbidden_rather_than_not_found() {
        assert_eq!(
            binary_err(&auth(), SubsonicErrorCode::NotAuthorized).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            binary_err(&auth(), SubsonicErrorCode::Generic).status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    /// Endpoints that return data, not bytes, must keep returning 200 with an
    /// envelope — that is what every client parses.
    #[test]
    fn ordinary_endpoints_still_answer_200() {
        assert_eq!(err(&auth(), SubsonicErrorCode::NotFound).status(), StatusCode::OK);
    }
}
