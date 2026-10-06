// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::access::{PODCAST, VISIBLE, Viewer};
use crate::podcasts::models::{PodcastEpisode, PodcastFeed};
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

pub(crate) const FEED_COLS: &str =
    "id, user_id, family_id, feed_url, source_type, youtube_channel_id, title, description, author, link, language,
     image_url, image_object_id, last_refreshed_at, refresh_error, created_at, updated_at,
     catalog_id, podcast_guid, itunes_id, categories, popularity_score, language_base, catalog_checked_at,
     auto_store_since";

/// Bind the four leading parameters every [`VISIBLE`] query expects.
macro_rules! bind_viewer {
    ($q:expr, $viewer:expr) => {
        $q.bind($viewer.user_id)
            .bind($viewer.family_id)
            .bind($viewer.is_family_admin)
            .bind(PODCAST)
    };
}

/// Aliased to `e` throughout — every query using this now joins `media_objects` for
/// `size_bytes`/`sha256` (mirror-plan B-2), so the episodes table needs a stable alias to
/// disambiguate from it.
const EPISODE_COLS: &str =
    "e.id, e.feed_id, e.guid, e.title, e.description, e.published_at, e.duration_secs,
     e.episode_number, e.season_number, e.audio_url, e.audio_object_id, e.image_url, e.image_object_id, e.created_at,
     mo.size_bytes, mo.sha256, e.transcript_url, e.transcript_type";

// ── Feeds ─────────────────────────────────────────────────────────────────

pub async fn list_feeds(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<PodcastFeed>> {
    let sql = format!("SELECT {FEED_COLS} FROM podcast_feeds t WHERE {VISIBLE} ORDER BY t.title");

    bind_viewer!(sqlx::query_as::<_, PodcastFeed>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list podcast feeds")
}

pub async fn find_feed(
    pool: &PgPool,
    id: Uuid,
    viewer: Viewer,
) -> anyhow::Result<Option<PodcastFeed>> {
    let sql = format!("SELECT {FEED_COLS} FROM podcast_feeds t WHERE t.id = $5 AND {VISIBLE}");

    bind_viewer!(sqlx::query_as::<_, PodcastFeed>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find podcast feed")
}

/// Which of these feeds publish a transcript for at least one episode — the
/// ones whose episodes can be translated (`podcast_translate`). EXISTS stops at
/// the first such episode, so a big back catalogue costs nothing extra.
pub async fn feeds_with_transcripts(pool: &PgPool, ids: &[Uuid]) -> anyhow::Result<std::collections::HashSet<Uuid>> {
    let rows: Vec<(Uuid,)> = sqlx::query_as(
        "SELECT f FROM unnest($1::uuid[]) AS f
          WHERE EXISTS (SELECT 1 FROM podcast_episodes e
                         WHERE e.feed_id = f AND e.transcript_url IS NOT NULL AND e.trashed_at IS NULL)",
    )
    .bind(ids)
    .fetch_all(pool)
    .await
    .context("db: feeds with transcripts")?;
    Ok(rows.into_iter().map(|(id,)| id).collect())
}

/// How many episodes of each visible feed this user has not finished.
///
/// An episode with no progress row counts as unplayed, which is why this is a LEFT JOIN on a
/// COALESCE rather than a count of rows: most episodes of most shows have never been touched.
pub async fn unplayed_counts(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<(Uuid, i64)>> {
    let sql = format!(
        "SELECT e.feed_id, COUNT(*)
         FROM podcast_episodes e
         JOIN podcast_feeds t ON t.id = e.feed_id
         LEFT JOIN podcast_progress pp ON pp.episode_id = e.id AND pp.user_id = $1
         WHERE {VISIBLE} AND COALESCE(pp.completed, false) = false
         GROUP BY e.feed_id"
    );

    bind_viewer!(sqlx::query_as::<_, (Uuid, i64)>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: unplayed episode counts")
}

/// Owner-scoped lookup for mutations (unsubscribe, refresh, re-share).
pub async fn find_feed_owned(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<PodcastFeed>> {
    sqlx::query_as::<_, PodcastFeed>(&format!(
        "SELECT {FEED_COLS} FROM podcast_feeds WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find owned podcast feed")
}

pub async fn insert_feed(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Option<Uuid>,
    feed_url: &str,
    title: &str,
) -> anyhow::Result<PodcastFeed> {
    sqlx::query_as::<_, PodcastFeed>(&format!(
        "INSERT INTO podcast_feeds (user_id, family_id, feed_url, title)
         VALUES ($1, $2, $3, $4)
         RETURNING {FEED_COLS}"
    ))
    .bind(user_id)
    .bind(family_id)
    .bind(feed_url)
    .bind(title)
    .fetch_one(pool)
    .await
    .context("db: insert podcast feed")
}

/// Record what the Podcast Index catalogue said about a feed.
///
/// `catalog_checked_at` is stamped even when nothing was found, so the backfill
/// job can tell "never asked" from "asked, not in the catalogue" and stop
/// re-asking about the second kind on every pass.
///
/// Never overwrites `categories` with an empty array: a later refresh that
/// fails to reach the catalogue must not erase categories an earlier one — or
/// the feed's own RSS — already supplied.
#[allow(clippy::too_many_arguments)]
pub async fn set_catalog_metadata(
    pool: &PgPool,
    id: Uuid,
    catalog_id: Option<i64>,
    podcast_guid: Option<&str>,
    itunes_id: Option<i64>,
    categories: &[String],
    popularity_score: Option<i32>,
    language_base: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE podcast_feeds SET
            catalog_id         = COALESCE($2, catalog_id),
            podcast_guid       = COALESCE($3, podcast_guid),
            itunes_id          = COALESCE($4, itunes_id),
            categories         = CASE WHEN cardinality($5::text[]) > 0
                                      THEN $5::text[] ELSE categories END,
            popularity_score   = COALESCE($6, popularity_score),
            language_base      = COALESCE($7, language_base),
            catalog_checked_at = CURRENT_TIMESTAMP,
            updated_at         = CURRENT_TIMESTAMP
         WHERE id = $1",
    )
    .bind(id)
    .bind(catalog_id)
    .bind(podcast_guid)
    .bind(itunes_id)
    .bind(categories)
    .bind(popularity_score)
    .bind(language_base)
    .execute(pool)
    .await
    .context("db: set podcast catalog metadata")?;
    Ok(())
}

/// Feeds whose catalogue metadata is missing or stale, oldest first.
///
/// Covers two populations: feeds subscribed before migration 0060 (which have
/// no categories at all) and feeds whose entry predates the newest dump. A
/// feed the catalogue answered "not found" for has `catalog_checked_at` set
/// and `catalog_id` NULL, so it is retried on the same slow cadence as
/// everything else rather than on every pass — the catalogue is refreshed
/// weekly, and a show absent from this week's dump may well be in next week's.
pub async fn feeds_needing_catalog_sync(
    pool: &PgPool,
    stale_after_days: i64,
    limit: i64,
) -> anyhow::Result<Vec<(Uuid, String)>> {
    sqlx::query_as::<_, (Uuid, String)>(
        "SELECT id, feed_url FROM podcast_feeds
         WHERE catalog_checked_at IS NULL
            OR catalog_checked_at < CURRENT_TIMESTAMP - make_interval(days => $1::int)
         ORDER BY catalog_checked_at ASC NULLS FIRST
         LIMIT $2",
    )
    .bind(stale_after_days)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list feeds needing catalog sync")
}

/// Catalogue ids of everything the viewer can already see.
///
/// Used to filter recommendations. Deliberately computed here rather than
/// passed to the metadata service: sending the household's subscription list
/// off the box to get a filtered answer back would hand the mirror host
/// exactly the profile this design exists to never build.
pub async fn subscribed_catalog_ids(
    pool: &PgPool,
    viewer: Viewer,
) -> anyhow::Result<std::collections::HashSet<i64>> {
    let sql = format!(
        "SELECT DISTINCT t.catalog_id FROM podcast_feeds t
         WHERE t.catalog_id IS NOT NULL AND {VISIBLE}"
    );

    let ids = bind_viewer!(sqlx::query_scalar::<_, i64>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list subscribed catalog ids")?;

    Ok(ids.into_iter().collect())
}

/// Feeds that are the same show subscribed twice within one household.
///
/// `podcast_feeds` is UNIQUE (user_id, feed_url), so two members of a family
/// both following the same show is two rows, two server-side episode downloads
/// and two library entries. Grouping by GUID within a family finds those.
///
/// **What this deliberately does not claim to find:** the same show subscribed
/// through two genuinely different feed URLs. Podcast Index synthesizes a
/// UUIDv5 from the feed URL when the publisher declares no `<podcast:guid>`,
/// so an aggregator's copy and the publisher's own feed carry different GUIDs
/// (verified 2026-08-29). Catching those needs title/author matching, which is
/// fuzzy and is not what this query is.
///
/// **Reports, never merges.** Merging touches episodes, playback progress and
/// downloaded files; it needs its own decision and its own migration, not a
/// side effect of a background sweep.
pub async fn duplicate_feeds_by_guid(pool: &PgPool) -> anyhow::Result<Vec<(String, i64)>> {
    sqlx::query_as::<_, (String, i64)>(
        "SELECT podcast_guid, count(*) AS copies
         FROM podcast_feeds
         WHERE podcast_guid IS NOT NULL
         GROUP BY podcast_guid, COALESCE(family_id, user_id)
         HAVING count(*) > 1
         ORDER BY copies DESC",
    )
    .fetch_all(pool)
    .await
    .context("db: find duplicate podcast feeds")
}

pub async fn delete_feed(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM podcast_feeds WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete podcast feed")?;
    Ok(())
}

// ── Episodes ─────────────────────────────────────────────────────────────

pub async fn list_episodes(
    pool: &PgPool,
    feed_id: Uuid,
    limit: i64,
    offset: i64,
    query: Option<&str>,
) -> anyhow::Result<Vec<PodcastEpisode>> {
    // Accent-insensitive through `translate` rather than the unaccent extension, which the
    // database may not have: "stanek" finds "Staněk".
    const FOLD: &str = "'áäčďéěíĺľňóôöŕřšťúůüýžÁÄČĎÉĚÍĹĽŇÓÔÖŔŘŠŤÚŮÜÝŽ', 'aacdeeillnooorrstuuuyzAACDEEILLNOOORRSTUUUYZ'";
    // `%` and `_` typed by someone are text, not wildcards.
    let query = query
        .map(str::trim)
        .filter(|q| !q.is_empty())
        .map(|q| q.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    sqlx::query_as::<_, PodcastEpisode>(&format!(
        "SELECT {EPISODE_COLS} FROM podcast_episodes e
         LEFT JOIN media_objects mo ON mo.id = e.audio_object_id
         WHERE e.feed_id = $1
           AND ($4::text IS NULL
                OR lower(translate(e.title || ' ' || COALESCE(e.description, ''), {FOLD}))
                   LIKE '%' || lower(translate($4, {FOLD})) || '%' ESCAPE '\\')
         ORDER BY e.published_at IS NULL, e.published_at DESC
         LIMIT $2 OFFSET $3"
    ))
    .bind(feed_id)
    .bind(limit)
    .bind(offset)
    .bind(query.as_deref())
    .fetch_all(pool)
    .await
    .context("db: list podcast episodes")
}

pub async fn find_episode(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<PodcastEpisode>> {
    sqlx::query_as::<_, PodcastEpisode>(&format!(
        "SELECT {EPISODE_COLS} FROM podcast_episodes e
         LEFT JOIN media_objects mo ON mo.id = e.audio_object_id
         WHERE e.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("db: find podcast episode")
}

/// Newest episodes across every feed the viewer can see, for Subsonic's
/// `getNewestPodcasts`.
///
/// Joined against `podcast_feeds` rather than querying episodes directly:
/// episodes carry no owner of their own, so visibility can only be decided by
/// the feed they belong to.
pub async fn list_recent_episodes(
    pool: &PgPool,
    viewer: Viewer,
    limit: i64,
) -> anyhow::Result<Vec<PodcastEpisode>> {
    let sql = format!(
        "SELECT {EPISODE_COLS}
         FROM podcast_episodes e
         JOIN podcast_feeds t ON t.id = e.feed_id
         LEFT JOIN media_objects mo ON mo.id = e.audio_object_id
         WHERE {VISIBLE}
         ORDER BY e.published_at IS NULL, e.published_at DESC
         LIMIT $5"
    );

    bind_viewer!(sqlx::query_as::<_, PodcastEpisode>(&sql), viewer)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list recent podcast episodes")
}

/// Total episode count per feed, so `getPodcasts` need not load every episode
/// just to report how many there are.
pub async fn episode_counts(pool: &PgPool, feed_ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, i64)>> {
    if feed_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, (Uuid, i64)>(
        "SELECT feed_id, COUNT(*) FROM podcast_episodes WHERE feed_id = ANY($1) GROUP BY feed_id",
    )
    .bind(feed_ids)
    .fetch_all(pool)
    .await
    .context("db: podcast episode counts")
}
