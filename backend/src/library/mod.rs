// SPDX-License-Identifier: AGPL-3.0-or-later
/// Library module — per-user catalogs, cross-library search, continue listening.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::families::FamilyContext;
use axum::Router;
use axum::extract::{Query, State};
use axum::routing::get;
use axum::Json;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum ContinueItem {
    Episode {
        episode_id: String,
        feed_id: String,
        episode_title: String,
        feed_title: String,
        position_secs: f64,
        duration_secs: Option<i32>,
        updated_at: String,
    },
    Book {
        book_id: String,
        book_title: String,
        author: Option<String>,
        position_secs: f64,
        total_duration_secs: Option<i32>,
        updated_at: String,
    },
    /// A personal-use podcast episode translation (podcast-translation-plan.md P9) — the
    /// original episode's own progress and this one are unrelated (different table, different
    /// length audio), so this exists as its own arm rather than reusing `Episode`'s shape.
    Translation {
        translation_id: String,
        episode_id: String,
        feed_id: String,
        episode_title: String,
        feed_title: String,
        target_language: String,
        position_secs: f64,
        duration_secs: Option<i32>,
        updated_at: String,
    },
}

#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    20
}

#[derive(Serialize)]
#[serde(tag = "kind")]
pub enum SearchResult {
    Feed {
        id: String,
        title: String,
        author: Option<String>,
        description: Option<String>,
    },
    Episode {
        id: String,
        feed_id: String,
        title: String,
        feed_title: Option<String>,
        published_at: Option<String>,
    },
    Book {
        id: String,
        title: String,
        author: Option<String>,
    },
    Track {
        id: String,
        title: String,
        artist: Option<String>,
        album: Option<String>,
    },
}

// ── Router ────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/continue", get(continue_listening))
        .route("/search", get(search))
        .route("/private", get(private_library))
        .route("/changes", get(changes))
        .nest("/folders", crate::library_folders::routes::router())
}

// ── Delta sync ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ChangesQuery {
    /// RFC3339 timestamp from the previous sync's `now`. Omitted ⇒ a full
    /// snapshot, which is what a first run wants.
    #[serde(default)]
    pub since: Option<String>,
}

#[derive(Serialize)]
pub struct ChangesResponse {
    /// Echo of the requested cursor (null on a full sync).
    pub since: Option<String>,
    /// Pass this back as `since` next time. Taken *before* the queries run,
    /// so a row written mid-request is re-sent rather than skipped.
    pub now: String,
    /// True when no cursor was supplied and this is a complete snapshot.
    pub full_sync: bool,
    pub audiobooks: Vec<serde_json::Value>,
    pub podcasts: Vec<serde_json::Value>,
    pub tracks: Vec<serde_json::Value>,
    /// Items removed since `since`, so the client can purge its cache.
    pub deleted: Vec<DeletedItem>,
}

#[derive(Serialize)]
pub struct DeletedItem {
    pub media_kind: String,
    pub item_id: String,
    pub deleted_at: String,
}

/// Parse a sync cursor, tolerating the `+` → space mangling that happens when
/// an RFC3339 offset is put in a query string unencoded. We hand out `Z`-form
/// timestamps precisely so this cannot bite, but a client that builds its own
/// cursor from a local clock should not get a cryptic 400 either.
fn parse_cursor(raw: &str) -> Result<DateTime<Utc>, AuthError> {
    DateTime::parse_from_rfc3339(raw)
        .or_else(|_| DateTime::parse_from_rfc3339(&raw.replacen(' ', "+", 1)))
        .map(|dt| dt.with_timezone(&Utc))
        .map_err(|e| AuthError::BadRequest(format!("invalid `since` timestamp: {e}")))
}

/// GET /api/v1/library/changes?since=<rfc3339>
///
/// One request that returns everything a cached client needs to catch up:
/// changed content of every kind plus tombstones for deletions. Built for
/// mobile, where re-fetching whole lists over cellular is the thing to avoid.
async fn changes(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(q): Query<ChangesQuery>,
) -> Result<Json<ChangesResponse>, AuthError> {
    // Snapshot the cursor before querying: anything written while we read
    // will be picked up next time instead of falling in the gap.
    let now = Utc::now();

    let since = match q.since.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
        Some(raw) => Some(parse_cursor(raw)?),
        None => None,
    };

    // A full sync is just a delta from the epoch.
    let cursor = since.unwrap_or_else(|| DateTime::<Utc>::from_timestamp(0, 0).unwrap_or(now));
    let pool = state.db();
    let viewer = family.viewer();

    let books = crate::db::sync::changed_books(pool, viewer, cursor)
        .await
        .map_err(AuthError::Internal)?;
    let feeds = crate::db::sync::changed_feeds(pool, viewer, cursor)
        .await
        .map_err(AuthError::Internal)?;
    let tracks = crate::db::sync::changed_tracks(pool, viewer, cursor)
        .await
        .map_err(AuthError::Internal)?;

    // Tombstones only make sense for an incremental sync: on a full one the
    // client is rebuilding from scratch and has nothing to purge.
    let deleted = match since {
        Some(since) => crate::db::sync::deletions_since(pool, family.user_id, family.family_id, since)
            .await
            .map_err(AuthError::Internal)?
            .into_iter()
            .map(|t| DeletedItem {
                media_kind: t.media_kind,
                item_id: t.item_id.to_string(),
                deleted_at: t.deleted_at.to_rfc3339(),
            })
            .collect(),
        None => Vec::new(),
    };

    Ok(Json(ChangesResponse {
        since: since.map(|s| s.to_rfc3339_opts(chrono::SecondsFormat::Millis, true)),
        now: now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true),
        full_sync: since.is_none(),
        audiobooks: books
            .into_iter()
            .map(|b| {
                serde_json::json!({
                    "id": b.id.to_string(),
                    "title": b.title,
                    "author": b.author,
                    "narrator": b.narrator,
                    "total_duration_secs": b.total_duration_secs,
                    "visibility": crate::db::access::visibility_of(b.family_id),
                    "is_owner": b.user_id == family.user_id,
                    "owner_id": b.user_id.to_string(),
                    "updated_at": b.updated_at.to_rfc3339(),
                })
            })
            .collect(),
        podcasts: feeds
            .into_iter()
            .map(|f| {
                serde_json::json!({
                    "id": f.id.to_string(),
                    "title": f.title,
                    "author": f.author,
                    "feed_url": f.feed_url,
                    "visibility": crate::db::access::visibility_of(f.family_id),
                    "is_owner": f.user_id == family.user_id,
                    "owner_id": f.user_id.to_string(),
                    "updated_at": f.updated_at.to_rfc3339(),
                })
            })
            .collect(),
        tracks: tracks
            .into_iter()
            .map(|t| {
                serde_json::json!({
                    "id": t.id.to_string(),
                    "album_artist": t.effective_album_artist(),
                    "title": t.title,
                    "artist": t.artist,
                    "album": t.album,
                    "track_number": t.track_number,
                    "disc_number": t.disc_number,
                    "duration_secs": t.duration_secs,
                    "visibility": crate::db::access::visibility_of(t.family_id),
                    "is_owner": t.user_id == family.user_id,
                    "owner_id": t.user_id.to_string(),
                    "updated_at": t.updated_at.to_rfc3339(),
                })
            })
            .collect(),
        deleted,
    }))
}

/// One private item, in the caller's "Soukromé" folder.
#[derive(Serialize)]
pub struct PrivateItem {
    pub kind: String,
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    /// Same URL the item's own list returns; absent when it has no picture.
    pub cover_url: Option<String>,
    /// Songs only: the album, so a client can group a long list of songs.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<String>,
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/library/private
/// Everything the caller owns that is NOT shared with their family — the
/// backing list for a "Soukromé" section. Always scoped to the caller: there
/// is no way to read anyone else's private folder, not even as family admin.
async fn private_library(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<PrivateItem>>, AuthError> {
    let pool = state.db();
    let mut items = Vec::new();

    let books = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, bool)>(
        "SELECT id, title, author, cover_object_id IS NOT NULL FROM audiobook_books
         WHERE user_id = $1 AND family_id IS NULL ORDER BY title",
    )
    .bind(family.user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, title, author, has_cover) in books {
        items.push(PrivateItem {
            kind: "audiobook".to_string(),
            id: id.to_string(),
            title,
            subtitle: author,
            cover_url: has_cover.then(|| format!("/api/v1/audiobooks/{id}/cover")),
            album: None,
        });
    }

    let feeds = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, Option<uuid::Uuid>, Option<String>)>(
        "SELECT id, title, author, image_object_id, image_url FROM podcast_feeds
         WHERE user_id = $1 AND family_id IS NULL ORDER BY title",
    )
    .bind(family.user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, title, author, image_object, image_url) in feeds {
        items.push(PrivateItem {
            kind: "podcast".to_string(),
            id: id.to_string(),
            title,
            subtitle: author,
            // The stored copy through our proxy when there is one, as the feed list does.
            cover_url: if image_object.is_some() { Some(format!("/api/v1/podcasts/{id}/image")) } else { image_url },
            album: None,
        });
    }

    let tracks = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, Option<String>, bool)>(
        "SELECT id, title, artist, album, cover_object_id IS NOT NULL FROM music_tracks
         WHERE user_id = $1 AND family_id IS NULL ORDER BY album NULLS LAST, title",
    )
    .bind(family.user_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, title, artist, album, has_cover) in tracks {
        items.push(PrivateItem {
            kind: "music".to_string(),
            id: id.to_string(),
            title,
            subtitle: artist,
            cover_url: has_cover.then(|| format!("/api/v1/music/tracks/{id}/cover")),
            album,
        });
    }

    Ok(Json(items))
}

/// GET /api/v1/library/continue
/// Returns the 20 most recently played (not completed) items across all content types.
async fn continue_listening(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<ContinueItem>>, AuthError> {
    let pool = state.db();

    // Recent podcast episodes in progress (not completed)
    let episode_rows = sqlx::query_as::<
        _,
        (uuid::Uuid, uuid::Uuid, String, String, f64, Option<i32>, DateTime<Utc>),
    >(
        &format!("SELECT pp.episode_id, pe.feed_id, pe.title, t.title,
                pp.position_secs, pe.duration_secs, pp.updated_at
         FROM podcast_progress pp
         JOIN podcast_episodes pe ON pe.id = pp.episode_id
         JOIN podcast_feeds    t  ON t.id  = pe.feed_id
         WHERE pp.user_id = $1 AND pp.completed = false
           AND {visible_podcast}
         ORDER BY pp.updated_at DESC
         LIMIT 20", visible_podcast = crate::db::access::visible_for("'podcast'")),
    )
    .bind(family.user_id)
    .bind(family.family_id)
    .bind(family.is_family_admin())
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    // Recent audiobooks in progress (not completed). `audiobook_progress.position_secs` is
    // scoped to a single file (whichever one the client last reported), but the client divides
    // this by the *whole book's* `total_duration_secs` to show a percentage — so it has to be a
    // book-wide cumulative position here, not the raw per-file value, or anyone past file 1
    // sees a percentage that understates their real progress. The LATERAL join sums the
    // durations of every file that sorts before the progress-tracked one; COALESCE covers the
    // zero-prior-files case (LATERAL returns no row, not a zero, when nothing matches).
    let book_rows = sqlx::query_as::<
        _,
        (uuid::Uuid, String, Option<String>, f64, Option<i32>, DateTime<Utc>),
    >(
        &format!("SELECT ap.book_id, t.title, t.author,
                COALESCE(prior.prior_secs, 0) + ap.position_secs, t.total_duration_secs, ap.updated_at
         FROM audiobook_progress ap
         JOIN audiobook_books t ON t.id = ap.book_id
         JOIN audiobook_files cur ON cur.id = ap.file_id
         LEFT JOIN LATERAL (
             SELECT SUM(af.duration_secs) AS prior_secs
             FROM audiobook_files af
             WHERE af.book_id = ap.book_id AND af.position < cur.position
         ) prior ON true
         WHERE ap.user_id = $1 AND ap.completed = false
           AND {visible_audiobook}
         ORDER BY ap.updated_at DESC
         LIMIT 20", visible_audiobook = crate::db::access::visible_for("'audiobook'")),
    )
    .bind(family.user_id)
    .bind(family.family_id)
    .bind(family.is_family_admin())
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    // Recent podcast episode translations in progress (not completed). Family-scoped, not
    // owner-only — podcast-translation-plan.md §0 treats a translation as belonging to the
    // requesting user's household, and `db::podcast_translate`'s own queries were widened to
    // `family_id` on 2026-08-25 for the same reason (see that migration's commit). This was
    // the one call site that got missed in that pass — `t.user_id = $1` here meant a
    // translation never showed up in Continue Listening for anyone but whoever requested it,
    // even though listing/playing/progress were already family-wide by then.
    let translation_rows = sqlx::query_as::<
        _,
        (uuid::Uuid, uuid::Uuid, uuid::Uuid, String, String, String, f64, Option<i32>, DateTime<Utc>),
    >(
        "SELECT t.id, t.episode_id, pe.feed_id, pe.title, pf.title, t.target_language,
                t.progress_secs, t.duration_secs, t.progress_updated_at
         FROM podcast_episode_translations t
         JOIN podcast_episodes pe ON pe.id = t.episode_id
         JOIN podcast_feeds    pf ON pf.id = pe.feed_id
         WHERE t.family_id = $1 AND t.status = 'complete' AND t.completed = false
           AND t.progress_updated_at IS NOT NULL
         ORDER BY t.progress_updated_at DESC
         LIMIT 20",
    )
    .bind(family.family_id)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    // Merge and sort by updated_at descending, take top 20
    let mut items: Vec<(DateTime<Utc>, ContinueItem)> = Vec::new();

    for (episode_id, feed_id, episode_title, feed_title, position_secs, duration_secs, updated_at) in
        episode_rows
    {
        items.push((
            updated_at,
            ContinueItem::Episode {
                episode_id: episode_id.to_string(),
                feed_id: feed_id.to_string(),
                episode_title,
                feed_title,
                position_secs,
                duration_secs,
                updated_at: updated_at.to_rfc3339(),
            },
        ));
    }

    for (book_id, book_title, author, position_secs, total_duration_secs, updated_at) in book_rows {
        items.push((
            updated_at,
            ContinueItem::Book {
                book_id: book_id.to_string(),
                book_title,
                author,
                position_secs,
                total_duration_secs,
                updated_at: updated_at.to_rfc3339(),
            },
        ));
    }

    for (
        translation_id,
        episode_id,
        feed_id,
        episode_title,
        feed_title,
        target_language,
        position_secs,
        duration_secs,
        updated_at,
    ) in translation_rows
    {
        items.push((
            updated_at,
            ContinueItem::Translation {
                translation_id: translation_id.to_string(),
                episode_id: episode_id.to_string(),
                feed_id: feed_id.to_string(),
                episode_title,
                feed_title,
                target_language,
                position_secs,
                duration_secs,
                updated_at: updated_at.to_rfc3339(),
            },
        ));
    }

    items.sort_by_key(|i| std::cmp::Reverse(i.0));
    items.truncate(20);

    Ok(Json(items.into_iter().map(|(_, item)| item).collect()))
}

/// GET /api/v1/library/search?q=…&limit=20
/// Simple case-insensitive search across feeds, episodes, and audiobooks.
async fn search(
    family: FamilyContext,
    State(state): State<AppState>,
    Query(params): Query<SearchQuery>,
) -> Result<Json<Vec<SearchResult>>, AuthError> {
    let pool = state.db();
    let pattern = format!("%{}%", params.q.replace('%', "\\%").replace('_', "\\_"));
    let limit = params.limit.min(50);

    let mut results: Vec<SearchResult> = Vec::new();

    // Podcast feeds
    let feeds = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, Option<String>)>(
        &format!("SELECT t.id, t.title, t.author, t.description
         FROM podcast_feeds t
         WHERE {visible_podcast}
           AND (
                    LOWER(COALESCE(t.title, '')) LIKE LOWER($4)
                 OR LOWER(COALESCE(t.author, '')) LIKE LOWER($4)
                 OR LOWER(COALESCE(t.description, '')) LIKE LOWER($4)
               )
         LIMIT $5", visible_podcast = crate::db::access::visible_for("'podcast'")),
    )
    .bind(family.user_id)
    .bind(family.family_id)
    .bind(family.is_family_admin())
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, title, author, description) in feeds {
        results.push(SearchResult::Feed {
            id: id.to_string(),
            title,
            author,
            description,
        });
    }

    // Podcast episodes (via user's feeds)
    let episodes = sqlx::query_as::<
        _,
        (uuid::Uuid, uuid::Uuid, String, Option<String>, Option<DateTime<Utc>>),
    >(
        &format!("SELECT pe.id, pe.feed_id, pe.title, t.title, pe.published_at
         FROM podcast_episodes pe
         JOIN podcast_feeds t ON t.id = pe.feed_id
         WHERE {visible_podcast}
           AND LOWER(pe.title) LIKE LOWER($4)
         ORDER BY pe.published_at DESC
         LIMIT $5", visible_podcast = crate::db::access::visible_for("'podcast'")),
    )
    .bind(family.user_id)
    .bind(family.family_id)
    .bind(family.is_family_admin())
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, feed_id, title, feed_title, published_at) in episodes {
        results.push(SearchResult::Episode {
            id: id.to_string(),
            feed_id: feed_id.to_string(),
            title,
            feed_title,
            published_at: published_at.map(|t| t.to_rfc3339()),
        });
    }

    // Audiobooks
    let books = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>)>(
        &format!("SELECT t.id, t.title, t.author
         FROM audiobook_books t
         WHERE {visible_audiobook}
           AND (LOWER(COALESCE(t.title, '')) LIKE LOWER($4)
                OR LOWER(COALESCE(t.author, '')) LIKE LOWER($4))
         LIMIT $5", visible_audiobook = crate::db::access::visible_for("'audiobook'")),
    )
    .bind(family.user_id)
    .bind(family.family_id)
    .bind(family.is_family_admin())
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, title, author) in books {
        results.push(SearchResult::Book {
            id: id.to_string(),
            title,
            author,
        });
    }

    // Music tracks — previously missing from cross-library search.
    let tracks = sqlx::query_as::<_, (uuid::Uuid, String, Option<String>, Option<String>)>(
        &format!("SELECT t.id, t.title, t.artist, t.album
         FROM music_tracks t
         WHERE {visible_music}
           AND (LOWER(COALESCE(t.title, '')) LIKE LOWER($4)
                OR LOWER(COALESCE(t.artist, '')) LIKE LOWER($4)
                OR LOWER(COALESCE(t.album, '')) LIKE LOWER($4))
         ORDER BY t.title
         LIMIT $5", visible_music = crate::db::access::visible_for("'music'")),
    )
    .bind(family.user_id)
    .bind(family.family_id)
    .bind(family.is_family_admin())
    .bind(&pattern)
    .bind(limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    for (id, title, artist, album) in tracks {
        results.push(SearchResult::Track {
            id: id.to_string(),
            title,
            artist,
            album,
        });
    }

    Ok(Json(results))
}

