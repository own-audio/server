// SPDX-License-Identifier: AGPL-3.0-or-later
//! Podcasts followed by their feed, and the directory to find them in
//! (docs/api-v2-completion-plan.md, phase 2): follow and unfollow, refresh,
//! keeping episodes on the server, what is still unplayed, an episode's
//! transcript as timed lines, and the directory — search, categories, a
//! category's shows, shows like one, and a look at a show before following it.
//!
//! The work is v1's (`crate::podcasts`), called rather than copied. What v2
//! changes: refreshing and storing are queued (`202`) and their results reach
//! clients through the change feed; a directory this server does not offer is
//! `501 feature_unavailable` (modules `podcast_search`, `podcast_discovery`),
//! and a directory that does not answer is `502`, never an empty success.

use crate::app::AppState;
use crate::db::{self, catalog};
use crate::families::FamilyContext;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use crate::v2::spoken::{self, Episode, Podcast};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

/// The most a directory list holds in one answer.
const DIRECTORY_MAX: usize = 100;
const DIRECTORY_DEFAULT: usize = 25;

fn directory_down(e: anyhow::Error) -> Problem {
    tracing::warn!("podcast directory: {e:#}");
    Problem::new(StatusCode::BAD_GATEWAY, "directory_unavailable").detail("the podcast directory did not answer; try again later")
}

fn uploaded() -> Problem {
    Problem::new(StatusCode::CONFLICT, "uploaded_podcast").detail("its episodes are uploaded by its owner, not fetched from a feed")
}

fn retired() -> Problem {
    Problem::new(StatusCode::CONFLICT, "youtube_not_supported").detail("a YouTube channel followed before this server stopped fetching YouTube")
}

/// Fetching from the publisher means nothing for a show the server does not fetch.
fn fetched(row: &catalog::PodcastRow) -> Result<(), Problem> {
    match row.source.as_str() {
        "upload" => Err(uploaded()),
        "youtube" => Err(retired()),
        _ => Ok(()),
    }
}

/// The show's owner, or a household admin when it is shared with the household.
fn may_manage(family: &FamilyContext, row: &catalog::PodcastRow) -> Result<(), Problem> {
    if row.user_id == family.user_id || (family.is_family_admin() && row.family_id == Some(family.family_id)) {
        Ok(())
    } else {
        Err(Problem::forbidden().detail("only its owner or a household admin may"))
    }
}

async fn visible(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<catalog::PodcastRow, Problem> {
    catalog::find_podcast(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)
}

// ── Following ────────────────────────────────────────────────────────────────

/// A feed address as the caller sent it: absolute `http`/`https`, not YouTube.
pub(crate) fn check_feed_url(raw: &str) -> Result<String, Problem> {
    let url = raw.trim();
    let invalid = |detail: &str| Problem::bad_request("invalid_feed_url", detail).field("/feed_url", "invalid", detail);
    let parsed = url::Url::parse(url).map_err(|_| invalid("not an absolute address"))?;
    if !matches!(parsed.scheme(), "http" | "https") || parsed.host_str().is_none() {
        return Err(invalid("an http or https address"));
    }
    if crate::podcasts::is_youtube_address(url) {
        return Err(Problem::bad_request("youtube_not_supported", "YouTube channels are not podcast feeds; this server does not download from YouTube")
            .field("/feed_url", "youtube_not_supported", ""));
    }
    Ok(url.to_string())
}

/// What went wrong reading a feed: the address refused (private, not http), or the feed
/// itself (unreachable, not a feed) — the caller's to fix either way; anything else is ours.
fn feed_failure(e: anyhow::Error) -> Problem {
    if crate::http::outbound::is_refused(&e) {
        return Problem::bad_request("invalid_feed_url", e.to_string()).field("/feed_url", "refused", e.to_string());
    }
    let text = format!("{e:#}");
    if ["fetch feed", "read feed body", "parse feed"].iter().any(|p| text.starts_with(p)) {
        return Problem::new(StatusCode::UNPROCESSABLE_ENTITY, "feed_unreadable").detail(text).field("/feed_url", "unreadable", "");
    }
    Problem::internal(e)
}

/// `POST /podcasts` with `feed_url`: the feed, followed. Called by `uploads::create_podcast`,
/// which also makes shows whose episodes are uploaded.
pub(crate) async fn follow(state: &AppState, family: &FamilyContext, feed_url: &str, visibility: Option<&str>) -> Result<(StatusCode, Json<Podcast>), Problem> {
    let url = check_feed_url(feed_url)?;
    let family_id = crate::db::access::family_id_for_visibility(visibility, family.family_id)
        .map_err(|e| Problem::bad_request("invalid_visibility", e.to_string()).field("/visibility", "invalid", ""))?;
    let viewer = family.viewer();
    // Already here (the caller's, or shared with them): that one, without fetching anything.
    if let Some(id) = db::podcasts::visible_feed_by_url(state.db(), viewer, &url).await? {
        return Ok((StatusCode::OK, Json(spoken::podcast_by_id(state, family, id).await?)));
    }
    let (feed, created) = crate::podcasts::subscribe_rss_feed(state, family.user_id, family_id, &url, viewer).await.map_err(feed_failure)?;
    let status = if created { StatusCode::CREATED } else { StatusCode::OK };
    Ok((status, Json(spoken::podcast_by_id(state, family, feed.id).await?)))
}

/// Stops following a show: the show and its episodes leave the library (and the change feed
/// sends their removal). Its owner, or a household admin for a show shared with the household.
/// An uploaded show, which nobody follows, goes to the trash with its episodes for 30 days
/// instead (kind `podcast` in `/trash`), restorable until then.
#[utoipa::path(delete, path = "/podcasts/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Unfollowed, or an uploaded show in the trash"),
        (status = 403, description = "Visible, but neither the owner's nor shared with an admin caller", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`uploaded_podcast`: answered by servers before v2 revision 13, which kept no shows in the trash", body = crate::v2::problem::ProblemBody)))]
pub async fn unfollow(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    let row = visible(&state, &family, id).await?;
    may_manage(&family, &row)?;
    if row.source == "upload" {
        crate::trash::move_to_trash_in(&state, family.viewer(), Uuid::new_v4(), crate::db::trash::Kind::Podcast, id).await?;
        return Ok(StatusCode::NO_CONTENT);
    }
    if !db::podcasts::unfollow(state.db(), id).await? {
        return Err(Problem::not_found());
    }
    // v1's file-sync clients learn it from their own feed of removals.
    let _ = db::sync::record_deletion(state.db(), crate::db::access::PODCAST, id, row.user_id, row.family_id).await;
    Ok(StatusCode::NO_CONTENT)
}

// ── Refreshing ───────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Queued {
    /// How many shows (or episodes) are queued; their results arrive through `/sync/changes`
    /// and the `library` event.
    pub queued: i64,
}

/// Reads the show's feed again: new episodes, and the show's and episodes' art kept on this
/// server. Queued; the new episodes arrive through `/sync/changes`.
#[utoipa::path(post, path = "/podcasts/{id}/refresh", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 202, description = "Queued", body = Queued),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`uploaded_podcast` (nothing to fetch) or `youtube_not_supported`", body = crate::v2::problem::ProblemBody)))]
pub async fn refresh(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<(StatusCode, Json<Queued>), Problem> {
    let row = visible(&state, &family, id).await?;
    fetched(&row)?;
    db::jobs::enqueue_feed_refresh(state.db(), id, true).await?;
    crate::jobs::worker::wake();
    Ok((StatusCode::ACCEPTED, Json(Queued { queued: 1 })))
}

/// Reads every followed show's feed again (the caller's, and those shared with them).
#[utoipa::path(post, path = "/podcasts/refresh", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 202, description = "Queued", body = Queued)))]
pub async fn refresh_all(family: FamilyContext, State(state): State<AppState>) -> Result<(StatusCode, Json<Queued>), Problem> {
    // Bounded by the household: the shows it follows, not a catalogue.
    let feeds = db::podcasts::list_feeds(state.db(), family.viewer()).await?;
    let mut queued = 0;
    for feed in feeds.iter().filter(|f| f.source_type == "rss") {
        db::jobs::enqueue_feed_refresh(state.db(), feed.id, false).await?;
        queued += 1;
    }
    crate::jobs::worker::wake();
    Ok((StatusCode::ACCEPTED, Json(Queued { queued })))
}

// ── Keeping episodes on the server ───────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct PodcastPatch {
    /// The server stores each episode published from now on by itself, so the household keeps
    /// it even when the feed drops it (paid feeds whose links expire). No back catalogue: that
    /// is `POST /podcasts/{id}/store`.
    pub auto_store: Option<bool>,
}

/// Changes a followed show's settings. Its owner, or a household admin for a shared show;
/// either may add to the library.
#[utoipa::path(patch, path = "/podcasts/{id}", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    request_body = PodcastPatch,
    responses((status = 200, body = Podcast),
        (status = 403, description = "Not the owner (or an admin of a shared show), or may not add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`uploaded_podcast` or `youtube_not_supported`: nothing is fetched to store", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_podcast(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<PodcastPatch>) -> Result<Json<Podcast>, Problem> {
    let row = visible(&state, &family, id).await?;
    may_manage(&family, &row)?;
    family.require_can_upload()?;
    if let Some(on) = body.auto_store {
        fetched(&row)?;
        db::podcasts::set_auto_store(state.db(), id, on).await?;
    }
    Ok(Json(spoken::podcast_by_id(&state, &family, id).await?))
}

#[derive(Deserialize, ToSchema, Default)]
pub struct StoreRequest {
    /// Only the newest this many episodes not stored yet; all of them when absent.
    pub latest: Option<i64>,
    /// Count only: how many episodes and roughly how many bytes, nothing queued.
    #[serde(default)]
    pub preview: bool,
}

#[derive(Serialize, ToSchema)]
pub struct StoreAnswer {
    /// Episodes queued, or that would be for a preview.
    pub episodes: i64,
    /// A rough size from the episodes' durations at 128 kbit/s; feeds rarely state sizes.
    pub estimated_bytes: i64,
}

/// Keeps a show's back catalogue on the server (all of it, or the newest `latest` episodes),
/// so a paid feed stays in the household's library after the subscription ends. An episode
/// queued once is not queued again, and one whose copy was deleted is not stored again. Each
/// stored episode arrives through `/sync/changes` with `stored: true`.
#[utoipa::path(post, path = "/podcasts/{id}/store", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    request_body = StoreRequest,
    responses((status = 202, description = "Queued", body = StoreAnswer),
        (status = 200, description = "`preview`: what would be queued", body = StoreAnswer),
        (status = 403, description = "Not the owner (or an admin of a shared show), or may not add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`uploaded_podcast` or `youtube_not_supported`", body = crate::v2::problem::ProblemBody)))]
pub async fn store(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, body: Option<Json<StoreRequest>>) -> Result<(StatusCode, Json<StoreAnswer>), Problem> {
    let body = body.map(|Json(b)| b).unwrap_or_default();
    let row = visible(&state, &family, id).await?;
    may_manage(&family, &row)?;
    family.require_can_upload()?;
    fetched(&row)?;
    if body.latest.is_some_and(|n| n < 1) {
        return Err(Problem::bad_request("invalid_latest", "latest is at least 1").field("/latest", "out_of_range", "1 or more"));
    }
    let (episodes, estimated_bytes) = crate::podcasts::store_back_catalogue(state.db(), id, body.latest, body.preview).await?;
    if body.preview {
        return Ok((StatusCode::OK, Json(StoreAnswer { episodes, estimated_bytes })));
    }
    crate::jobs::worker::wake();
    Ok((StatusCode::ACCEPTED, Json(StoreAnswer { episodes, estimated_bytes })))
}

/// Keeps this episode on the server: fetched from the publisher and stored, queued. Answers
/// the episode as it is now (`stored` turns true when the copy is in; `/sync/changes` says so).
/// Anyone who may add to the library and can see the show.
#[utoipa::path(put, path = "/episodes/{id}/stored", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    params(("id" = Uuid, Path)),
    responses((status = 202, description = "Queued", body = Episode),
        (status = 200, description = "Stored already", body = Episode),
        (status = 402, description = "`storage_refused`: the household is out of room", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "May not add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`no_audio`: the feed gives no file for it; `youtube_not_supported`", body = crate::v2::problem::ProblemBody)))]
pub async fn put_stored(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<(StatusCode, Json<Episode>), Problem> {
    let row = catalog::find_episode(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    family.require_can_upload()?;
    if row.stored {
        return Ok((StatusCode::OK, Json(spoken::episode(&state, row))));
    }
    let show = visible(&state, &family, row.podcast_id).await?;
    fetched(&show)?;
    if row.audio_url.is_none() {
        return Err(Problem::new(StatusCode::CONFLICT, "no_audio").detail("the feed gives no file for this episode"));
    }
    crate::storage::quota::Guard::for_state(&state)
        .check(state.db(), state.storage().bucket(), family.family_id, 0)
        .await
        .map_err(crate::storage::quota::to_auth_error)?;
    let payload = serde_json::json!({ "episode_id": id.to_string(), "asked": true });
    db::jobs::enqueue_once(state.db(), "episode_download", "episode_id", &id.to_string(), &payload).await?;
    crate::jobs::worker::wake();
    Ok((StatusCode::ACCEPTED, Json(spoken::episode(&state, row))))
}

/// Drops the server's copy of this episode: into the household's trash for 30 days. The
/// episode stays listed, playable from the publisher. Its show's owner, or a household admin
/// for a shared show; an episode not stored answers as it is.
#[utoipa::path(delete, path = "/episodes/{id}/stored", tag = "library", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, description = "The episode, no longer stored", body = Episode),
        (status = 403, description = "Visible, but neither the owner nor a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_stored(family: FamilyContext, State(state): State<AppState>, headers: HeaderMap, Path(id): Path<Uuid>) -> Result<Json<Episode>, Problem> {
    let row = catalog::find_episode(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    if row.stored {
        crate::trash::move_to_trash(&state, family.viewer(), Some(&headers), crate::db::trash::Kind::PodcastEpisode, id).await?;
    }
    Ok(Json(spoken::episode_by_id(&state, &family, id).await?))
}

// ── Unplayed ─────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Unplayed {
    pub podcast_id: Uuid,
    /// Episodes the caller has not finished.
    pub unplayed: i64,
}

/// How many episodes of each show the caller has not finished, every visible show once (zero
/// included), in id order. Per listener: the shows are the household's, this is yours.
#[utoipa::path(get, path = "/me/podcasts/unplayed", tag = "me", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<Unplayed>)))]
pub async fn unplayed(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<Unplayed>>, Problem> {
    let limit = page.limit()?;
    let rows = db::podcasts::unplayed_page(state.db(), family.viewer(), page.after::<Uuid>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |&(id, _)| id);
    Ok(Json(Page {
        items: page.items.into_iter().map(|(podcast_id, unplayed)| Unplayed { podcast_id, unplayed }).collect(),
        next: page.next,
        included: None,
    }))
}

// ── Transcript ───────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Segment {
    pub start_ms: i64,
    pub end_ms: i64,
    /// Who speaks, when the file says (WebVTT voices, Podcast Index JSON).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
}

#[derive(Serialize, ToSchema)]
pub struct Transcript {
    /// `segments` (timed lines, from WebVTT, SRT or Podcast Index JSON) or `text` (a file
    /// without timing: `segments` is empty and `text` holds it).
    pub format: String,
    pub segments: Vec<Segment>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

pub(crate) fn transcript_of(raw: &str, mime: &str) -> Transcript {
    let (timed, text) = crate::podcasts::transcript::segments(raw, mime);
    Transcript {
        format: if timed.is_empty() { "text" } else { "segments" }.into(),
        segments: timed.into_iter().map(|s| Segment { start_ms: s.start_ms, end_ms: s.end_ms, speaker: s.speaker, text: s.text }).collect(),
        text,
    }
}

/// The episode's transcript to read along: the publisher's `<podcast:transcript>` (read by the
/// server, so the client needs no access to the publisher) or the file uploaded with the
/// episode, as timed lines.
#[utoipa::path(get, path = "/episodes/{id}/transcript", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Transcript),
        (status = 404, description = "Unknown or not visible; `no_transcript`: the episode has none", body = crate::v2::problem::ProblemBody),
        (status = 422, description = "`transcript_too_large`: over 4 MiB, not a transcript", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`transcript_unavailable`: the publisher's file could not be read", body = crate::v2::problem::ProblemBody)))]
pub async fn transcript(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Transcript>, Problem> {
    use crate::podcasts::transcript::MAX_BYTES;
    let row = catalog::find_episode(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let mime = row.transcript_type.clone().unwrap_or_default();
    let unavailable = |e: anyhow::Error| {
        tracing::warn!(episode = %id, "transcript: {e:#}");
        Problem::new(StatusCode::BAD_GATEWAY, "transcript_unavailable").detail("the publisher's transcript could not be read")
    };
    let raw = if let Some(object_id) = row.transcript_object_id {
        let key = db::subsonic::media_object_key(state.db(), object_id).await?.ok_or_else(Problem::not_found)?;
        state.storage().get(&key).await?
    } else if let Some(url) = row.transcript_url.as_deref() {
        let response = crate::http::outbound::get(url).await.map_err(unavailable)?;
        crate::http::outbound::bytes(response, MAX_BYTES + 1).await.map_err(unavailable)?
    } else {
        return Err(Problem::new(StatusCode::NOT_FOUND, "no_transcript").detail("this episode has no transcript"));
    };
    if raw.len() as u64 > MAX_BYTES {
        return Err(Problem::new(StatusCode::UNPROCESSABLE_ENTITY, "transcript_too_large").detail("over 4 MiB; not a transcript"));
    }
    Ok(Json(transcript_of(&String::from_utf8_lossy(&raw), &mime)))
}

// ── The directory ────────────────────────────────────────────────────────────

/// A show in the directory, before or after anyone here follows it.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct DirectoryShow {
    /// What `POST /podcasts` takes to follow it.
    pub feed_url: String,
    pub title: String,
    pub author: Option<String>,
    pub description: Option<String>,
    pub link: Option<String>,
    /// The publisher's art.
    pub image: Option<String>,
    pub language: Option<String>,
    /// Unset when the directory does not know.
    pub episode_count: Option<i32>,
    /// Lowercased; may be empty.
    pub categories: Vec<String>,
    /// The show on this server, when the caller follows it or sees it shared.
    pub podcast_id: Option<Uuid>,
}

impl From<crate::metadata::PodcastCatalogEntry> for DirectoryShow {
    fn from(e: crate::metadata::PodcastCatalogEntry) -> Self {
        DirectoryShow {
            feed_url: e.url,
            title: e.title,
            author: e.author,
            description: e.description,
            link: e.link,
            image: e.image_url,
            language: e.language,
            // Zero is the catalogue's "not known".
            episode_count: (e.episode_count > 0).then_some(e.episode_count),
            categories: e.categories,
            podcast_id: None,
        }
    }
}

impl From<crate::metadata::itunes::Show> for DirectoryShow {
    fn from(s: crate::metadata::itunes::Show) -> Self {
        DirectoryShow {
            feed_url: s.feed_url,
            title: s.title,
            author: s.author,
            description: None,
            link: s.link,
            image: s.image_url,
            language: None,
            episode_count: s.episode_count,
            categories: s.categories,
            podcast_id: None,
        }
    }
}

/// `limit`, 1 to 100; 25 when absent.
fn directory_limit(limit: Option<i64>) -> Result<usize, Problem> {
    match limit {
        None => Ok(DIRECTORY_DEFAULT),
        Some(n) if (1..=DIRECTORY_MAX as i64).contains(&n) => Ok(n as usize),
        Some(_) => Err(Problem::bad_request("invalid_limit", format!("limit is 1 to {DIRECTORY_MAX}")).field("limit", "out_of_range", format!("1 to {DIRECTORY_MAX}"))),
    }
}

/// Discovery's `podcast_discovery`: the catalogue's categories, browsing and similar shows.
fn catalogue(state: &AppState) -> Result<&crate::app::config::MetadataConfig, Problem> {
    state.config().metadata.as_ref().ok_or_else(|| {
        crate::v2::account::unavailable("podcast_discovery", "this server has no podcast catalogue (the metadata service)")
    })
}

/// Marks the shows the caller already has (`podcast_id`).
async fn mark_followed(state: &AppState, family: &FamilyContext, shows: &mut [DirectoryShow]) -> Result<(), Problem> {
    let urls: Vec<String> = shows.iter().map(|s| s.feed_url.clone()).collect();
    let here = db::podcasts::visible_feeds_by_urls(state.db(), family.viewer(), &urls).await?;
    for show in shows {
        show.podcast_id = here.get(&show.feed_url).copied();
    }
    Ok(())
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct DirectorySearchQuery {
    /// Two characters or more.
    pub q: String,
    /// A language subtag (`en`) for this search only; without it, the caller's
    /// `discovery_languages` (`GET /me`), and every language when those are empty.
    pub language: Option<String>,
    /// 1 to 100; 25 when absent.
    pub limit: Option<i64>,
}

/// Shows matching `q`, best first: the server's podcast catalogue, or Apple's public search
/// where there is none (discovery: `modules.podcast_search`). What the caller follows already
/// carries its `podcast_id`. One page; the search answers its best and no more.
#[utoipa::path(get, path = "/podcasts/directory/search", tag = "directory", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(DirectorySearchQuery),
    responses((status = 200, body = Page<DirectoryShow>),
        (status = 400, description = "`invalid_query`: `q` under 2 characters; `invalid_limit`", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable`: no directory to search here", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`directory_unavailable`: the directory did not answer", body = crate::v2::problem::ProblemBody)))]
pub async fn directory_search(family: FamilyContext, State(state): State<AppState>, Query(q): Query<DirectorySearchQuery>) -> Result<Json<Page<DirectoryShow>>, Problem> {
    let query = q.q.trim();
    if query.chars().count() < 2 {
        return Err(Problem::bad_request("invalid_query", "search for two characters or more").field("q", "too_short", "2 characters or more"));
    }
    let limit = directory_limit(q.limit)?;
    let mut shows: Vec<DirectoryShow> = match state.config().metadata.as_ref() {
        Some(config) => {
            let languages = crate::podcasts::discovery_languages(&state, family.user_id, q.language.as_deref()).await?;
            crate::podcasts::search_catalogue(config, query, &languages, limit).await.map_err(directory_down)?.into_iter().map(Into::into).collect()
        }
        None if state.config().itunes.enabled => {
            crate::metadata::itunes::search(query, limit).await.map_err(directory_down)?.into_iter().map(Into::into).collect()
        }
        None => return Err(crate::v2::account::unavailable("podcast_search", "this server has no podcast directory to search")),
    };
    mark_followed(&state, &family, &mut shows).await?;
    Ok(Json(Page { items: shows, next: None, included: None }))
}

/// A catalogue category and how many shows sit in it.
#[derive(Serialize, ToSchema)]
pub struct DirectoryCategory {
    pub category: String,
    pub feed_count: i64,
    /// Shows that published within the last year: the number worth showing.
    pub active_count: i64,
}

/// The catalogue's categories (discovery: `modules.podcast_discovery`). One page: the
/// catalogue's taxonomy, not a list that grows.
#[utoipa::path(get, path = "/podcasts/directory/categories", tag = "directory", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    responses((status = 200, body = Page<DirectoryCategory>),
        (status = 501, description = "`feature_unavailable`: no catalogue here", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`directory_unavailable`", body = crate::v2::problem::ProblemBody)))]
pub async fn directory_categories(_family: FamilyContext, State(state): State<AppState>) -> Result<Json<Page<DirectoryCategory>>, Problem> {
    let config = catalogue(&state)?;
    let categories = crate::metadata::mirror::MetadataMirror::new(config).podcast_categories().await.map_err(directory_down)?;
    let items = categories
        .into_iter()
        .map(|c| DirectoryCategory { category: c.category, feed_count: c.feed_count, active_count: c.active_count })
        .collect();
    Ok(Json(Page { items, next: None, included: None }))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BrowseQuery {
    /// From `GET /podcasts/directory/categories`.
    pub category: String,
    /// As on search.
    pub language: Option<String>,
    /// 1 to 100; 25 when absent.
    pub limit: Option<i64>,
    /// `next` from the previous page.
    pub cursor: Option<String>,
}

/// The best shows in one category, in the caller's languages, without those the household
/// follows already (discovery: `modules.podcast_discovery`). A page can hold fewer than
/// `limit` for that reason and still have a `next`.
#[utoipa::path(get, path = "/podcasts/directory", tag = "directory", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(BrowseQuery),
    responses((status = 200, body = Page<DirectoryShow>),
        (status = 400, description = "`invalid_category`, `invalid_limit`, `invalid_cursor`", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable`: no catalogue here", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`directory_unavailable`", body = crate::v2::problem::ProblemBody)))]
pub async fn directory_browse(family: FamilyContext, State(state): State<AppState>, Query(q): Query<BrowseQuery>) -> Result<Json<Page<DirectoryShow>>, Problem> {
    let category = q.category.trim();
    if category.is_empty() {
        return Err(Problem::bad_request("invalid_category", "name a category").field("category", "required", ""));
    }
    let limit = directory_limit(q.limit)?;
    let offset: u32 = PageQuery { limit: None, cursor: q.cursor.clone() }.after::<u32>()?.unwrap_or(0);
    let config = catalogue(&state)?;
    let languages = crate::podcasts::discovery_languages(&state, family.user_id, q.language.as_deref()).await?;
    let candidates = crate::podcasts::browse_catalogue(config, category, &languages, limit, offset).await.map_err(directory_down)?;
    let full = candidates.len() >= limit;
    let known = db::podcasts::subscribed_catalog_ids(state.db(), family.viewer()).await?;
    let items = candidates.into_iter().filter(|e| !known.contains(&e.id)).map(Into::into).collect();
    let next = full.then(|| crate::v2::page::encode(&(offset + limit as u32)));
    Ok(Json(Page { items, next, included: None }))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SimilarQuery {
    /// A show here.
    pub podcast_id: Option<Uuid>,
    /// Or a show not followed yet, by its feed.
    pub feed_url: Option<String>,
    /// 1 to 50; 20 when absent.
    pub limit: Option<i64>,
}

/// Shows like one (a show here by `podcast_id`, or any by `feed_url`), in the caller's
/// languages, without those the household follows already (discovery:
/// `modules.podcast_discovery`). Empty when the catalogue does not know the show: it is a
/// weekly snapshot. The catalogue is asked about the show, never about who asks.
#[utoipa::path(get, path = "/podcasts/directory/similar", tag = "directory", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(SimilarQuery),
    responses((status = 200, body = Page<DirectoryShow>),
        (status = 400, description = "`invalid_query`: neither `podcast_id` nor `feed_url`; `invalid_limit`", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "`podcast_id` unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 501, description = "`feature_unavailable`: no catalogue here", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`directory_unavailable`", body = crate::v2::problem::ProblemBody)))]
pub async fn directory_similar(family: FamilyContext, State(state): State<AppState>, Query(q): Query<SimilarQuery>) -> Result<Json<Page<DirectoryShow>>, Problem> {
    let limit = match q.limit {
        None => 20,
        Some(n) if (1..=50).contains(&n) => n as usize,
        Some(_) => return Err(Problem::bad_request("invalid_limit", "limit is 1 to 50").field("limit", "out_of_range", "1 to 50")),
    };
    let feed_url = q.feed_url.as_deref().map(str::trim).filter(|u| !u.is_empty());
    if q.podcast_id.is_none() && feed_url.is_none() {
        return Err(Problem::bad_request("invalid_query", "name a show: podcast_id or feed_url").field("podcast_id", "required", "or feed_url"));
    }
    let seed = match q.podcast_id {
        Some(id) => Some(db::podcasts::find_feed(state.db(), id, family.viewer()).await?.ok_or_else(Problem::not_found)?),
        None => None,
    };
    let config = catalogue(&state)?;
    let mirror = crate::metadata::mirror::MetadataMirror::new(config);
    let catalog_id = match (seed, feed_url) {
        (Some(feed), _) => match feed.catalog_id {
            Some(id) => Some(id),
            None => mirror.podcast_lookup(Some(&feed.feed_url), None, None).await.map_err(directory_down)?.map(|e| e.id),
        },
        (None, Some(url)) => mirror.podcast_lookup(Some(url), None, None).await.map_err(directory_down)?.map(|e| e.id),
        (None, None) => None,
    };
    let Some(catalog_id) = catalog_id else {
        return Ok(Json(Page { items: Vec::new(), next: None, included: None }));
    };
    // With no languages set the catalogue keeps to the show's own language; set, they replace it.
    let languages = crate::podcasts::discovery_languages(&state, family.user_id, None).await?;
    let candidates = mirror
        .podcast_similar(catalog_id, crate::podcasts::language_param(&languages).as_deref(), limit)
        .await
        .map_err(directory_down)?;
    let known = db::podcasts::subscribed_catalog_ids(state.db(), family.viewer()).await?;
    let items = candidates.into_iter().filter(|e| e.id != catalog_id && !known.contains(&e.id)).map(Into::into).collect();
    Ok(Json(Page { items, next: None, included: None }))
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PreviewQuery {
    pub feed_url: String,
    /// Newest episodes, 1 to 50; 12 when absent.
    pub limit: Option<i64>,
}

/// An episode in a show's feed, read but not stored.
#[derive(Serialize, ToSchema, Debug, PartialEq)]
pub struct DirectoryEpisode {
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    /// The publisher's file.
    pub audio_url: Option<String>,
    pub image: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct DirectoryPreview {
    pub show: DirectoryShow,
    /// Newest first.
    pub episodes: Vec<DirectoryEpisode>,
}

/// The show and newest episodes as its feed says now, nothing stored: for deciding whether to
/// follow it. Reads the feed itself, so it works on any server, catalogue or not; a big feed
/// takes as long as following it would.
pub(crate) fn preview_of(feed_url: &str, channel: &rss::Channel, limit: usize) -> DirectoryPreview {
    let image = channel.itunes_ext().and_then(|e| e.image()).map(str::to_string).or_else(|| channel.image().map(|i| i.url().to_string()));
    let mut categories: Vec<String> = Vec::new();
    for c in channel.itunes_ext().map(|e| e.categories()).unwrap_or_default() {
        let name = c.text().trim().to_ascii_lowercase();
        if !name.is_empty() && !categories.contains(&name) {
            categories.push(name);
        }
    }
    let show = DirectoryShow {
        feed_url: feed_url.to_string(),
        title: channel.title().trim().to_string(),
        author: channel.itunes_ext().and_then(|e| e.author()).map(str::to_string),
        description: Some(channel.description().to_string()).filter(|d| !d.is_empty()),
        link: Some(channel.link().to_string()).filter(|l| !l.is_empty()),
        image,
        language: channel.language().map(str::to_string),
        episode_count: i32::try_from(channel.items().len()).ok(),
        categories,
        podcast_id: None,
    };
    let mut episodes: Vec<DirectoryEpisode> = channel
        .items()
        .iter()
        .map(|item| DirectoryEpisode {
            guid: item
                .guid()
                .map(|g| g.value().to_string())
                .or_else(|| item.link().map(str::to_string))
                .unwrap_or_else(|| item.title().unwrap_or_default().to_string()),
            title: item.title().unwrap_or("Untitled").to_string(),
            description: item.description().map(str::to_string),
            published_at: item.pub_date().and_then(|s| DateTime::parse_from_rfc2822(s).ok()).map(|d| d.with_timezone(&Utc)),
            duration_ms: item.itunes_ext().and_then(|e| e.duration()).and_then(crate::podcasts::parse_duration).map(|s| i64::from(s) * 1000),
            audio_url: item.enclosure().map(|e| e.url().to_string()),
            image: item.itunes_ext().and_then(|e| e.image()).map(str::to_string),
        })
        .collect();
    // Undated items last: most feeds are newest-first, enough are not.
    episodes.sort_by(|a, b| match (a.published_at, b.published_at) {
        (Some(l), Some(r)) => r.cmp(&l),
        (Some(_), None) => std::cmp::Ordering::Less,
        (None, Some(_)) => std::cmp::Ordering::Greater,
        (None, None) => std::cmp::Ordering::Equal,
    });
    episodes.truncate(limit);
    DirectoryPreview { show, episodes }
}

/// A show not followed yet, with its newest episodes, read from its feed now; nothing is
/// stored. When the caller has it already, `show.podcast_id` says which.
#[utoipa::path(get, path = "/podcasts/directory/preview", tag = "directory", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PreviewQuery),
    responses((status = 200, body = DirectoryPreview),
        (status = 400, description = "`invalid_feed_url` (also an address this server will not fetch); `invalid_limit`", body = crate::v2::problem::ProblemBody),
        (status = 422, description = "`feed_unreadable`: not reachable, or not a feed", body = crate::v2::problem::ProblemBody)))]
pub async fn directory_preview(family: FamilyContext, State(state): State<AppState>, Query(q): Query<PreviewQuery>) -> Result<Json<DirectoryPreview>, Problem> {
    let limit = match q.limit {
        None => 12,
        Some(n) if (1..=50).contains(&n) => n as usize,
        Some(_) => return Err(Problem::bad_request("invalid_limit", "limit is 1 to 50").field("limit", "out_of_range", "1 to 50")),
    };
    let url = check_feed_url(&q.feed_url)?;
    let channel = crate::podcasts::fetch_rss_url(&url).await.map_err(feed_failure)?;
    let mut preview = preview_of(&url, &channel, limit);
    preview.show.podcast_id = db::podcasts::visible_feed_by_url(state.db(), family.viewer(), &url).await?;
    Ok(Json(preview))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn feed_addresses_are_checked_before_anything_is_fetched() {
        assert_eq!(check_feed_url("  https://example.com/feed.xml ").unwrap(), "https://example.com/feed.xml");
        for (raw, code) in [
            ("example.com/feed", "invalid_feed_url"),
            ("ftp://example.com/feed", "invalid_feed_url"),
            ("file:///etc/passwd", "invalid_feed_url"),
            ("https://www.youtube.com/@channel", "youtube_not_supported"),
            ("https://youtu.be/abc", "youtube_not_supported"),
        ] {
            assert_eq!(check_feed_url(raw).unwrap_err().code(), code, "{raw}");
        }
    }

    #[test]
    fn a_feed_that_cannot_be_read_is_the_callers_to_fix() {
        let unreadable = feed_failure(anyhow::anyhow!("parse feed: not xml"));
        assert_eq!((unreadable.status(), unreadable.code()), (StatusCode::UNPROCESSABLE_ENTITY, "feed_unreadable"));
        let fetch = feed_failure(anyhow::anyhow!("connection refused").context("fetch feed"));
        assert_eq!(fetch.code(), "feed_unreadable");
        assert_eq!(feed_failure(anyhow::anyhow!("db: insert feed")).status(), StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[test]
    fn directory_limits() {
        assert_eq!(directory_limit(None).unwrap(), 25);
        assert_eq!(directory_limit(Some(100)).unwrap(), 100);
        assert_eq!(directory_limit(Some(0)).unwrap_err().code(), "invalid_limit");
        assert_eq!(directory_limit(Some(101)).unwrap_err().code(), "invalid_limit");
    }

    #[test]
    fn a_preview_reads_the_feed_newest_first() {
        let xml = r#"<?xml version="1.0"?><rss version="2.0" xmlns:itunes="http://www.itunes.com/dtds/podcast-1.0.dtd">
            <channel><title> A Show </title><link>https://example.com</link><description>About</description><language>cs</language>
            <itunes:author>Anna</itunes:author><itunes:image href="https://example.com/art.jpg"/><itunes:category text="Society &amp; Culture"/>
            <item><title>Old</title><guid>g1</guid><pubDate>Mon, 01 Jan 2024 10:00:00 GMT</pubDate><enclosure url="https://example.com/1.mp3" type="audio/mpeg" length="1"/><itunes:duration>1:02</itunes:duration></item>
            <item><title>Undated</title><guid>g0</guid></item>
            <item><title>New</title><guid>g2</guid><pubDate>Tue, 02 Jan 2024 10:00:00 GMT</pubDate></item>
            </channel></rss>"#;
        let channel = rss::Channel::read_from(xml.as_bytes()).unwrap();
        let p = preview_of("https://example.com/feed", &channel, 2);
        assert_eq!(p.show.title, "A Show");
        assert_eq!(p.show.author.as_deref(), Some("Anna"));
        assert_eq!(p.show.image.as_deref(), Some("https://example.com/art.jpg"));
        assert_eq!(p.show.episode_count, Some(3));
        assert_eq!(p.show.categories, vec!["society & culture".to_string()]);
        assert_eq!(p.episodes.iter().map(|e| e.title.as_str()).collect::<Vec<_>>(), vec!["New", "Old"]);
        assert_eq!(p.episodes[1].duration_ms, Some(62_000));
        assert_eq!(p.episodes[1].audio_url.as_deref(), Some("https://example.com/1.mp3"));
    }

    #[test]
    fn transcripts_answer_segments_or_text() {
        let t = transcript_of("WEBVTT\n\n00:00:01.000 --> 00:00:02.500\n<v Host>Hello\n", "text/vtt");
        assert_eq!(t.format, "segments");
        assert_eq!((t.segments[0].start_ms, t.segments[0].end_ms, t.segments[0].speaker.as_deref()), (1000, 2500, Some("Host")));
        let plain = transcript_of("Just words.", "text/plain");
        assert_eq!((plain.format.as_str(), plain.segments.len(), plain.text.as_deref()), ("text", 0, Some("Just words.")));
    }

    #[test]
    fn catalogue_entries_become_directory_shows() {
        let entry: crate::metadata::PodcastCatalogEntry =
            serde_json::from_value(serde_json::json!({ "id": 7, "url": "https://example.com/f", "title": "T", "episode_count": 0, "image_url": "https://example.com/i.jpg" })).unwrap();
        let show = DirectoryShow::from(entry);
        assert_eq!((show.episode_count, show.image.as_deref(), show.podcast_id), (None, Some("https://example.com/i.jpg"), None));
    }
}
