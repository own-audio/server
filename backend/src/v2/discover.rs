// SPDX-License-Identifier: AGPL-3.0-or-later
//! Finding music (docs/api-v2-plan.md, V10): one search across artists,
//! albums and tracks, random tracks, the caller's own most and most recently
//! played, and a track's lyrics.
//!
//! Search matches parts of names, any case, and puts what starts with the
//! words first. Plays are the caller's own and never anyone else's: the
//! listening history stays personal (migration 0070).

use crate::app::AppState;
use crate::db::catalog;
use crate::families::FamilyContext;
use crate::v2::music::{Album, Contributor, Included, Track, albums, contributor, included, tracks, wanted};
use crate::v2::page::Page;
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

fn limit_of(limit: Option<i64>, default: i64, max: i64) -> Result<i64, Problem> {
    match limit {
        None => Ok(default),
        Some(n) if (1..=max).contains(&n) => Ok(n),
        Some(_) => Err(Problem::bad_request("invalid_limit", format!("limit is 1 to {max}")).field("limit", "out_of_range", format!("1 to {max}"))),
    }
}

async fn with_included(state: &AppState, family: &FamilyContext, include: Option<&str>, items: &[Track]) -> Result<Option<Included>, Problem> {
    let w = wanted(include, &["albums", "contributors", "genres"])?;
    let credit_ids: Vec<Uuid> = items.iter().flat_map(|t| t.credits.iter().map(|c| c.contributor_id)).collect();
    let album_ids: Vec<Uuid> = items.iter().filter_map(|t| t.album_id).collect();
    let genre_ids: Vec<Uuid> = items.iter().flat_map(|t| t.genre_ids.iter().copied()).collect();
    included(state, family.viewer(), &w, credit_ids.into_iter(), album_ids.into_iter(), std::iter::empty(), genre_ids.into_iter()).await
}

// ── Search ───────────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct SearchQuery {
    /// What to look for: part of a name or title, any case.
    pub q: String,
    /// What to look in, comma-separated: `artists`, `albums`, `tracks` (all three by default).
    pub types: Option<String>,
    /// At most this many of each, 1 to 100; 20 by default.
    pub limit: Option<i64>,
    /// For the tracks: `albums`, `contributors`, `genres`.
    pub include: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct SearchAnswer {
    /// Contributors credited as artists.
    pub artists: Vec<Contributor>,
    pub albums: Vec<Album>,
    /// Whose title, album or a credited name matches.
    pub tracks: Vec<Track>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub included: Option<Included>,
}

/// Search the music the caller can see.
#[utoipa::path(get, path = "/search", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(SearchQuery),
    responses((status = 200, body = SearchAnswer),
        (status = 400, description = "`invalid_query` (empty), `invalid_type`, `invalid_limit`", body = crate::v2::problem::ProblemBody)))]
pub async fn search(family: FamilyContext, State(state): State<AppState>, Query(q): Query<SearchQuery>) -> Result<Json<SearchAnswer>, Problem> {
    let text = q.q.trim();
    if text.is_empty() || text.chars().count() > 200 {
        return Err(Problem::bad_request("invalid_query", "search for 1 to 200 characters").field("q", "invalid", "1 to 200 characters"));
    }
    let limit = limit_of(q.limit, 20, 100)?;
    let (mut want_artists, mut want_albums, mut want_tracks) = (q.types.is_none(), q.types.is_none(), q.types.is_none());
    for part in q.types.as_deref().unwrap_or("").split(',').map(str::trim).filter(|p| !p.is_empty()) {
        match part {
            "artists" => want_artists = true,
            "albums" => want_albums = true,
            "tracks" => want_tracks = true,
            other => {
                return Err(Problem::bad_request("invalid_type", format!("`{other}`: one of artists, albums, tracks")).field("types", "invalid", other.to_string()));
            }
        }
    }
    let viewer = family.viewer();
    let pool = state.db();
    let artists = if want_artists {
        catalog::search_artists(pool, viewer, text, limit).await?.into_iter().map(|k| contributor(&state, k)).collect()
    } else {
        Vec::new()
    };
    let found_albums = if want_albums { albums(&state, viewer, catalog::search_albums(pool, viewer, text, limit).await?).await? } else { Vec::new() };
    let found_tracks = if want_tracks { tracks(&state, catalog::search_tracks(pool, viewer, text, limit).await?).await? } else { Vec::new() };
    let included = with_included(&state, &family, q.include.as_deref(), &found_tracks).await?;
    Ok(Json(SearchAnswer { artists, albums: found_albums, tracks: found_tracks, included }))
}

// ── Random tracks ────────────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct RandomQuery {
    /// Only from this genre.
    pub genre_id: Option<Uuid>,
    /// 1 to 500; 50 by default.
    pub limit: Option<i64>,
    /// `albums`, `contributors`, `genres`.
    pub include: Option<String>,
}

/// Tracks the caller can see, in random order; a new draw on every call, so
/// the answer has no next page.
#[utoipa::path(get, path = "/tracks/random", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(RandomQuery),
    responses((status = 200, body = Page<Track>)))]
pub async fn random_tracks(family: FamilyContext, State(state): State<AppState>, Query(q): Query<RandomQuery>) -> Result<Json<Page<Track>>, Problem> {
    let limit = limit_of(q.limit, 50, 500)?;
    let items = tracks(&state, catalog::random_tracks(state.db(), family.viewer(), q.genre_id, limit).await?).await?;
    let included = with_included(&state, &family, q.include.as_deref(), &items).await?;
    Ok(Json(Page { items, next: None, included }))
}

// ── The caller's plays ───────────────────────────────────────────────────────

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct PlayedQuery {
    /// `track` or `album`.
    pub kind: String,
    /// `recent` (the default) or `most`.
    pub order: Option<String>,
    /// 1 to 500; 100 by default.
    pub limit: Option<i64>,
}

#[derive(Serialize, ToSchema)]
pub struct Played {
    pub kind: String,
    pub id: Uuid,
    /// How many times the caller played it (an album: its tracks' plays together).
    pub play_count: i64,
    pub last_played_at: DateTime<Utc>,
}

/// What the caller played, most recently or most often first: the tops of
/// their own history, not a page through it.
#[utoipa::path(get, path = "/me/played", tag = "state", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PlayedQuery),
    responses((status = 200, body = Page<Played>),
        (status = 400, description = "`unsupported_kind`, `invalid_order`, `invalid_limit`", body = crate::v2::problem::ProblemBody)))]
pub async fn played(family: FamilyContext, State(state): State<AppState>, Query(q): Query<PlayedQuery>) -> Result<Json<Page<Played>>, Problem> {
    let per_album = match q.kind.as_str() {
        "track" => false,
        "album" => true,
        other => return Err(Problem::bad_request("unsupported_kind", format!("`{other}`: track or album")).field("kind", "invalid", other.to_string())),
    };
    let most = match q.order.as_deref() {
        None | Some("recent") => false,
        Some("most") => true,
        Some(other) => return Err(Problem::bad_request("invalid_order", format!("`{other}`: recent or most")).field("order", "invalid", other.to_string())),
    };
    let limit = limit_of(q.limit, 100, 500)?;
    let rows = catalog::played(state.db(), family.viewer(), per_album, most, limit).await?;
    let items = rows.into_iter().map(|(id, play_count, last_played_at)| Played { kind: q.kind.clone(), id, play_count, last_played_at }).collect();
    Ok(Json(Page { items, next: None, included: None }))
}

// ── Lyrics ───────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Line {
    /// Where the line starts; absent for lyrics without timing.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<i64>,
    pub text: String,
}

#[derive(Serialize, ToSchema)]
pub struct Lyrics {
    /// The lyrics as stored (LRC timing kept); `null` when the track has none.
    pub text: Option<String>,
    /// True when the lines carry their times.
    pub synced: bool,
    pub lines: Vec<Line>,
}

/// A track's lyrics: the owner's, a sidecar `.lrc`, or the file's own tag
/// (read the first time they are asked for).
#[utoipa::path(get, path = "/tracks/{id}/lyrics", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Lyrics), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn lyrics(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Lyrics>, Problem> {
    catalog::find_track(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let (stored, audio) = catalog::track_lyrics(state.db(), id).await?;
    let text = match (stored, audio) {
        (Some(text), _) => text,
        (None, None) => String::new(),
        (None, Some(object)) => {
            let key: String = sqlx::query_scalar("SELECT object_key FROM media_objects WHERE id = $1")
                .bind(object)
                .fetch_one(state.db())
                .await
                .map_err(anyhow::Error::from)?;
            let temp = state.storage().download_to_temp(&key).await?;
            let found = crate::music::parse_lyrics_tag(temp.path()).unwrap_or_default();
            crate::db::music::update_track_lyrics(state.db(), id, &found).await?;
            found
        }
    };
    Ok(Json(answer(text)))
}

fn answer(text: String) -> Lyrics {
    if text.trim().is_empty() {
        return Lyrics { text: None, synced: false, lines: Vec::new() };
    }
    let timed = crate::subsonic::playback::parse_lrc(&text);
    let (synced, lines) = if timed.is_empty() {
        (false, text.lines().map(|l| Line { start_ms: None, text: l.trim_end().to_string() }).collect())
    } else {
        (true, timed.into_iter().map(|(ms, text)| Line { start_ms: Some(ms), text }).collect())
    };
    Lyrics { text: Some(text), synced, lines }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lyrics_with_and_without_times() {
        let synced = answer("[00:01.50]One\n[00:03.00][00:10.00]Two\n".into());
        assert!(synced.synced);
        assert_eq!(synced.lines.iter().map(|l| l.start_ms).collect::<Vec<_>>(), vec![Some(1500), Some(3000), Some(10000)]);
        let plain = answer("One\nTwo".into());
        assert!(!plain.synced && plain.lines.len() == 2 && plain.lines[0].start_ms.is_none());
        let none = answer("  ".into());
        assert!(none.text.is_none() && none.lines.is_empty());
    }
}
