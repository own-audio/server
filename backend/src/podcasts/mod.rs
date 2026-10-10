// SPDX-License-Identifier: AGPL-3.0-or-later
/// Podcasts module — feeds, episodes, ingest, refresh jobs, playback metadata.
pub mod models;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::db::access;
use crate::families::FamilyContext;
use crate::storage::ObjectStore;
use crate::youtube;
use axum::extract::{Json, Path, Query, State};
use axum::http::{StatusCode, header};
use axum::response::IntoResponse;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::{router::OpenApiRouter, routes};
use sha2::{Digest, Sha256};
use tracing::info;
use uuid::Uuid;


// Outbound fetches (feeds, artwork, enclosures) go through `http::outbound`:
// a real user agent, bounded stalls, public addresses only, size caps.

/// Feeds and artwork are small; anything past this is not a feed.
const MAX_FEED_BYTES: u64 = 20 * 1024 * 1024;

/// The most an episode may be to be stored on the server
/// (`PODCASTS__MAX_EPISODE_BYTES`, default 512 MiB). Past it, the episode is
/// still playable from the publisher's URL.
static MAX_EPISODE_BYTES: std::sync::LazyLock<u64> = std::sync::LazyLock::new(|| {
    std::env::var("PODCASTS__MAX_EPISODE_BYTES")
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(512 * 1024 * 1024)
});

// ── DTOs ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct SubscribeRequest {
    pub feed_url: String,
    /// `private` (default) or `family`.
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct PodcastSearchRequest {
    pub q: String,
    /// Language subtag (`en`, not `en-US`). Optional, and unset means no
    /// filter.
    ///
    /// Deliberately not defaulted to the household's own languages, which is
    /// what this was first sketched as. The catalogue's search takes one
    /// language, households are not reliably monolingual, and a guessed filter
    /// hides results with no visible explanation — the user sees an empty
    /// search box, not a filter they can turn off. The client knows whether it
    /// has a "search all languages" affordance to offer; the server does not,
    /// so the server does not guess.
    #[serde(default)]
    pub language: Option<String>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct EpisodeListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    /// Only episodes whose title or description holds this (any case, any accents).
    #[serde(default)]
    pub q: Option<String>,
}

fn default_limit() -> i64 {
    50
}

#[derive(Serialize, ToSchema)]
pub struct FeedResponse {
    pub id: String,
    pub feed_url: String,
    pub source_type: String,
    pub title: String,
    pub description: Option<String>,
    pub author: Option<String>,
    pub link: Option<String>,
    pub language: Option<String>,
    pub image_url: Option<String>,
    pub last_refreshed_at: Option<String>,
    /// `private` or `family` — which folder this feed lives in.
    pub visibility: String,
    /// False for family-shared feeds owned by someone else.
    pub is_owner: bool,
    /// Whose it is — for acting on a family member's item (a family shortcut).
    pub owner_id: String,
    /// Podcast Index categories, lowercased, or the feed's own iTunes
    /// categories when the catalogue does not know it. Empty is normal — a
    /// feed subscribed while the metadata service was unreachable has none
    /// until the backfill job reaches it, and clients must render fine
    /// without them.
    pub categories: Vec<String>,
    /// `language` folded to its subtag. Clients grouping or filtering by
    /// language must use this, not `language`, which is whatever the
    /// publisher wrote.
    pub language_base: Option<String>,
    /// The server stores each new episode by itself, so the family keeps it
    /// even if the feed later drops it (docs/file-sync-plan.md §5.6).
    pub auto_store: bool,
    /// At least one episode publishes a transcript, so it can be translated.
    /// Filled in by the list and the single-feed read only; false elsewhere.
    pub has_transcripts: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetAutoStoreRequest {
    pub enabled: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct SetVisibilityRequest {
    /// `private` or `family`.
    pub visibility: String,
}

#[derive(Serialize, ToSchema)]
pub struct EpisodeResponse {
    pub id: String,
    pub feed_id: String,
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<String>,
    pub duration_secs: Option<i32>,
    pub episode_number: Option<i32>,
    pub audio_url: Option<String>,
    /// Resolved image URL: S3 proxy path when downloaded, else raw RSS URL.
    pub image_url: Option<String>,
    /// True when the audio file has been downloaded into the object store.
    pub has_local: bool,
    /// Saved playback position in seconds (null = never played).
    pub progress_secs: Option<f64>,
    /// True when the user has listened to the whole episode.
    pub completed: bool,
    /// From `media_objects`, `null` when `has_local` is false or the checksum job hasn't
    /// caught up yet (mirror-plan B-2).
    pub size_bytes: Option<i64>,
    pub sha256: Option<String>,
    /// True when the feed published a `<podcast:transcript>` for this episode — the gate for
    /// docs/podcast-translation-plan.md's translation feature (no speech-to-text, ever; a
    /// missing transcript means the feature simply isn't offered for this episode).
    pub has_transcript: bool,
}

#[derive(Serialize, ToSchema)]
pub struct StreamResponse {
    pub url: String,
    pub expires_in_secs: u64,
}

#[derive(Serialize, ToSchema)]
pub struct PodcastSearchResult {
    pub title: String,
    pub feed_url: String,
    pub link: Option<String>,
    pub description: Option<String>,
    pub author: Option<String>,
    pub image_url: Option<String>,
    pub language: Option<String>,
    pub episode_count: Option<i32>,
    /// Added 2026-08-29. Additive on the wire — older clients ignore it.
    #[serde(default)]
    pub categories: Vec<String>,
}


// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_feeds))
        .routes(routes!(search_podcasts))
        .routes(routes!(discover_categories))
        .routes(routes!(discover_browse))
        .routes(routes!(similar_feeds))
        .routes(routes!(unplayed_counts))
        .routes(routes!(discover_similar))
        .routes(routes!(discover_preview_episodes))
        .routes(routes!(subscribe))
        .routes(routes!(get_feed, unsubscribe))
        .routes(routes!(get_feed_image))
        .routes(routes!(list_episodes))
        .routes(routes!(refresh_feed))
        .routes(routes!(sync_images))
        .routes(routes!(set_feed_visibility))
        .routes(routes!(set_auto_store))
        .routes(routes!(store_all))
        .routes(routes!(download_episode, delete_episode_download))
        .routes(routes!(stream_episode))
        .routes(routes!(get_episode_image))
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/podcasts/
///
/// The feeds the caller can see: their own and those shared with the family.
#[utoipa::path(get, path = "/", tag = "podcasts", security(("bearer" = [])),
    responses((status = 200, body = Vec<FeedResponse>)))]
async fn list_feeds(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<FeedResponse>>, AuthError> {
    let feeds = db::podcasts::list_feeds(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    let ids: Vec<Uuid> = feeds.iter().map(|f| f.id).collect();
    let with_transcripts = db::podcasts::feeds_with_transcripts(state.db(), &ids)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        feeds
            .into_iter()
            .map(|f| {
                let has = with_transcripts.contains(&f.id);
                FeedResponse { has_transcripts: has, ..feed_to_response(f, family.user_id) }
            })
            .collect(),
    ))
}

/// POST /api/v1/podcasts/search
///
/// Answered from the self-hosted Podcast Index catalogue. Until 2026-08-29
/// this proxied `apollo.rss.com`, which meant every search anyone typed left
/// the machine — a privacy cost the product's own copy leaves no room for, and
/// a dependency that returned 500s for a spell when the upstream started
/// rejecting requests with no User-Agent.
///
/// **There is deliberately no fallback to that host.** A fallback fires
/// precisely when the metadata service is down and nobody is watching, which
/// is the worst possible moment to quietly resume shipping search terms to a
/// third party. An unreachable catalogue is an error.
#[utoipa::path(post, path = "/search", tag = "podcasts", security(("bearer" = [])),
    request_body = PodcastSearchRequest,
    responses((status = 200, body = Vec<PodcastSearchResult>), (status = 400, description = "Query shorter than 2 characters", body = crate::http::openapi::ErrorBody), (status = 500, description = "Metadata service not configured or unreachable", body = crate::http::openapi::ErrorBody)))]
async fn search_podcasts(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<PodcastSearchRequest>,
) -> Result<Json<Vec<PodcastSearchResult>>, AuthError> {
    let query = body.q.trim();
    if query.len() < 2 {
        return Err(AuthError::BadRequest("search query must be at least 2 characters".to_string()));
    }

    let Some(config) = state.config().metadata.as_ref() else {
        // Without the catalogue: Apple's public search, unless switched off.
        if !state.config().itunes.enabled {
            return Err(AuthError::Internal(anyhow::anyhow!(
                "podcast search is off: no metadata service (METADATA__BASE_URL) and ITUNES__ENABLED=false"
            )));
        }
        let shows = crate::metadata::itunes::search(query, 25).await.map_err(AuthError::Internal)?;
        return Ok(Json(
            shows
                .into_iter()
                .map(|s| PodcastSearchResult {
                    title: s.title,
                    feed_url: s.feed_url,
                    link: s.link,
                    description: None,
                    author: s.author,
                    image_url: s.image_url,
                    language: None,
                    episode_count: s.episode_count,
                    categories: s.categories,
                })
                .collect(),
        ));
    };

    let languages = discovery_languages(&state, family.user_id, body.language.as_deref()).await?;

    let mirror = crate::metadata::mirror::MetadataMirror::new(config);
    let mut entries = mirror
        .podcast_search(query, language_param(&languages).as_deref(), 25)
        .await
        .map_err(AuthError::Internal)?;

    // A catalogue that predates 2026-08-31 takes one language, not a list: it reads `cs,en` as
    // one language nobody publishes in and answers nothing at all — which on a client looks
    // exactly like a broken search, and did (a listener with four languages set got an empty
    // screen for every query). Asking one language at a time is what such a catalogue can
    // answer. Only on the empty path, so a new catalogue never pays for it; delete this once
    // every deployment this talks to understands lists.
    if entries.is_empty() && languages.len() > 1 {
        let mut per_language = Vec::new();
        for language in &languages {
            per_language.push(
                mirror
                    .podcast_search(query, Some(language.as_str()), 25)
                    .await
                    .map_err(AuthError::Internal)?,
            );
        }
        entries = interleave_by_language(per_language, 25);
    }

    let results = entries
        .into_iter()
        .map(|entry| PodcastSearchResult {
            title: entry.title,
            feed_url: entry.url,
            link: entry.link,
            description: entry.description,
            author: entry.author,
            image_url: entry.image_url,
            language: entry.language,
            // Zero means "the catalogue does not know", which is what the old
            // upstream expressed as a missing field. Keep the wire shape.
            episode_count: (entry.episode_count > 0).then_some(entry.episode_count),
            categories: entry.categories,
        })
        .collect();

    Ok(Json(results))
}

/// Which languages this caller's discovery answers in.
///
/// The user's own list (migration 0063) unless the request names one, which a
/// client does when the listener picks a language on the screen itself — a
/// deliberate, momentary override of a standing preference. Empty means every
/// language, which is what search and browse did before the setting existed.
///
/// **This is the one step that needs to know who asked, and it happens here,
/// in the household's own backend.** The catalogue is told which languages to
/// answer in and never which user wanted them — the same split as filtering
/// out already-subscribed feeds; see docs/podcast-recommendations-plan.md §0.
async fn discovery_languages(
    state: &AppState,
    user_id: Uuid,
    requested: Option<&str>,
) -> Result<Vec<String>, AuthError> {
    if let Some(language) = requested.and_then(base_language) {
        return Ok(vec![language]);
    }

    Ok(crate::db::users::find_by_id(state.db(), user_id)
        .await
        .map_err(AuthError::Internal)?
        .map(|user| user.discovery_languages)
        .unwrap_or_default())
}

/// The catalogue takes them as one comma-separated `language` parameter;
/// `None` there means no filter at all.
fn language_param(languages: &[String]) -> Option<String> {
    (!languages.is_empty()).then(|| languages.join(","))
}


/// Merges per-language result lists into one, round-robin, deduplicated by catalogue id.
///
/// Round-robin rather than concatenated: a listener who lists four languages wants a screen with
/// all four on it, not twenty-five results in the first one and none of the rest.
fn interleave_by_language(
    per_language: Vec<Vec<crate::metadata::PodcastCatalogEntry>>,
    limit: usize,
) -> Vec<crate::metadata::PodcastCatalogEntry> {
    let mut seen: std::collections::HashSet<i64> = std::collections::HashSet::new();
    let mut merged = Vec::new();
    let longest = per_language.iter().map(Vec::len).max().unwrap_or(0);

    for index in 0..longest {
        for list in &per_language {
            let Some(entry) = list.get(index) else { continue };
            if seen.insert(entry.id) {
                merged.push(entry.clone());
                if merged.len() == limit {
                    return merged;
                }
            }
        }
    }
    merged
}

/// Shared shape for everything that returns catalogue feeds, so a client can
/// render a search result, a browse result and a recommendation with one view.
fn catalog_entry_to_result(entry: crate::metadata::PodcastCatalogEntry) -> PodcastSearchResult {
    PodcastSearchResult {
        title: entry.title,
        feed_url: entry.url,
        link: entry.link,
        description: entry.description,
        author: entry.author,
        image_url: entry.image_url,
        language: entry.language,
        episode_count: (entry.episode_count > 0).then_some(entry.episode_count),
        categories: entry.categories,
    }
}

fn metadata_config(state: &AppState) -> Result<&crate::app::config::MetadataConfig, AuthError> {
    state.config().metadata.as_ref().ok_or_else(|| {
        AuthError::Internal(anyhow::anyhow!(
            "podcast discovery needs the metadata service (METADATA__BASE_URL / METADATA__API_KEY)"
        ))
    })
}

/// GET /api/v1/podcasts/{id}/similar — shows like this one.
///
/// The catalogue answers "what is like this feed" knowing nothing about who
/// asked. **The one step that needs to know what the household already has —
/// filtering those out — happens here, in the household's own backend, and
/// never leaves it.** That split is the whole privacy design; see
/// docs/podcast-recommendations-plan.md §0.
///
/// A feed with no `catalog_id` returns an empty list rather than an error:
/// the catalogue is a weekly snapshot and simply may not know a new show.
#[utoipa::path(get, path = "/{id}/similar", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    responses((status = 200, description = "Empty when the catalogue does not know this feed", body = Vec<PodcastSearchResult>), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody), (status = 500, description = "Metadata service not configured or unreachable", body = crate::http::openapi::ErrorBody)))]
async fn similar_feeds(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<PodcastSearchResult>>, AuthError> {
    let feed = db::podcasts::find_feed(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let Some(catalog_id) = feed.catalog_id else {
        return Ok(Json(Vec::new()));
    };

    similar_to_catalog_id(&state, &family, catalog_id).await.map(Json)
}

#[derive(Serialize, ToSchema)]
pub struct UnplayedCount {
    pub feed_id: String,
    pub unplayed: i64,
}

/// GET /api/v1/podcasts/unplayed-counts
///
/// One number per show: how much of it is still waiting. Per listener, which is why it is not
/// folded into `GET /podcasts` — that list is the household's, this is yours.
#[utoipa::path(get, path = "/unplayed-counts", tag = "podcasts", security(("bearer" = [])),
    responses((status = 200, body = Vec<UnplayedCount>)))]
async fn unplayed_counts(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<UnplayedCount>>, AuthError> {
    let rows = db::podcasts::unplayed_counts(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        rows.into_iter()
            .map(|(feed_id, unplayed)| UnplayedCount {
                feed_id: feed_id.to_string(),
                unplayed,
            })
            .collect(),
    ))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct FeedUrlQuery {
    pub feed_url: String,
}

/// GET /api/v1/podcasts/discover/similar?feed_url=…
///
/// The same answer as `/{id}/similar`, for a show nobody here follows yet —
/// which is exactly when "more like this" is worth asking. The feed is looked
/// up in the catalogue by its URL, since a show that is not subscribed has no
/// id of ours to name it by.
#[utoipa::path(get, path = "/discover/similar", tag = "podcasts", security(("bearer" = [])),
    params(FeedUrlQuery),
    responses((status = 200, description = "Empty when the catalogue does not know this feed", body = Vec<PodcastSearchResult>), (status = 400, description = "No feed_url", body = crate::http::openapi::ErrorBody), (status = 500, description = "Metadata service not configured or unreachable", body = crate::http::openapi::ErrorBody)))]
async fn discover_similar(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<FeedUrlQuery>,
) -> Result<Json<Vec<PodcastSearchResult>>, AuthError> {
    let feed_url = params.feed_url.trim();
    if feed_url.is_empty() {
        return Err(AuthError::BadRequest("feed_url is required".to_string()));
    }

    let config = metadata_config(&state)?;
    let mirror = crate::metadata::mirror::MetadataMirror::new(config);
    let Some(entry) = mirror
        .podcast_lookup(Some(feed_url), None, None)
        .await
        .map_err(AuthError::Internal)?
    else {
        // The catalogue is a weekly snapshot and may simply not know this show.
        return Ok(Json(Vec::new()));
    };

    similar_to_catalog_id(&state, &family, entry.id).await.map(Json)
}

/// The half of "shows like this one" that must stay in the household's own
/// backend: what it already follows is filtered here, never sent out.
async fn similar_to_catalog_id(
    state: &AppState,
    family: &FamilyContext,
    catalog_id: i64,
) -> Result<Vec<PodcastSearchResult>, AuthError> {
    let config = metadata_config(state)?;
    // With no languages set the catalogue stays locked to the seed feed's own
    // language, which is the right answer for "more like this". With them set,
    // they replace the lock: someone who reads Czech and English should be
    // offered both even when the show in hand is in neither.
    let languages = discovery_languages(state, family.user_id, None).await?;
    let candidates = crate::metadata::mirror::MetadataMirror::new(config)
        .podcast_similar(catalog_id, language_param(&languages).as_deref(), 20)
        .await
        .map_err(AuthError::Internal)?;

    let known = db::podcasts::subscribed_catalog_ids(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    Ok(candidates
        .into_iter()
        .filter(|entry| !known.contains(&entry.id))
        .map(catalog_entry_to_result)
        .collect())
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PreviewQuery {
    pub feed_url: String,
    #[serde(default = "default_preview_limit")]
    pub limit: usize,
}

fn default_preview_limit() -> usize {
    12
}

#[derive(Serialize, ToSchema)]
pub struct PreviewEpisode {
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i32>,
}

/// GET /api/v1/podcasts/discover/preview?feed_url=…&limit=12
///
/// The newest episodes of a show, for deciding whether to follow it. The feed
/// is fetched and parsed on the spot and **nothing is stored** — subscribing is
/// still what puts a show in the library.
///
/// The work is the same as subscribing minus the writes, so a very large feed
/// takes as long here as it does there; callers need a waiting state.
#[utoipa::path(get, path = "/discover/preview", tag = "podcasts", security(("bearer" = [])),
    params(PreviewQuery),
    responses((status = 200, description = "Newest first; `limit` is clamped to 1–50", body = Vec<PreviewEpisode>), (status = 400, description = "No feed_url", body = crate::http::openapi::ErrorBody), (status = 404, description = "The feed could not be fetched or parsed", body = crate::http::openapi::ErrorBody)))]
async fn discover_preview_episodes(
    _family: FamilyContext,
    Query(params): Query<PreviewQuery>,
) -> Result<Json<Vec<PreviewEpisode>>, AuthError> {
    let feed_url = params.feed_url.trim();
    if feed_url.is_empty() {
        return Err(AuthError::BadRequest("feed_url is required".to_string()));
    }

    let channel = fetch_rss_url(feed_url).await.map_err(|e| {
        if crate::http::outbound::is_refused(&e) { AuthError::BadRequest(e.to_string()) } else { AuthError::ItemNotFound }
    })?;
    Ok(Json(newest_episodes(&channel, params.limit)))
}

/// The newest `limit` items of a feed.
///
/// Sorted here rather than trusted: most feeds are newest-first, enough are not,
/// and an undated item sorts last instead of pretending to be current.
fn newest_episodes(channel: &rss::Channel, limit: usize) -> Vec<PreviewEpisode> {
    let mut episodes: Vec<PreviewEpisode> = channel
        .items
        .iter()
        .map(|item| PreviewEpisode {
            guid: item
                .guid()
                .map(|g| g.value().to_string())
                .or_else(|| item.link().map(str::to_string))
                .unwrap_or_else(|| item.title().unwrap_or_default().to_string()),
            title: item.title().unwrap_or("Untitled").to_string(),
            description: item.description().map(str::to_string),
            published_at: item
                .pub_date()
                .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
                .map(|d| d.with_timezone(&Utc)),
            duration_secs: item
                .itunes_ext()
                .and_then(|e| e.duration())
                .and_then(parse_duration),
        })
        .collect();

    episodes.sort_by(|a, b| match (a.published_at, b.published_at) {
        (Some(left), Some(right)) => right.cmp(&left),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    episodes.truncate(limit.clamp(1, 50));
    episodes
}

/// GET /api/v1/podcasts/discover/categories
///
/// The catalogue's categories, with how many feeds sit in each.
#[utoipa::path(get, path = "/discover/categories", tag = "podcasts", security(("bearer" = [])),
    responses((status = 200, body = Vec<crate::metadata::PodcastCategoryCount>), (status = 500, description = "Metadata service not configured or unreachable", body = crate::http::openapi::ErrorBody)))]
async fn discover_categories(
    _family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<crate::metadata::PodcastCategoryCount>>, AuthError> {
    let config = metadata_config(&state)?;
    crate::metadata::mirror::MetadataMirror::new(config)
        .podcast_categories()
        .await
        .map(Json)
        .map_err(AuthError::Internal)
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BrowseQuery {
    pub category: String,
    pub language: Option<String>,
    #[serde(default)]
    pub offset: u32,
}

/// GET /api/v1/podcasts/discover/browse?category=…
///
/// Feeds the household already follows are filtered out here for the same
/// reason as in `similar_feeds` — and because a discovery screen that keeps
/// offering you what you already subscribe to reads as broken.
#[utoipa::path(get, path = "/discover/browse", tag = "podcasts", security(("bearer" = [])),
    params(BrowseQuery),
    responses((status = 200, description = "Up to 25 feeds, without those the household already follows", body = Vec<PodcastSearchResult>), (status = 400, description = "No category", body = crate::http::openapi::ErrorBody), (status = 500, description = "Metadata service not configured or unreachable", body = crate::http::openapi::ErrorBody)))]
async fn discover_browse(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(query): Query<BrowseQuery>,
) -> Result<Json<Vec<PodcastSearchResult>>, AuthError> {
    if query.category.trim().is_empty() {
        return Err(AuthError::BadRequest("category is required".to_string()));
    }

    let config = metadata_config(&state)?;
    let languages = discovery_languages(&state, family.user_id, query.language.as_deref()).await?;

    let mirror = crate::metadata::mirror::MetadataMirror::new(config);
    let mut candidates = mirror
        .podcast_browse(
            query.category.trim(),
            language_param(&languages).as_deref(),
            25,
            query.offset,
        )
        .await
        .map_err(AuthError::Internal)?;

    // Same one-language-at-a-time fallback as `search_podcasts`, and for the same catalogue.
    if candidates.is_empty() && languages.len() > 1 {
        let mut per_language = Vec::new();
        for language in &languages {
            per_language.push(
                mirror
                    .podcast_browse(query.category.trim(), Some(language.as_str()), 25, query.offset)
                    .await
                    .map_err(AuthError::Internal)?,
            );
        }
        candidates = interleave_by_language(per_language, 25);
    }

    let known = db::podcasts::subscribed_catalog_ids(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(
        candidates
            .into_iter()
            .filter(|entry| !known.contains(&entry.id))
            .map(catalog_entry_to_result)
            .collect(),
    ))
}

/// POST /api/v1/podcasts/subscribe
///
/// Subscribes to an RSS feed or a YouTube channel URL; an existing subscription is returned as-is.
#[utoipa::path(post, path = "/subscribe", tag = "podcasts", security(("bearer" = [])),
    request_body = SubscribeRequest,
    responses((status = 201, description = "Subscribed", body = FeedResponse), (status = 200, description = "Already subscribed: the existing feed", body = FeedResponse), (status = 400, description = "Bad visibility", body = crate::http::openapi::ErrorBody), (status = 500, description = "The feed or channel could not be fetched", body = crate::http::openapi::ErrorBody)))]
async fn subscribe(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<SubscribeRequest>,
) -> Result<(StatusCode, Json<FeedResponse>), AuthError> {
    if youtube::looks_like_channel_url(&body.feed_url) {
        return subscribe_youtube(family, state, body).await;
    }

    let feed_family_id =
        access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let (feed, created) = subscribe_rss_feed(
        &state,
        family.user_id,
        feed_family_id,
        &body.feed_url,
        family.viewer(),
    )
    .await
    .map_err(fetch_failure)?;

    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(feed_to_response(feed, family.user_id))))
}

/// Subscribes to an RSS podcast: fetch, dedupe, insert, ingest episodes.
///
/// Extracted so Subsonic's `createPodcastChannel` performs exactly the same
/// work as the REST route rather than a second implementation — the duplicated
/// column list in `db/subsonic.rs` is the cautionary tale.
///
/// The `bool` is whether a new feed was created; an existing subscription is
/// returned as-is rather than duplicated.
/// The language subtag of a BCP-47 tag: `en-US` and `en-us` both give `en`.
///
/// Exists because the wild carries 112 spellings of English, so filtering on
/// the raw `language` column matches a fraction of what it should. Mirrors
/// `podcastindex.base_language` on the catalogue side; the two must agree or a
/// feed enriched here will not match a catalogue query there.
pub(crate) fn base_language(tag: &str) -> Option<String> {
    let base = tag.trim().to_ascii_lowercase();
    let base = base.split('-').next().unwrap_or("").trim();
    (!base.is_empty()).then(|| base.to_string())
}

/// The feed's own iTunes categories, in the catalogue's vocabulary.
///
/// The fallback for a feed the catalogue does not know — which is normal for
/// anything published in the last week, since the catalogue is a weekly
/// snapshot. Subcategories are flattened in alongside their parents rather
/// than dropped: "Society & Culture > Personal Journals" is two useful facts.
///
/// **The `&` split is not cosmetic.** Podcast Index stores the iTunes taxonomy
/// already split — "Society & Culture" is `society` + `culture`, "Health &
/// Fitness" is `health` + `fitness` — while RSS carries the joined label. Keep
/// the joined form and a feed enriched from its own RSS can never match one
/// enriched from the catalogue, so "more like this" quietly returns nothing
/// across the boundary. Caught by the P2 end-to-end test, which stored
/// `society & culture` for a feed whose catalogue entry says `society`.
fn rss_categories(channel: &rss::Channel) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let Some(ext) = channel.itunes_ext() else {
        return out;
    };

    for category in ext.categories() {
        for text in [Some(category.text()), category.subcategory().map(|s| s.text())]
            .into_iter()
            .flatten()
        {
            for part in text.split('&') {
                let value = part.trim().to_ascii_lowercase();
                if !value.is_empty() && !out.contains(&value) {
                    out.push(value);
                }
            }
        }
    }
    out
}

/// Ask the Podcast Index catalogue about a feed and store what it says.
///
/// **Best-effort by design, exactly like the waitlist's confirmation email.**
/// Every failure path here is swallowed to a warning: an unreachable metadata
/// service, an unconfigured one, a feed the catalogue has never seen. None of
/// them is a reason to fail a subscribe the user asked for, and a feed with no
/// categories is a working feed — it simply has no "more like this" until the
/// backfill job reaches it.
async fn enrich_from_catalog(
    state: &AppState,
    feed_id: Uuid,
    feed_url: &str,
    channel: &rss::Channel,
) {
    // The feed's own categories are the floor. If the catalogue answers, its
    // categories replace these; if it does not, these are what the feed gets,
    // which is the difference between "no recommendations for new shows" and
    // "recommendations for every show that bothered to categorize itself".
    let fallback = rss_categories(channel);
    let language_base = channel.language.as_deref().and_then(base_language);

    let entry = match state.config().metadata.as_ref() {
        Some(config) => {
            match crate::metadata::mirror::MetadataMirror::new(config)
                .podcast_lookup(Some(feed_url), None, None)
                .await
            {
                Ok(entry) => entry,
                Err(err) => {
                    tracing::warn!(%err, %feed_url, "podcast catalogue lookup failed");
                    None
                }
            }
        }
        None => None,
    };

    let result = match entry {
        Some(entry) => {
            let categories = if entry.categories.is_empty() {
                fallback
            } else {
                entry.categories
            };
            db::podcasts::set_catalog_metadata(
                state.db(),
                feed_id,
                Some(entry.id),
                entry.podcast_guid.as_deref(),
                entry.itunes_id,
                &categories,
                Some(entry.popularity_score),
                entry.language_base.as_deref().or(language_base.as_deref()),
            )
            .await
        }
        None => {
            db::podcasts::set_catalog_metadata(
                state.db(),
                feed_id,
                None,
                None,
                None,
                &fallback,
                None,
                language_base.as_deref(),
            )
            .await
        }
    };

    if let Err(err) = result {
        tracing::warn!(%err, %feed_url, "storing podcast catalogue metadata failed");
    }
}

pub(crate) async fn subscribe_rss_feed(
    state: &AppState,
    user_id: Uuid,
    feed_family_id: Option<Uuid>,
    feed_url: &str,
    viewer: access::Viewer,
) -> anyhow::Result<(crate::podcasts::models::PodcastFeed, bool)> {
    let channel = fetch_rss_url(feed_url).await?;
    let pool = state.db();

    let existing = db::podcasts::list_feeds(pool, viewer).await?;
    if let Some(found) = existing.into_iter().find(|f| f.feed_url == feed_url) {
        return Ok((found, false));
    }

    let feed = db::podcasts::insert_feed(pool, user_id, feed_family_id, feed_url, &channel.title).await?;

    // Prefer the iTunes high-resolution artwork over the standard RSS image. Stored as a raw
    // URL only — `get_feed_image` fetches and caches it lazily on first request, same reasoning
    // as the per-episode art below: subscribing must not block on an external image host at all.
    let image_url: Option<String> = channel
        .itunes_ext()
        .and_then(|e| e.image())
        .map(str::to_string)
        .or_else(|| channel.image().map(|i| i.url().to_string()));
    let image_object_id: Option<Uuid> = None;

    sqlx::query(
        "UPDATE podcast_feeds SET description=$2, author=$3, link=$4, language=$5,
         image_url=$6, image_object_id=$7, last_refreshed_at=CURRENT_TIMESTAMP WHERE id=$1",
    )
    .bind(feed.id)
    .bind(Some(channel.description.as_str()))
    .bind(channel.itunes_ext.as_ref().and_then(|e| e.author.as_deref()))
    .bind(Some(channel.link.as_str()))
    .bind(channel.language.as_deref())
    .bind(image_url.as_deref())
    .bind(image_object_id)
    .execute(pool)
    .await?;

    enrich_from_catalog(state, feed.id, feed_url, &channel).await;

    ingest_episodes_inner(pool, feed.id, &channel)
        .await
        .map_err(|e| anyhow::anyhow!("ingest episodes: {e:?}"))?;

    let fresh = db::podcasts::find_feed(pool, feed.id, viewer)
        .await?
        .ok_or_else(|| anyhow::anyhow!("feed vanished after insert"))?;

    Ok((fresh, true))
}

async fn subscribe_youtube(
    family: FamilyContext,
    state: AppState,
    body: SubscribeRequest,
) -> Result<(StatusCode, Json<FeedResponse>), AuthError> {
    let pool = state.db();
    let feed_family_id =
        access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let snapshot = youtube::fetch_channel(&body.feed_url)
        .await
        .map_err(AuthError::Internal)?;

    let existing = db::podcasts::list_feeds(pool, family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    if let Some(existing_feed) = existing.into_iter().find(|f| {
        f.feed_url == body.feed_url
            || (f.source_type == "youtube"
                && f.youtube_channel_id.is_some()
                && f.youtube_channel_id == snapshot.channel_id)
    }) {
        return Ok((StatusCode::OK, Json(feed_to_response(existing_feed, family.user_id))));
    }

    let canonical_feed_url = snapshot.channel_url.clone().unwrap_or_else(|| body.feed_url.clone());
    let feed = db::podcasts::insert_feed(pool, family.user_id, feed_family_id, &canonical_feed_url, &snapshot.title)
        .await
        .map_err(AuthError::Internal)?;

    let image_object_id = if let Some(ref url) = snapshot.image_url {
        store_image_from_url(pool, state.storage(), url).await.ok()
    } else {
        None
    };

    sqlx::query(
        "UPDATE podcast_feeds
            SET source_type = 'youtube',
                youtube_channel_id = $2,
                description = $3,
                author = $4,
                link = $5,
                language = NULL,
                image_url = $6,
                image_object_id = $7,
                last_refreshed_at = CURRENT_TIMESTAMP,
                refresh_error = NULL
          WHERE id = $1",
    )
    .bind(feed.id)
    .bind(snapshot.channel_id.as_deref())
    .bind(snapshot.description.as_deref())
    .bind(snapshot.author.as_deref())
    .bind(snapshot.channel_url.as_deref())
    .bind(snapshot.image_url.as_deref())
    .bind(image_object_id)
    .execute(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    ingest_youtube_episodes_inner(pool, feed.id, &snapshot, state.storage()).await?;

    let fresh = db::podcasts::find_feed(pool, feed.id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("feed vanished after insert")))?;

    Ok((StatusCode::CREATED, Json(feed_to_response(fresh, family.user_id))))
}

/// GET /api/v1/podcasts/:id
///
/// One feed.
#[utoipa::path(get, path = "/{id}", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    responses((status = 200, body = FeedResponse), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn get_feed(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<FeedResponse>, AuthError> {
    let feed = db::podcasts::find_feed(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let has_transcripts = db::podcasts::feeds_with_transcripts(state.db(), &[feed.id])
        .await
        .map_err(AuthError::Internal)?
        .contains(&feed.id);

    Ok(Json(FeedResponse { has_transcripts, ..feed_to_response(feed, family.user_id) }))
}

/// DELETE /api/v1/podcasts/:id
///
/// Unsubscribes; only the feed's owner can.
#[utoipa::path(delete, path = "/{id}", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    responses((status = 204, description = "Unsubscribed"), (status = 404, description = "No such feed owned by the caller", body = crate::http::openapi::ErrorBody)))]
async fn unsubscribe(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    // Unsubscribing is an owner action; resolve the row first so the
    // tombstone knows whether the feed was shared.
    let feed = db::podcasts::find_feed_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::podcasts::delete_feed(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?;

    let _ = db::sync::record_deletion(
        state.db(),
        access::PODCAST,
        id,
        family.user_id,
        feed.family_id,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/podcasts/:id/episodes
///
/// A page of the feed's episodes, with the caller's progress on each.
#[utoipa::path(get, path = "/{id}/episodes", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id"), EpisodeListQuery),
    responses((status = 200, body = Vec<EpisodeResponse>), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn list_episodes(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(q): Query<EpisodeListQuery>,
) -> Result<Json<Vec<EpisodeResponse>>, AuthError> {
    let pool = state.db();

    // Verify feed belongs to this user
    db::podcasts::find_feed(pool, id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let episodes = db::podcasts::list_episodes(pool, id, q.limit, q.offset, q.q.as_deref())
        .await
        .map_err(AuthError::Internal)?;

    // Fetch all progress rows for this user + feed in one query
    let progress_rows: Vec<(Uuid, f64, bool)> = sqlx::query_as(
        "SELECT episode_id, position_secs, completed
           FROM podcast_progress
          WHERE user_id = $1 AND episode_id IN (SELECT id FROM podcast_episodes WHERE feed_id = $2)",
    )
    .bind(family.user_id)
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    Ok(Json(
        episodes
            .into_iter()
            .map(|ep| {
                let prog = progress_rows.iter().find(|(eid, _, _)| *eid == ep.id);
                episode_to_response_with_progress(ep, prog.map(|(_, pos, done)| (*pos, *done)))
            })
            .collect(),
    ))
}

/// POST /api/v1/podcasts/:id/refresh  — re-fetch feed and sync new episodes
#[utoipa::path(post, path = "/{id}/refresh", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    responses((status = 204, description = "Refreshed"), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody), (status = 500, description = "The feed could not be fetched", body = crate::http::openapi::ErrorBody)))]
async fn refresh_feed(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    let pool = state.db();

    let feed = db::podcasts::find_feed(pool, id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if feed.source_type == "youtube" {
        refresh_youtube_feed_inner(pool, state.storage(), id, &feed.feed_url).await?;
        return Ok(StatusCode::NO_CONTENT);
    }

    let channel = fetch_rss(&feed.feed_url, &state.config().auth.session_secret).await?;

    // Raw URL only, same reasoning as `subscribe_rss_feed` — an interactive tap of the
    // Refresh button must not block on an external image host either. `COALESCE` below keeps
    // whatever object id a prior lazy fetch (or `sync_images`) already resolved.
    let image_url: Option<String> = channel
        .itunes_ext()
        .and_then(|e| e.image())
        .map(str::to_string)
        .or_else(|| channel.image().map(|i| i.url().to_string()));

    sqlx::query(
        "UPDATE podcast_feeds SET last_refreshed_at=CURRENT_TIMESTAMP, refresh_error=NULL,
         image_url=$2 WHERE id=$1",
    )
    .bind(id)
    .bind(image_url.as_deref())
    .execute(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    ingest_episodes_inner(pool, id, &channel).await?;
    if let Err(e) = queue_auto_store(pool, id).await {
        tracing::warn!(feed_id = %id, "could not queue auto-store downloads: {e:#}");
    }

    Ok(StatusCode::NO_CONTENT)
}

pub async fn refresh_feed_job(
    pool: &sqlx::PgPool,
    storage: &ObjectStore,
    feed_id: Uuid,
) -> anyhow::Result<()> {
    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT feed_url, source_type FROM podcast_feeds WHERE id = $1",
    )
    .bind(feed_id)
    .fetch_optional(pool)
    .await?;

    let (feed_url, source_type) = row.ok_or_else(|| anyhow::anyhow!("feed {feed_id} not found"))?;

    // Count episodes before and after so we can tell whether anything new
    // actually arrived; ingest upserts, so row counts are the honest signal.
    let before: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM podcast_episodes WHERE feed_id = $1")
            .bind(feed_id)
            .fetch_one(pool)
            .await?;

    if source_type == "youtube" {
        refresh_youtube_feed_inner(pool, storage, feed_id, &feed_url).await?;
    } else {
        let channel = fetch_rss_url(&feed_url).await?;
        ingest_episodes(pool, feed_id, &channel).await?;
        sqlx::query(
            "UPDATE podcast_feeds SET last_refreshed_at = CURRENT_TIMESTAMP, refresh_error = NULL WHERE id = $1",
        )
        .bind(feed_id)
        .execute(pool)
        .await?;
    }

    let after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM podcast_episodes WHERE feed_id = $1")
        .bind(feed_id)
        .fetch_one(pool)
        .await?;

    if after > before {
        notify_new_episodes(pool, feed_id, after - before).await;
    }
    if let Err(e) = queue_auto_store(pool, feed_id).await {
        tracing::warn!(%feed_id, "could not queue auto-store downloads: {e:#}");
    }

    Ok(())
}

/// Queue a "new episodes" notification for everyone who can see this feed:
/// its owner, plus family members when it is shared and they are allowed it.
///
/// Best-effort — a refresh must not fail because a notification could not be
/// queued.
async fn notify_new_episodes(pool: &sqlx::PgPool, feed_id: Uuid, new_count: i64) {
    let feed = match sqlx::query_as::<_, (String, Uuid, Option<Uuid>)>(
        "SELECT title, user_id, family_id FROM podcast_feeds WHERE id = $1",
    )
    .bind(feed_id)
    .fetch_optional(pool)
    .await
    {
        Ok(Some(feed)) => feed,
        _ => return,
    };

    let (title, owner_id, family_id) = feed;

    // The owner always hears about it; family members only for a shared feed
    // and only if their policy/grants let them play it.
    let mut recipients = vec![owner_id];
    if let Some(family_id) = family_id {
        if let Ok(audience) =
            crate::db::access::item_audience(pool, family_id, crate::db::access::PODCAST, feed_id)
                .await
        {
            recipients.extend(
                audience
                    .into_iter()
                    .filter(|(user_id, allowed, _locked)| *allowed && *user_id != owner_id)
                    .map(|(user_id, _, _)| user_id),
            );
        }
    }

    let body = if new_count == 1 {
        "1 new episode".to_string()
    } else {
        format!("{new_count} new episodes")
    };
    let data = serde_json::json!({
        "media_kind": "podcast",
        "item_id": feed_id.to_string(),
    });

    for user_id in recipients {
        let _ = crate::db::sync::queue_notification(
            pool,
            user_id,
            "new_episodes",
            &title,
            Some(&body),
            Some(&data),
        )
        .await;
    }
}

async fn refresh_youtube_feed_inner(
    pool: &sqlx::PgPool,
    storage: &ObjectStore,
    feed_id: Uuid,
    feed_url: &str,
) -> Result<(), AuthError> {
    let snapshot = youtube::fetch_channel(feed_url)
        .await
        .map_err(AuthError::Internal)?;

    let image_object_id = if let Some(ref url) = snapshot.image_url {
        store_image_from_url(pool, storage, url).await.ok()
    } else {
        None
    };

    sqlx::query(
        "UPDATE podcast_feeds
            SET source_type = 'youtube',
                youtube_channel_id = COALESCE($2, youtube_channel_id),
                title = $3,
                description = $4,
                author = $5,
                link = $6,
                image_url = $7,
                image_object_id = COALESCE($8, image_object_id),
                    last_refreshed_at = CURRENT_TIMESTAMP,
                refresh_error = NULL
          WHERE id = $1",
    )
    .bind(feed_id)
    .bind(snapshot.channel_id.as_deref())
    .bind(&snapshot.title)
    .bind(snapshot.description.as_deref())
    .bind(snapshot.author.as_deref())
    .bind(snapshot.channel_url.as_deref())
    .bind(snapshot.image_url.as_deref())
    .bind(image_object_id)
    .execute(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    ingest_youtube_episodes_inner(pool, feed_id, &snapshot, storage).await
}

/// PUT /api/v1/podcasts/:id/visibility — owner only.
#[utoipa::path(put, path = "/{id}/visibility", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    request_body = SetVisibilityRequest,
    responses((status = 200, body = FeedResponse), (status = 400, description = "visibility is not `private` or `family`", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such feed owned by the caller", body = crate::http::openapi::ErrorBody)))]
async fn set_feed_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetVisibilityRequest>,
) -> Result<Json<FeedResponse>, AuthError> {
    let shared = match body.visibility.trim() {
        access::VIS_PRIVATE => false,
        access::VIS_FAMILY => true,
        _ => {
            return Err(AuthError::BadRequest(
                "visibility must be 'private' or 'family'".to_string(),
            ));
        }
    };

    let updated = access::set_shared(
        state.db(),
        family.user_id,
        family.family_id,
        access::PODCAST,
        id,
        shared,
    )
    .await
    .map_err(AuthError::Internal)?;

    if !updated {
        return Err(AuthError::ItemNotFound);
    }

    let fresh = db::podcasts::find_feed_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(feed_to_response(fresh, family.user_id)))
}

/// PUT /api/v1/podcasts/{id}/auto-store — `{"enabled": true}` makes the server
/// store every episode published from now on, whether or not any device is
/// running; paid feeds whose links expire are the reason (§5.6). No backlog.
/// The show's subscriber, or a family admin when it is shared with the family.
#[utoipa::path(put, path = "/{id}/auto-store", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    request_body = SetAutoStoreRequest,
    responses((status = 200, body = FeedResponse), (status = 403, description = "Neither the subscriber nor a family admin of a shared feed", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn set_auto_store(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetAutoStoreRequest>,
) -> Result<Json<FeedResponse>, AuthError> {
    family.require_can_upload()?;
    let feed = db::podcasts::find_feed(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let may = feed.user_id == family.user_id
        || (family.is_family_admin() && feed.family_id == Some(family.family_id));
    if !may {
        return Err(AuthError::Forbidden);
    }
    sqlx::query(
        "UPDATE podcast_feeds
            SET auto_store_since = CASE WHEN $2 THEN COALESCE(auto_store_since, now()) END
          WHERE id = $1",
    )
    .bind(id)
    .bind(body.enabled)
    .execute(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    let fresh = db::podcasts::find_feed(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    Ok(Json(feed_to_response(fresh, family.user_id)))
}

#[derive(Deserialize, ToSchema)]
struct StoreAllRequest {
    /// True: say how many episodes and roughly how many bytes, queue nothing.
    #[serde(default)]
    preview: bool,
    /// Only the newest this many episodes not stored yet; all of them when absent.
    #[serde(default)]
    latest: Option<i64>,
}

#[derive(Serialize, ToSchema)]
struct StoreAllResponse {
    /// Episodes queued — or, for a preview, that would be.
    episodes: i64,
    /// A rough size from the episodes' durations at 128 kbit/s; feeds rarely state sizes.
    estimated_bytes: i64,
}

/// Episodes of a show not stored yet and never queued — still listed, with audio, not
/// deleted — newest first, at most `$2` of them (all with a null).
const UNSTORED: &str = "id IN (SELECT id FROM podcast_episodes
                               WHERE feed_id = $1 AND audio_object_id IS NULL AND trashed_at IS NULL
                                 AND audio_url IS NOT NULL AND auto_stored_at IS NULL
                               ORDER BY COALESCE(published_at, created_at) DESC
                               LIMIT $2)";

/// POST /api/v1/podcasts/{id}/store-all — stores the back catalogue on the server (all of it,
/// or the newest `latest` episodes): an
/// `episode_download` job per episode not stored yet, so a paid feed stays in the family's
/// library after the subscription ends. Same permission as auto-store. An episode queued once
/// is not queued again, and one whose copy was deleted is not stored again.
#[utoipa::path(post, path = "/{id}/store-all", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    request_body = StoreAllRequest,
    responses((status = 200, description = "Episodes queued, or that would be for a preview", body = StoreAllResponse), (status = 403, description = "Neither the subscriber nor a family admin of a shared feed", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn store_all(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<StoreAllRequest>,
) -> Result<Json<StoreAllResponse>, AuthError> {
    family.require_can_upload()?;
    let feed = db::podcasts::find_feed(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let may = feed.user_id == family.user_id
        || (family.is_family_admin() && feed.family_id == Some(family.family_id));
    if !may {
        return Err(AuthError::Forbidden);
    }
    let latest = body.latest.filter(|n| *n > 0);
    let (episodes, seconds): (i64, Option<i64>) = sqlx::query_as(&format!(
        "SELECT count(*), sum(COALESCE(duration_secs, 1800))::bigint FROM podcast_episodes WHERE {UNSTORED}"
    ))
    .bind(id)
    .bind(latest)
    .fetch_one(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    let estimated_bytes = seconds.unwrap_or(0) * 16_000;
    if body.preview {
        return Ok(Json(StoreAllResponse { episodes, estimated_bytes }));
    }
    let queued: Vec<Uuid> = sqlx::query_scalar(&format!(
        "UPDATE podcast_episodes SET auto_stored_at = now() WHERE {UNSTORED} RETURNING id"
    ))
    .bind(id)
    .bind(latest)
    .fetch_all(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    for episode_id in &queued {
        let payload = serde_json::json!({ "episode_id": episode_id.to_string() });
        db::jobs::enqueue(state.db(), "episode_download", Some(&payload), None)
            .await
            .map_err(AuthError::Internal)?;
    }
    Ok(Json(StoreAllResponse { episodes: queued.len() as i64, estimated_bytes }))
}

/// Queue `episode_download` for the new episodes of an auto-store show — those
/// published since the switch was turned on and never queued before, so an
/// episode the family deleted is not stored again. Returns how many.
pub async fn queue_auto_store(pool: &sqlx::PgPool, feed_id: Uuid) -> anyhow::Result<usize> {
    let queued: Vec<Uuid> = sqlx::query_scalar(
        "UPDATE podcast_episodes e SET auto_stored_at = now()
           FROM podcast_feeds f
          WHERE f.id = e.feed_id AND e.feed_id = $1 AND f.auto_store_since IS NOT NULL
            AND e.auto_stored_at IS NULL AND e.audio_object_id IS NULL AND e.trashed_at IS NULL
            AND e.audio_url IS NOT NULL
            AND COALESCE(e.published_at, e.created_at) >= f.auto_store_since
         RETURNING e.id",
    )
    .bind(feed_id)
    .fetch_all(pool)
    .await?;
    for episode_id in &queued {
        let payload = serde_json::json!({ "episode_id": episode_id.to_string() });
        db::jobs::enqueue(pool, "episode_download", Some(&payload), None).await?;
    }
    Ok(queued.len())
}

// ── Internal helpers ──────────────────────────────────────────────────────

async fn fetch_rss(url: &str, _secret: &str) -> Result<rss::Channel, AuthError> {
    fetch_rss_url(url).await.map_err(fetch_failure)
}

/// A URL the outbound guard refused is the caller's mistake (400); anything
/// else that went wrong fetching is ours (500).
pub(crate) fn fetch_failure(error: anyhow::Error) -> AuthError {
    if crate::http::outbound::is_refused(&error) {
        AuthError::BadRequest(error.to_string())
    } else {
        AuthError::Internal(error)
    }
}

/// Public version used by the background worker (returns anyhow::Result).
pub async fn fetch_rss_url(url: &str) -> anyhow::Result<rss::Channel> {
    let response = crate::http::outbound::get(url).await.map_err(|e| e.context("fetch feed"))?;
    let bytes = crate::http::outbound::bytes(response, MAX_FEED_BYTES).await.map_err(|e| e.context("read feed body"))?;

    rss::Channel::read_from(&bytes[..])
        .map_err(|e| anyhow::anyhow!("parse feed: {e}"))
}

/// Public version used by the background worker (returns anyhow::Result).
pub async fn ingest_episodes(
    pool: &sqlx::PgPool,
    feed_id: Uuid,
    channel: &rss::Channel,
) -> anyhow::Result<()> {
    ingest_episodes_inner(pool, feed_id, channel)
        .await
        .map_err(|e| match e {
            AuthError::Internal(inner) => inner,
            other => anyhow::anyhow!("{other:?}"),
        })
}

async fn ingest_youtube_episodes_inner(
    pool: &sqlx::PgPool,
    feed_id: Uuid,
    channel: &youtube::YouTubeChannelSnapshot,
    storage: &ObjectStore,
) -> Result<(), AuthError> {
    for item in &channel.entries {
        let guid = item.video_id.clone();
        let title = item.title.clone();
        let description = item.description.clone();
        let audio_url = Some(item.watch_url.clone());
        let published_at = item.published_at;
        let duration_secs = item.duration_secs;
        let episode_number = None::<i32>;

        let ep_image_object_id = if let Some(ref url) = item.image_url {
            store_image_from_url(pool, storage, url).await.ok()
        } else {
            None
        };

        sqlx::query(
            "INSERT INTO podcast_episodes
             (feed_id, guid, title, description, audio_url, published_at,
              duration_secs, episode_number, image_url, image_object_id)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10)
             ON CONFLICT (feed_id, guid) DO UPDATE
               SET title            = EXCLUDED.title,
                   description      = COALESCE(EXCLUDED.description, podcast_episodes.description),
                   audio_url        = COALESCE(EXCLUDED.audio_url, podcast_episodes.audio_url),
                   published_at     = COALESCE(EXCLUDED.published_at, podcast_episodes.published_at),
                   duration_secs    = COALESCE(EXCLUDED.duration_secs, podcast_episodes.duration_secs),
                   image_url        = COALESCE(EXCLUDED.image_url, podcast_episodes.image_url),
                   image_object_id  = COALESCE(EXCLUDED.image_object_id, podcast_episodes.image_object_id)",
        )
        .bind(feed_id)
        .bind(&guid)
        .bind(&title)
        .bind(&description)
        .bind(&audio_url)
        .bind(published_at)
        .bind(duration_secs)
        .bind(episode_number)
        .bind(item.image_url.as_deref())
        .bind(ep_image_object_id)
        .execute(pool)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    Ok(())
}

async fn ingest_episodes_inner(
    pool: &sqlx::PgPool,
    feed_id: Uuid,
    channel: &rss::Channel,
) -> Result<(), AuthError> {
    for item in &channel.items {
        let guid = match item.guid() {
            Some(g) => g.value().to_string(),
            None => match item.link() {
                Some(l) => l.to_string(),
                None => continue,
            },
        };

        let title = item.title().unwrap_or("Untitled").to_string();
        let description = item.description().map(str::to_string);
        let audio_url = item.enclosure().map(|e| e.url().to_string());
        let published_at: Option<DateTime<Utc>> = item
            .pub_date()
            .and_then(|s| chrono::DateTime::parse_from_rfc2822(s).ok())
            .map(|d| d.with_timezone(&Utc));

        let duration_secs: Option<i32> = item
            .itunes_ext()
            .and_then(|e| e.duration())
            .and_then(parse_duration);

        let episode_number: Option<i32> = item
            .itunes_ext()
            .and_then(|e| e.episode())
            .and_then(|n| n.parse().ok());

        // Episode-level cover art (often same URL as channel — deduped by hash). The raw URL
        // is stored now; `image_object_id` is filled in lazily, on first request, by
        // `get_episode_image` — not fetched here. A feed with hundreds of episodes each with
        // unique art used to mean hundreds of sequential external fetch+upload round trips
        // inside this one call, which is exactly what made subscribing to such a feed exceed
        // Cloudflare's origin timeout (524) even though it "worked" against a local backend
        // with no edge timeout in front of it. `storage` is still threaded through as a
        // parameter — the background job path and `sync_images` (an explicit, separately
        // callable "pre-warm every image now" action, `POST /podcasts/{id}/sync-images`) both
        // still do real eager fetching, just never as a side effect of this loop.
        let ep_image_url: Option<String> = item
            .itunes_ext()
            .and_then(|e| e.image())
            .map(str::to_string);
        let ep_image_object_id: Option<Uuid> = None;

        let transcript = extract_transcript(item);

        // Upsert episode; on conflict fill in image fields only if not yet set, but let a
        // freshly-fetched transcript URL overwrite an older one — feeds commonly attach
        // transcripts some time after publishing, so a later refresh must be able to fill
        // this in (or replace it) even when the episode row already exists.
        sqlx::query(
            "INSERT INTO podcast_episodes
             (feed_id, guid, title, description, audio_url, published_at,
              duration_secs, episode_number, image_url, image_object_id,
              transcript_url, transcript_type)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12)
             ON CONFLICT (feed_id, guid) DO UPDATE
               SET image_url        = COALESCE(podcast_episodes.image_url,        EXCLUDED.image_url),
                   image_object_id  = COALESCE(podcast_episodes.image_object_id,  EXCLUDED.image_object_id),
                   transcript_url   = COALESCE(EXCLUDED.transcript_url,  podcast_episodes.transcript_url),
                   transcript_type  = COALESCE(EXCLUDED.transcript_type, podcast_episodes.transcript_type)",
        )
        .bind(feed_id)
        .bind(&guid)
        .bind(&title)
        .bind(&description)
        .bind(&audio_url)
        .bind(published_at)
        .bind(duration_secs)
        .bind(episode_number)
        .bind(ep_image_url.as_deref())
        .bind(ep_image_object_id)
        .bind(transcript.as_ref().map(|(url, _)| url.as_str()))
        .bind(transcript.as_ref().map(|(_, mime)| mime.as_str()))
        .execute(pool)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    Ok(())
}

/// Reads the Podcasting 2.0 `<podcast:transcript>` tag(s) off a feed item, preferring a
/// format with cue timing (VTT, then SRT) over plain JSON/text — see Phase 2 of
/// docs/podcast-translation-plan.md, which strips cues down to plain text either way, but a
/// richer source degrades more gracefully if that stripping ever needs to preserve timing.
/// A feed may list several variants of the same transcript; only the best one is kept.
fn extract_transcript(item: &rss::Item) -> Option<(String, String)> {
    let variants = item.extensions().get("podcast")?.get("transcript")?;

    let mut best: Option<(&str, &str, u8)> = None;
    for ext in variants {
        let attrs = ext.attrs();
        let Some(url) = attrs.get("url") else { continue };
        let mime = attrs.get("type").map(String::as_str).unwrap_or("text/plain");
        let rank: u8 = match mime {
            "text/vtt" => 3,
            "application/srt" | "text/srt" | "application/x-subrip" => 2,
            "application/json" | "application/json+srv3" => 1,
            _ => 0,
        };
        if best.map(|(_, _, best_rank)| rank > best_rank).unwrap_or(true) {
            best = Some((url.as_str(), mime, rank));
        }
    }

    best.map(|(url, mime, _)| (url.to_string(), mime.to_string()))
}

/// Parse "HH:MM:SS" or "MM:SS" or plain seconds into `i32`.
fn parse_duration(s: &str) -> Option<i32> {
    let parts: Vec<&str> = s.split(':').collect();
    match parts.len() {
        1 => parts[0].parse().ok(),
        2 => {
            let m: i32 = parts[0].parse().ok()?;
            let s: i32 = parts[1].parse().ok()?;
            Some(m * 60 + s)
        }
        3 => {
            let h: i32 = parts[0].parse().ok()?;
            let m: i32 = parts[1].parse().ok()?;
            let s: i32 = parts[2].parse().ok()?;
            Some(h * 3600 + m * 60 + s)
        }
        _ => None,
    }
}

fn feed_to_response(f: crate::podcasts::models::PodcastFeed, viewer_id: Uuid) -> FeedResponse {
    // If the image is stored in S3, point the frontend at our proxy endpoint
    // so it never hits the external CDN directly.
    let image_url = if f.image_object_id.is_some() {
        Some(format!("/api/v1/podcasts/{}/image", f.id))
    } else {
        f.image_url
    };
    FeedResponse {
        id: f.id.to_string(),
        feed_url: f.feed_url,
        source_type: f.source_type,
        title: f.title,
        description: f.description,
        author: f.author,
        link: f.link,
        language: f.language,
        visibility: crate::db::access::visibility_of(f.family_id).to_string(),
        is_owner: f.user_id == viewer_id,
        owner_id: f.user_id.to_string(),
        image_url,
        last_refreshed_at: f.last_refreshed_at.map(|t| t.to_rfc3339()),
        categories: f.categories,
        language_base: f.language_base,
        auto_store: f.auto_store_since.is_some(),
        has_transcripts: false,
    }
}

fn episode_to_response(e: crate::podcasts::models::PodcastEpisode) -> EpisodeResponse {
    episode_to_response_with_progress(e, None)
}

fn episode_to_response_with_progress(
    e: crate::podcasts::models::PodcastEpisode,
    progress: Option<(f64, bool)>,
) -> EpisodeResponse {
    let image_url = if e.image_object_id.is_some() {
        Some(format!("/api/v1/podcasts/{}/episodes/{}/image", e.feed_id, e.id))
    } else {
        e.image_url
    };
    EpisodeResponse {
        has_local: e.audio_object_id.is_some(),
        id: e.id.to_string(),
        feed_id: e.feed_id.to_string(),
        guid: e.guid,
        title: e.title,
        description: e.description,
        published_at: e.published_at.map(|t| t.to_rfc3339()),
        duration_secs: e.duration_secs,
        episode_number: e.episode_number,
        audio_url: e.audio_url,
        image_url,
        progress_secs: progress.map(|(pos, _)| pos),
        completed: progress.map(|(_, done)| done).unwrap_or(false),
        size_bytes: e.size_bytes,
        sha256: e.sha256,
        has_transcript: e.transcript_url.is_some(),
    }
}

/// Download an image from `url` into the object store, deduplicating by URL.
/// Key = `shared/images/{sha256_of_url}.{ext}`. Returns the `media_objects.id`.
///
/// Deliberately outside the `f/{family_id}/` prefix every other object uses:
/// this is public podcast artwork addressed by the content URL, so two
/// families subscribed to the same feed share one object. It is also reached
/// from the feed-refresh worker, which has no family in scope.
async fn store_image_from_url(
    pool: &sqlx::PgPool,
    storage: &ObjectStore,
    image_url: &str,
) -> anyhow::Result<Uuid> {
    // Derive a stable key from the URL
    let hash = format!("{:x}", Sha256::digest(image_url.as_bytes()));
    let ext = image_url
        .rsplit('.')
        .next()
        .map(|e| e.split('?').next().unwrap_or(e))
        .filter(|e| matches!(*e, "jpg" | "jpeg" | "png" | "webp" | "gif"))
        .unwrap_or("jpg");
    let key = format!("shared/images/{hash}.{ext}");
    let bucket = storage.bucket().to_string();

    // Return early if already stored (dedup)
    if let Some(id) = sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM media_objects WHERE bucket=$1 AND object_key=$2",
    )
    .bind(&bucket)
    .bind(&key)
    .fetch_optional(pool)
    .await?
    {
        return Ok(id);
    }

    // Download. `outbound_client()`, not a bare `reqwest::get` — that had no timeout at all
    // (a dead or slow CDN would hang the caller indefinitely) and no user agent, which some
    // hosts reject.
    let resp = crate::http::outbound::get(image_url).await.map_err(|e| e.context("fetch image"))?;
    let content_type = crate::http::outbound::content_type(&resp, "image/jpeg");
    let bytes = crate::http::outbound::bytes(resp, MAX_FEED_BYTES).await.map_err(|e| e.context("read image"))?;

    // Upload
    storage.put(&key, bytes, &content_type).await?;

    // Upsert media_objects and return id
    let id = sqlx::query_scalar::<_, Uuid>(
        "INSERT INTO media_objects (bucket, object_key, content_type)
         VALUES ($1, $2, $3)
         ON CONFLICT (bucket, object_key) DO UPDATE SET content_type = EXCLUDED.content_type
         RETURNING id",
    )
    .bind(&bucket)
    .bind(&key)
    .bind(&content_type)
    .fetch_one(pool)
    .await?;

    Ok(id)
}

const STREAM_EXPIRY_SECS: u64 = 4 * 3600; // 4 hours

/// POST /api/v1/podcasts/{id}/episodes/{ep_id}/download
/// Fetches the episode audio from its RSS enclosure URL, stores it in S3,
/// and marks the episode as local. Returns the updated episode.
#[utoipa::path(post, path = "/{id}/episodes/{ep_id}/download", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id"), ("ep_id" = Uuid, Path, description = "Episode id")),
    responses((status = 200, description = "The episode, now stored", body = EpisodeResponse), (status = 404, description = "No such feed or episode, or not visible to the caller", body = crate::http::openapi::ErrorBody), (status = 500, description = "The audio could not be fetched or stored", body = crate::http::openapi::ErrorBody)))]
async fn download_episode(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((feed_id, ep_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<EpisodeResponse>, AuthError> {
    family.require_can_upload()?;
    let pool = state.db();

    // Verify the feed belongs to this user
    let feed = db::podcasts::find_feed(pool, feed_id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let episode = db::podcasts::find_episode(pool, ep_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if episode.feed_id != feed_id {
        return Err(AuthError::ItemNotFound);
    }

    // If already downloaded, return immediately
    if episode.audio_object_id.is_some() {
        return Ok(Json(episode_to_response(episode)));
    }

    let guard = crate::storage::quota::Guard::for_state(&state);
    fetch_and_store_episode_audio(state.db(), state.storage(), &guard, family.family_id, &feed, &episode)
        .await
        .map_err(crate::storage::quota::to_auth_error)?;

    let updated = db::podcasts::find_episode(pool, ep_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(episode_to_response(updated)))
}

/// DELETE /api/v1/podcasts/{id}/episodes/{ep_id}/download
///
/// Moves the household's stored copy of one episode to the trash for 30 days
/// (docs/file-sync-plan.md §5.1). The episode itself stays — it is still
/// listed, still has its description, and `POST …/download` fetches it again.
/// Only `has_local` goes back to false.
///
/// **Owner or family admin, unlike the `POST` above.** Anyone who can see a
/// shared feed may fetch an episode into storage, because that only adds;
/// deleting takes something away from everyone else in the household.
///
/// The client calls this when someone declines an episode — see the podcast
/// app's "Not Interested". A failure there is not worth surfacing (the episode
/// is hidden either way), which is why this returns the updated episode rather
/// than an error the app would have to explain.
#[utoipa::path(delete, path = "/{id}/episodes/{ep_id}/download", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id"), ("ep_id" = Uuid, Path, description = "Episode id")),
    responses((status = 200, description = "The episode, no longer stored", body = EpisodeResponse), (status = 403, description = "Visible, but the caller is neither the owner nor a family admin", body = crate::http::openapi::ErrorBody), (status = 404, description = "No such feed or episode, or not visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn delete_episode_download(
    family: FamilyContext,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path((feed_id, ep_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<EpisodeResponse>, AuthError> {
    let pool = state.db();

    let episode = db::podcasts::find_episode(pool, ep_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    if episode.feed_id != feed_id {
        return Err(AuthError::ItemNotFound);
    }

    if episode.audio_object_id.is_some() {
        // The stored copy goes to the trash for 30 days; the episode stays in
        // the feed's list, now simply not stored.
        crate::trash::move_to_trash(&state, family.viewer(), Some(&headers), crate::db::trash::Kind::PodcastEpisode, ep_id)
            .await?;
    } else if !access::can_listen(pool, family.viewer(), access::PODCAST, feed_id)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::ItemNotFound);
    }

    let updated = db::podcasts::find_episode(pool, ep_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(episode_to_response(updated)))
}

/// Fetches an episode's audio, stores it, and links the object to the episode.
///
/// Extracted from the REST handler so the Subsonic surface's
/// `downloadPodcastEpisode` performs exactly the same work rather than a second
/// implementation that could drift — the two differ only in how they report it.
///
/// Returns immediately if the episode is already stored.
pub(crate) async fn fetch_and_store_episode_audio(
    pool: &sqlx::PgPool,
    storage: &ObjectStore,
    guard: &crate::storage::quota::Guard,
    family_id: Uuid,
    feed: &crate::podcasts::models::PodcastFeed,
    episode: &crate::podcasts::models::PodcastEpisode,
) -> anyhow::Result<()> {
    if episode.audio_object_id.is_some() {
        return Ok(());
    }

    let ep_id = episode.id;
    let audio_url = episode
        .audio_url
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("episode has no audio_url"))?;

    // Feeds don't reliably say how large an enclosure is, so the room check
    // runs twice: for a family already out of room before anything is
    // fetched, and with the real size before the file is kept.
    guard.check(pool, storage.bucket(), family_id, 0).await?;

    info!(%ep_id, source_type = %feed.source_type, audio_url = %audio_url, "starting episode download");

    enum Body {
        Bytes(bytes::Bytes),
        Temp(tempfile::NamedTempFile, u64),
    }
    let (body, content_type) = if feed.source_type == "youtube" {
        let (bytes, content_type) = youtube::download_audio_to_bytes(audio_url).await?;
        (Body::Bytes(bytes), content_type)
    } else {
        // Through the outbound guard: the enclosure URL comes from the feed,
        // and the body goes to a temporary file under a size cap rather than
        // into memory (security hardening plan C2, C3).
        let response = crate::http::outbound::get(audio_url).await.map_err(|e| e.context("fetch audio"))?;
        let content_type = crate::http::outbound::content_type(&response, "audio/mpeg");
        if content_type.starts_with("text/") {
            anyhow::bail!("fetch audio: host returned {content_type}, not audio");
        }
        let (temp, len) = crate::http::outbound::to_temp(response, *MAX_EPISODE_BYTES)
            .await
            .map_err(|e| e.context("read audio body"))?;
        (Body::Temp(temp, len), content_type)
    };

    let fetched_len = match &body {
        Body::Bytes(bytes) => bytes.len() as i64,
        Body::Temp(_, len) => *len as i64,
    };
    guard.check(pool, storage.bucket(), family_id, fetched_len).await?;

    let object_key = crate::storage::family_key(family_id, format!("episodes/{ep_id}"));
    let bucket = storage.bucket().to_string();
    let byte_len = match body {
        Body::Bytes(bytes) => {
            let len = bytes.len();
            storage
                .put(&object_key, bytes, &content_type)
                .await
                .map_err(|e| e.context(format!("store episode audio: ep_id={ep_id}")))?;
            len
        }
        Body::Temp(temp, len) => {
            storage
                .put_temp(&object_key, temp, &content_type)
                .await
                .map_err(|e| e.context(format!("store episode audio: ep_id={ep_id}")))?;
            len as usize
        }
    };

    // `size_bytes` is recorded because Subsonic reports a `size` per episode,
    // which clients use to show what a download will cost before starting it.
    let media_id: Uuid = sqlx::query_scalar(
        "INSERT INTO media_objects (bucket, object_key, content_type, size_bytes)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (bucket, object_key) DO UPDATE
           SET content_type = EXCLUDED.content_type, size_bytes = EXCLUDED.size_bytes
         RETURNING id",
    )
    .bind(&bucket)
    .bind(&object_key)
    .bind(&content_type)
    .bind(byte_len as i64)
    .fetch_one(pool)
    .await?;

    sqlx::query("UPDATE podcast_episodes SET audio_object_id = $1 WHERE id = $2")
        .bind(media_id)
        .bind(ep_id)
        .execute(pool)
        .await?;

    info!(%ep_id, %media_id, "episode download persisted successfully");
    // Where the stored copy sits in the own.audio folder (file-sync-plan §5.8).
    if let Err(e) = crate::filesync::paths::ensure_episode(pool, ep_id).await {
        tracing::warn!(%ep_id, "no path for the stored episode: {e:#}");
    }
    Ok(())
}

/// GET /api/v1/podcasts/{id}/episodes/{ep_id}/stream
/// Returns a short-lived presigned URL for streaming the downloaded episode.
#[utoipa::path(get, path = "/{id}/episodes/{ep_id}/stream", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id"), ("ep_id" = Uuid, Path, description = "Episode id")),
    responses((status = 200, description = "A presigned URL, valid for `expires_in_secs`", body = StreamResponse), (status = 404, description = "No such feed or episode, or not visible to the caller", body = crate::http::openapi::ErrorBody), (status = 409, description = "`episode_not_downloaded`: the episode is not stored on the server", body = crate::http::openapi::ErrorBody)))]
async fn stream_episode(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((feed_id, ep_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<StreamResponse>, AuthError> {
    let pool = state.db();

    // Verify feed ownership
    db::podcasts::find_feed(pool, feed_id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let episode = db::podcasts::find_episode(pool, ep_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if episode.feed_id != feed_id {
        return Err(AuthError::ItemNotFound);
    }

    let audio_object_id = episode
        .audio_object_id
        .ok_or_else(|| AuthError::Conflict("episode_not_downloaded".to_string()))?;

    // Read the stored key rather than recomputing it: the layout changed once
    // already (episodes are under the family prefix now) and objects written
    // by an older build must keep resolving.
    let key = db::subsonic::media_object_key(pool, audio_object_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("episode audio object row is missing")))?;

    let url = state
        .storage()
        .presigned_get(&key, STREAM_EXPIRY_SECS)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(StreamResponse {
        url,
        expires_in_secs: STREAM_EXPIRY_SECS,
    }))
}

/// POST /api/v1/podcasts/{id}/sync-images
/// Downloads & stores channel artwork + every episode's artwork (deduped by URL hash).
/// Safe to call multiple times — already-stored images are skipped.
#[utoipa::path(post, path = "/{id}/sync-images", tag = "podcasts", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Feed id")),
    responses((status = 200, body = FeedResponse), (status = 404, description = "No such feed, or not visible to the caller", body = crate::http::openapi::ErrorBody)))]
async fn sync_images(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<FeedResponse>, AuthError> {
    let pool = state.db();

    let feed = db::podcasts::find_feed(pool, id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let channel = fetch_rss(&feed.feed_url, &state.config().auth.session_secret).await?;

    // ── Channel artwork ──────────────────────────────────────────────────
    let channel_image_url: Option<String> = channel
        .itunes_ext()
        .and_then(|e| e.image())
        .map(str::to_string)
        .or_else(|| channel.image().map(|i| i.url().to_string()));

    let channel_image_object_id: Option<Uuid> = if let Some(ref url) = channel_image_url {
        store_image_from_url(pool, state.storage(), url).await.ok()
    } else {
        None
    };

    if channel_image_object_id.is_some() || channel_image_url.is_some() {
        sqlx::query(
            "UPDATE podcast_feeds SET
               image_url = COALESCE($2, image_url),
               image_object_id = COALESCE($3, image_object_id)
             WHERE id = $1",
        )
        .bind(id)
        .bind(channel_image_url.as_deref())
        .bind(channel_image_object_id)
        .execute(pool)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    // ── Episode artwork ──────────────────────────────────────────────────
    for item in &channel.items {
        let guid = match item.guid() {
            Some(g) => g.value().to_string(),
            None => continue,
        };
        let ep_image_url: Option<String> = item
            .itunes_ext()
            .and_then(|e| e.image())
            .map(str::to_string);

        let ep_image_object_id: Option<Uuid> = if let Some(ref url) = ep_image_url {
            store_image_from_url(pool, state.storage(), url).await.ok()
        } else {
            None
        };

        if ep_image_object_id.is_some() || ep_image_url.is_some() {
            sqlx::query(
                "UPDATE podcast_episodes SET
                   image_url = COALESCE(image_url, $3),
                   image_object_id = COALESCE(image_object_id, $4)
                 WHERE feed_id = $1 AND guid = $2",
            )
            .bind(id)
            .bind(&guid)
            .bind(ep_image_url.as_deref())
            .bind(ep_image_object_id)
            .execute(pool)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
        }
    }

    // Return the updated feed so the frontend can refresh state immediately
    let fresh = db::podcasts::find_feed(pool, id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(feed_to_response(fresh, family.user_id)))
}

/// GET /api/v1/podcasts/{id}/image
/// Proxies the podcast cover from S3 so the browser never hits an external CDN.
/// No auth required — podcast artwork is public content; UUID acts as capability token.
///
/// Subscribing/refreshing no longer fetch artwork eagerly (see `ingest_episodes_inner`'s doc
/// comment) — this is where that deferred fetch actually happens, once, on whichever request
/// is first to ask for it. Every request after that hits the fast, already-cached path above.
#[utoipa::path(get, path = "/{id}/image", tag = "podcasts",
    params(("id" = Uuid, Path, description = "Feed id")),
    responses((status = 200, description = "The cover art", content_type = "image/*"), (status = 404, description = "No such feed, or it has no artwork", body = crate::http::openapi::ErrorBody)))]
async fn get_feed_image(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AuthError> {
    let pool = state.db();

    if let Some((object_key, content_type)) = sqlx::query_as::<_, (String, String)>(
        "SELECT mo.object_key, mo.content_type
           FROM media_objects mo
           JOIN podcast_feeds pf ON pf.image_object_id = mo.id
          WHERE pf.id = $1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    {
        let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;
        return Ok((
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "max-age=86400, immutable".to_string()),
            ],
            bytes,
        ));
    }

    let image_url: String =
        sqlx::query_scalar::<_, Option<String>>("SELECT image_url FROM podcast_feeds WHERE id = $1")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?
            .flatten()
            .ok_or(AuthError::ItemNotFound)?;

    let object_id = store_image_from_url(pool, state.storage(), &image_url)
        .await
        .map_err(AuthError::Internal)?;

    sqlx::query("UPDATE podcast_feeds SET image_object_id = $2 WHERE id = $1")
        .bind(id)
        .bind(object_id)
        .execute(pool)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    let (object_key, content_type) = sqlx::query_as::<_, (String, String)>(
        "SELECT object_key, content_type FROM media_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "max-age=86400, immutable".to_string()),
        ],
        bytes,
    ))
}

/// GET /api/v1/podcasts/{id}/episodes/{ep_id}/image
/// Proxies the episode cover from S3.
/// No auth required — episode artwork is public content; UUID acts as capability token.
///
/// Same lazy-fetch-on-first-view shape as `get_feed_image` above, for the same reason.
#[utoipa::path(get, path = "/{id}/episodes/{ep_id}/image", tag = "podcasts",
    params(("id" = Uuid, Path, description = "Feed id"), ("ep_id" = Uuid, Path, description = "Episode id")),
    responses((status = 200, description = "The episode art", content_type = "image/*"), (status = 404, description = "No such episode, or it has no artwork", body = crate::http::openapi::ErrorBody)))]
async fn get_episode_image(
    State(state): State<AppState>,
    Path((feed_id, ep_id)): Path<(Uuid, Uuid)>,
) -> Result<impl IntoResponse, AuthError> {
    let pool = state.db();

    if let Some((object_key, content_type)) = sqlx::query_as::<_, (String, String)>(
        "SELECT mo.object_key, mo.content_type
           FROM media_objects mo
           JOIN podcast_episodes pe ON pe.image_object_id = mo.id
           JOIN podcast_feeds pf    ON pf.id = pe.feed_id
          WHERE pe.id = $1 AND pf.id = $2",
    )
    .bind(ep_id)
    .bind(feed_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    {
        let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;
        return Ok((
            [
                (header::CONTENT_TYPE, content_type),
                (header::CACHE_CONTROL, "max-age=86400, immutable".to_string()),
            ],
            bytes,
        ));
    }

    let image_url: String = sqlx::query_scalar::<_, Option<String>>(
        "SELECT pe.image_url FROM podcast_episodes pe
          WHERE pe.id = $1 AND pe.feed_id = $2",
    )
    .bind(ep_id)
    .bind(feed_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    .flatten()
    .ok_or(AuthError::ItemNotFound)?;

    let object_id = store_image_from_url(pool, state.storage(), &image_url)
        .await
        .map_err(AuthError::Internal)?;

    sqlx::query("UPDATE podcast_episodes SET image_object_id = $2 WHERE id = $1")
        .bind(ep_id)
        .bind(object_id)
        .execute(pool)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    let (object_key, content_type) = sqlx::query_as::<_, (String, String)>(
        "SELECT object_key, content_type FROM media_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_one(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;

    Ok((
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "max-age=86400, immutable".to_string()),
        ],
        bytes,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deliberately out of order, with one item carrying no date at all.
    const FEED_OUT_OF_ORDER: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
  <channel>
    <title>Test Feed</title>
    <link>https://example.com</link>
    <description>Test</description>
    <item>
      <title>Older</title>
      <guid>older</guid>
      <pubDate>Tue, 01 Sep 2026 08:00:00 +0000</pubDate>
      <itunes:duration>28:30</itunes:duration>
    </item>
    <item>
      <title>Undated</title>
      <guid>undated</guid>
    </item>
    <item>
      <title>Newest</title>
      <guid>newest</guid>
      <pubDate>Wed, 16 Sep 2026 08:00:00 +0000</pubDate>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn a_preview_lists_the_newest_episodes_first() {
        let channel = rss::Channel::read_from(FEED_OUT_OF_ORDER.as_bytes()).unwrap();
        let episodes = newest_episodes(&channel, 12);
        assert_eq!(
            episodes.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(),
            ["Newest", "Older", "Undated"],
            "an undated item must not pass for the latest one"
        );
        assert_eq!(episodes[1].duration_secs, Some(28 * 60 + 30));
    }

    #[test]
    fn a_preview_is_capped_at_what_was_asked_for() {
        let channel = rss::Channel::read_from(FEED_OUT_OF_ORDER.as_bytes()).unwrap();
        assert_eq!(newest_episodes(&channel, 2).len(), 2);
        // A caller asking for everything gets a screenful, not a whole archive.
        assert_eq!(newest_episodes(&channel, 5_000).len(), 3);
        assert_eq!(newest_episodes(&channel, 0).len(), 1, "zero is not a useful preview");
    }

    /// An item with neither guid nor link still has to be identifiable in a list.
    #[test]
    fn a_preview_episode_always_has_something_to_key_on() {
        let feed = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0"><channel><title>T</title><link>https://e.com</link><description>d</description>
<item><title>Only a title</title></item></channel></rss>"#;
        let channel = rss::Channel::read_from(feed.as_bytes()).unwrap();
        let episodes = newest_episodes(&channel, 12);
        assert_eq!(episodes[0].guid, "Only a title");
    }

    const FEED_WITH_TRANSCRIPTS: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<rss version="2.0" xmlns:podcast="https://podcastindex.org/namespace/1.0">
  <channel>
    <title>Test Feed</title>
    <link>https://example.com</link>
    <description>Test</description>
    <item>
      <title>Ep 1</title>
      <guid>ep1</guid>
      <podcast:transcript url="https://example.com/ep1.json" type="application/json" />
      <podcast:transcript url="https://example.com/ep1.vtt" type="text/vtt" />
    </item>
    <item>
      <title>Ep 2</title>
      <guid>ep2</guid>
      <podcast:transcript url="https://example.com/ep2.srt" type="application/srt" />
    </item>
    <item>
      <title>Ep 3 (no transcript)</title>
      <guid>ep3</guid>
    </item>
  </channel>
</rss>"#;

    #[test]
    fn extracts_transcript_preferring_vtt_over_json() {
        let channel = rss::Channel::read_from(FEED_WITH_TRANSCRIPTS.as_bytes()).unwrap();
        let found = extract_transcript(&channel.items[0]);
        assert_eq!(found, Some(("https://example.com/ep1.vtt".to_string(), "text/vtt".to_string())));
    }

    #[test]
    fn extracts_srt_transcript_when_thats_all_thats_offered() {
        let channel = rss::Channel::read_from(FEED_WITH_TRANSCRIPTS.as_bytes()).unwrap();
        let found = extract_transcript(&channel.items[1]);
        assert_eq!(found, Some(("https://example.com/ep2.srt".to_string(), "application/srt".to_string())));
    }

    #[test]
    fn no_transcript_tag_returns_none() {
        let channel = rss::Channel::read_from(FEED_WITH_TRANSCRIPTS.as_bytes()).unwrap();
        assert_eq!(extract_transcript(&channel.items[2]), None);
    }
}

#[cfg(test)]
mod catalog_tests {
    use super::{base_language, rss_categories};

    #[test]
    fn language_tags_fold_to_their_subtag() {
        for tag in ["en", "en-US", "EN-us", "  en-GB  "] {
            assert_eq!(base_language(tag).as_deref(), Some("en"), "tag: {tag}");
        }
        assert_eq!(base_language("de").as_deref(), Some("de"));
    }

    #[test]
    fn a_blank_language_is_none_not_empty() {
        // An empty string here would be stored and then never match anything,
        // which is worse than storing nothing at all.
        assert_eq!(base_language(""), None);
        assert_eq!(base_language("   "), None);
        assert_eq!(base_language("-"), None);
    }

    #[test]
    fn categories_come_back_flattened_and_lowercased() {
        let channel = rss::Channel::read_from(FEED_WITH_CATEGORIES.as_bytes()).unwrap();
        let categories = rss_categories(&channel);

        // Parent and subcategory both kept, both lowercased.
        assert!(categories.contains(&"personal journals".to_string()));
        assert!(categories.contains(&"news".to_string()));
    }

    #[test]
    fn a_joined_itunes_category_is_split_the_way_the_catalogue_splits_it() {
        // Podcast Index stores "Society & Culture" as two categories. A feed
        // enriched from its own RSS has to land on the same vocabulary or it
        // matches nothing the catalogue produced.
        let channel = rss::Channel::read_from(FEED_WITH_CATEGORIES.as_bytes()).unwrap();
        let categories = rss_categories(&channel);

        assert!(categories.contains(&"society".to_string()));
        assert!(categories.contains(&"culture".to_string()));
        assert!(!categories.contains(&"society & culture".to_string()));
    }

    #[test]
    fn a_repeated_category_appears_once() {
        let channel = rss::Channel::read_from(FEED_WITH_CATEGORIES.as_bytes()).unwrap();
        let categories = rss_categories(&channel);
        assert_eq!(categories.iter().filter(|c| *c == "news").count(), 1);
    }

    #[test]
    fn a_feed_with_no_itunes_extension_yields_nothing() {
        // The common case for a plain RSS feed, and it must not panic or
        // invent a category — an empty list is the honest answer.
        let channel = rss::Channel::read_from(BARE_FEED.as_bytes()).unwrap();
        assert!(rss_categories(&channel).is_empty());
    }

    const FEED_WITH_CATEGORIES: &str = r#"<?xml version="1.0"?>
<rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
  <channel>
    <title>Test</title><link>https://example.com</link><description>d</description>
    <itunes:category text="Society &amp; Culture">
      <itunes:category text="Personal Journals"/>
    </itunes:category>
    <itunes:category text="News"/>
    <itunes:category text="news"/>
  </channel>
</rss>"#;

    const BARE_FEED: &str = r#"<?xml version="1.0"?>
<rss version="2.0">
  <channel><title>Test</title><link>https://example.com</link><description>d</description></channel>
</rss>"#;
}
