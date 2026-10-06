// SPDX-License-Identifier: AGPL-3.0-or-later
/// Podcast channels and episodes over Subsonic.
///
/// Every piece of this already existed on `/api/v1/podcasts` — feeds, episodes,
/// refresh, subscribe, download. Nothing here is new capability; it is the same
/// data in the shape the protocol names, so a Subsonic client shows the
/// household's subscriptions instead of an empty tab.
///
/// **Downloaded is what "completed" means.** audio2 streams an episode from its
/// own storage and refuses one that has not been fetched yet, so `status` is
/// `completed` only when the episode has an audio object. Reporting anything
/// else would make a client offer play on a file the server cannot serve.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::podcasts::models::{PodcastEpisode, PodcastFeed};
use crate::subsonic::auth::SubsonicAuthUser;
use crate::subsonic::envelope::{self, SubsonicErrorCode};
use crate::subsonic::extract::SubsonicQuery;
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

/// A feed that failed its last refresh is reported as `error` so a client can
/// show why it looks stale, rather than silently presenting old episodes.
fn channel_json(feed: &PodcastFeed, episode_count: i64) -> Value {
    let mut obj = json!({
        "id": feed.id.to_string(),
        "url": feed.feed_url,
        "title": feed.title,
        "status": if feed.refresh_error.is_some() { "error" } else { "completed" },
        "episodeCount": episode_count,
    });
    if let Some(description) = &feed.description {
        obj["description"] = json!(description);
    }
    if let Some(error) = &feed.refresh_error {
        obj["errorMessage"] = json!(error);
    }
    // Cover art is addressed by the feed's own id; `getCoverArt` resolves it.
    if feed.image_object_id.is_some() {
        obj["coverArt"] = json!(feed.id.to_string());
    }
    if let Some(url) = &feed.image_url {
        obj["originalImageUrl"] = json!(url);
    }
    obj
}

pub(crate) fn episode_json(episode: &PodcastEpisode, content_type: Option<&(String, Option<i64>)>) -> Value {
    let downloaded = episode.audio_object_id.is_some();
    let mut obj = json!({
        "id": episode.id.to_string(),
        // The protocol separates the episode's identity from the id used to
        // stream it. audio2 has one row for both, so they are the same value.
        "streamId": episode.id.to_string(),
        "channelId": episode.feed_id.to_string(),
        "parent": episode.feed_id.to_string(),
        "title": episode.title,
        "isDir": false,
        "type": "podcast",
        "mediaType": "podcast",
        "status": if downloaded { "completed" } else { "new" },
    });

    if let Some(description) = &episode.description {
        obj["description"] = json!(description);
    }
    if let Some(published) = episode.published_at {
        obj["publishDate"] = json!(published.to_rfc3339());
        obj["created"] = json!(published.to_rfc3339());
    }
    if let Some(duration) = episode.duration_secs {
        obj["duration"] = json!(duration);
    }
    if let Some(track) = episode.episode_number {
        obj["track"] = json!(track);
    }
    if episode.image_object_id.is_some() {
        obj["coverArt"] = json!(episode.id.to_string());
    } else {
        // Fall back to the channel's art, which is what a listener expects to
        // see against an episode that carries none of its own.
        obj["coverArt"] = json!(episode.feed_id.to_string());
    }
    if let Some((content_type, size)) = content_type {
        obj["contentType"] = json!(content_type);
        obj["suffix"] = json!(suffix_for(content_type));
        if let Some(bytes) = size {
            obj["size"] = json!(bytes);
        }
    }
    obj
}

fn suffix_for(content_type: &str) -> &'static str {
    match content_type {
        "audio/mp4" | "audio/x-m4a" => "m4a",
        "audio/aac" => "aac",
        "audio/ogg" | "audio/opus" => "ogg",
        "audio/wav" | "audio/x-wav" => "wav",
        _ => "mp3",
    }
}

/// Media details for a batch of episodes, in one query rather than one each.
pub(crate) async fn media_for(
    pool: &sqlx::PgPool,
    episodes: &[PodcastEpisode],
) -> HashMap<Uuid, (String, Option<i64>)> {
    let object_ids: Vec<Uuid> = episodes.iter().filter_map(|e| e.audio_object_id).collect();
    let by_object = db::subsonic::media_info_for(pool, &object_ids)
        .await
        .unwrap_or_default();

    // Re-keyed by episode id so callers need not carry the object id around.
    episodes
        .iter()
        .filter_map(|e| {
            let object_id = e.audio_object_id?;
            let info = by_object.get(&object_id)?;
            Some((e.id, info.clone()))
        })
        .collect()
}

#[derive(Deserialize)]
pub struct GetPodcastsParams {
    id: Option<String>,
    #[serde(rename = "includeEpisodes")]
    include_episodes: Option<bool>,
}

/// Episodes returned per channel when a client asks for all of them.
///
/// Capped because `includeEpisodes=true` with no id means "every episode of
/// every subscription" — on this library that is several thousand rows, and a
/// client only renders the recent ones anyway.
const EPISODES_PER_CHANNEL: i64 = 200;

pub async fn get_podcasts(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<GetPodcastsParams>,
) -> Response {
    let feeds = match params.id.as_deref() {
        Some(raw) => {
            let Ok(feed_id) = raw.parse::<Uuid>() else {
                return err(&auth, SubsonicErrorCode::NotFound);
            };
            match db::podcasts::find_feed(state.db(), feed_id, auth.viewer()).await {
                Ok(Some(feed)) => vec![feed],
                Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
                Err(_) => return err(&auth, SubsonicErrorCode::Generic),
            }
        }
        None => match db::podcasts::list_feeds(state.db(), auth.viewer()).await {
            Ok(feeds) => feeds,
            Err(_) => return err(&auth, SubsonicErrorCode::Generic),
        },
    };

    let feed_ids: Vec<Uuid> = feeds.iter().map(|f| f.id).collect();
    let counts: HashMap<Uuid, i64> = db::podcasts::episode_counts(state.db(), &feed_ids)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

    // Asking for a single channel implies its episodes; a client that requests
    // one feed by id has already drilled into it.
    let include_episodes = params.include_episodes.unwrap_or(false) || params.id.is_some();

    let mut channels = Vec::with_capacity(feeds.len());
    for feed in &feeds {
        let mut channel = channel_json(feed, counts.get(&feed.id).copied().unwrap_or(0));
        if include_episodes {
            let episodes =
                match db::podcasts::list_episodes(state.db(), feed.id, EPISODES_PER_CHANNEL, 0, None).await {
                    Ok(episodes) => episodes,
                    Err(_) => return err(&auth, SubsonicErrorCode::Generic),
                };
            let media = media_for(state.db(), &episodes).await;
            channel["episode"] = json!(
                episodes
                    .iter()
                    .map(|e| episode_json(e, media.get(&e.id)))
                    .collect::<Vec<_>>()
            );
        }
        channels.push(channel);
    }

    ok(&auth, json!({ "podcasts": { "channel": channels } }))
}

#[derive(Deserialize)]
pub struct NewestPodcastsParams {
    count: Option<i64>,
}

pub async fn get_newest_podcasts(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<NewestPodcastsParams>,
) -> Response {
    let count = params.count.unwrap_or(20).clamp(1, 500);
    let episodes = match db::podcasts::list_recent_episodes(state.db(), auth.viewer(), count).await {
        Ok(episodes) => episodes,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };
    let media = media_for(state.db(), &episodes).await;

    ok(
        &auth,
        json!({
            "newestPodcasts": {
                "episode": episodes
                    .iter()
                    .map(|e| episode_json(e, media.get(&e.id)))
                    .collect::<Vec<_>>()
            }
        }),
    )
}

#[derive(Deserialize)]
pub struct EpisodeIdParams {
    id: String,
}

/// Resolves an episode id and confirms the viewer may see the feed it belongs
/// to. Episodes carry no owner of their own, so the feed is the access check.
async fn visible_episode(
    auth: &SubsonicAuthUser,
    state: &AppState,
    id: &str,
) -> Result<Option<(PodcastFeed, PodcastEpisode)>, ()> {
    let Ok(episode_id) = id.parse::<Uuid>() else {
        return Ok(None);
    };
    let Some(episode) = db::podcasts::find_episode(state.db(), episode_id)
        .await
        .map_err(|_| ())?
    else {
        return Ok(None);
    };
    let Some(feed) = db::podcasts::find_feed(state.db(), episode.feed_id, auth.viewer())
        .await
        .map_err(|_| ())?
    else {
        return Ok(None);
    };
    Ok(Some((feed, episode)))
}

/// Fetches and stores the episode, then reports success.
///
/// The protocol treats this as "start a download", and a client polls `status`
/// afterwards. audio2 fetches inline — the same thing the REST route does — so
/// by the time this answers, `status` is already `completed`.
pub async fn download_podcast_episode(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<EpisodeIdParams>,
) -> Response {
    let (feed, episode) = match visible_episode(&auth, &state, &params.id).await {
        Ok(Some(pair)) => pair,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(()) => return err(&auth, SubsonicErrorCode::Generic),
    };

    match crate::podcasts::fetch_and_store_episode_audio(state.db(), state.storage(), auth.family_id, &feed, &episode).await
    {
        Ok(()) => ok(&auth, json!({})),
        Err(error) => {
            tracing::warn!(episode = %episode.id, %error, "subsonic episode download failed");
            err(&auth, SubsonicErrorCode::Generic)
        }
    }
}

/// Moves the stored copy to the trash, returning the episode to `new`.
///
/// Deliberately not a row delete: the episode still exists in the feed, and
/// removing it would only see it reappear on the next refresh. This is the
/// "I'm done with this one" action a client's delete button actually means.
///
/// Same rule as the REST `DELETE …/download` (docs/file-sync-plan.md §5.1):
/// the copy waits 30 days in the trash, and only the show's subscriber or a
/// family admin may delete it. An episode that is not stored is a no-op.
pub async fn delete_podcast_episode(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<EpisodeIdParams>,
) -> Response {
    let (_, episode) = match visible_episode(&auth, &state, &params.id).await {
        Ok(Some(pair)) => pair,
        Ok(None) => return err(&auth, SubsonicErrorCode::NotFound),
        Err(()) => return err(&auth, SubsonicErrorCode::Generic),
    };
    if episode.audio_object_id.is_none() {
        return ok(&auth, json!({}));
    }

    match crate::trash::move_to_trash(&state, auth.viewer(), None, crate::db::trash::Kind::PodcastEpisode, episode.id).await {
        Ok(()) => ok(&auth, json!({})),
        Err(AuthError::Forbidden) => err(&auth, SubsonicErrorCode::NotAuthorized),
        Err(AuthError::ItemNotFound) => err(&auth, SubsonicErrorCode::NotFound),
        Err(_) => err(&auth, SubsonicErrorCode::Generic),
    }
}

#[derive(Deserialize)]
pub struct CreateChannelParams {
    url: String,
}

pub async fn create_podcast_channel(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<CreateChannelParams>,
) -> Response {
    let url = params.url.trim();
    if url.is_empty() {
        return err(&auth, SubsonicErrorCode::MissingParam);
    }

    // Subscribed privately: the protocol has no notion of family sharing, so
    // a feed added from a Subsonic client belongs to whoever added it and can
    // be shared afterwards from audio2's own UI.
    match crate::podcasts::subscribe_rss_feed(&state, auth.user_id, None, url, auth.viewer()).await {
        Ok(_) => ok(&auth, json!({})),
        Err(error) => {
            tracing::warn!(%url, %error, "subsonic podcast subscribe failed");
            err(&auth, SubsonicErrorCode::Generic)
        }
    }
}

#[derive(Deserialize)]
pub struct ChannelIdParams {
    id: String,
}

pub async fn delete_podcast_channel(
    auth: SubsonicAuthUser,
    State(state): State<AppState>,
    SubsonicQuery(params): SubsonicQuery<ChannelIdParams>,
) -> Response {
    let Ok(feed_id) = params.id.parse::<Uuid>() else {
        return err(&auth, SubsonicErrorCode::NotFound);
    };

    // Owner-scoped: unsubscribing is a mutation, and a family member who can
    // merely see a shared feed must not be able to remove it.
    match db::podcasts::delete_feed(state.db(), feed_id, auth.user_id).await {
        Ok(()) => ok(&auth, json!({})),
        Err(_) => err(&auth, SubsonicErrorCode::Generic),
    }
}

/// Queues a refresh for every visible feed.
///
/// The protocol's `refreshPodcasts` is fire-and-forget, and refreshing several
/// feeds inline would hold the request open for as long as the slowest one. The
/// existing job worker already refreshes feeds on a schedule, so this enqueues
/// rather than duplicating that logic.
pub async fn refresh_podcasts(auth: SubsonicAuthUser, State(state): State<AppState>) -> Response {
    let feeds = match db::podcasts::list_feeds(state.db(), auth.viewer()).await {
        Ok(feeds) => feeds,
        Err(_) => return err(&auth, SubsonicErrorCode::Generic),
    };

    for feed in &feeds {
        let _ = db::jobs::enqueue(
            state.db(),
            "feed_refresh",
            Some(&json!({ "feed_id": feed.id.to_string() })),
            None,
        )
        .await;
    }

    ok(&auth, json!({}))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn episode(downloaded: bool, own_art: bool) -> PodcastEpisode {
        PodcastEpisode {
            id: Uuid::new_v4(),
            feed_id: Uuid::new_v4(),
            guid: "guid".into(),
            title: "An episode".into(),
            description: None,
            published_at: Some(Utc::now()),
            duration_secs: Some(1800),
            episode_number: None,
            season_number: None,
            audio_url: Some("https://example.com/ep.mp3".into()),
            audio_object_id: downloaded.then(Uuid::new_v4),
            image_url: None,
            image_object_id: own_art.then(Uuid::new_v4),
            created_at: Utc::now(),
            size_bytes: None,
            sha256: None,
            transcript_url: None,
            transcript_type: None,
        }
    }

    /// audio2 streams only what it has stored, so "completed" has to mean
    /// downloaded. Claiming otherwise would make a client offer play on a file
    /// the server then refuses to serve.
    #[test]
    fn status_reflects_whether_the_audio_is_actually_stored() {
        assert_eq!(episode_json(&episode(true, false), None)["status"], "completed");
        assert_eq!(episode_json(&episode(false, false), None)["status"], "new");
    }

    /// Amperfy reads `publishDate` only when it is at least 19 characters, so
    /// a shorter date format would be silently dropped.
    #[test]
    fn publish_date_is_long_enough_for_clients_to_accept() {
        let json = episode_json(&episode(true, false), None);
        assert!(json["publishDate"].as_str().unwrap().len() >= 19);
    }

    /// An episode with no art of its own shows the channel's, rather than an
    /// empty frame.
    #[test]
    fn an_episode_without_art_falls_back_to_its_channel() {
        let ep = episode(true, false);
        let json = episode_json(&ep, None);
        assert_eq!(json["coverArt"], ep.feed_id.to_string());

        let with_art = episode(true, true);
        let json = episode_json(&with_art, None);
        assert_eq!(json["coverArt"], with_art.id.to_string());
    }

    /// The protocol distinguishes an episode's identity from the id used to
    /// stream it; audio2 has one row for both, and clients use `streamId`.
    #[test]
    fn stream_id_matches_the_episode_id() {
        let ep = episode(true, false);
        let json = episode_json(&ep, None);
        assert_eq!(json["streamId"], ep.id.to_string());
        assert_eq!(json["id"], ep.id.to_string());
    }

    #[test]
    fn media_details_appear_only_once_the_episode_is_stored() {
        let bare = episode_json(&episode(false, false), None);
        assert!(bare.get("size").is_none());
        assert!(bare.get("contentType").is_none());

        let stored = episode_json(
            &episode(true, false),
            Some(&("audio/mpeg".to_string(), Some(3_677_940))),
        );
        assert_eq!(stored["size"], 3_677_940);
        assert_eq!(stored["suffix"], "mp3");
    }
}
