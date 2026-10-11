// SPDX-License-Identifier: AGPL-3.0-or-later
//! Audiobooks and podcasts in the catalogue (DESIGN §7.4; docs/api-v2-plan.md,
//! V3b): books with their files, chapters and series; podcasts and their
//! episodes. Credits are the same contributors music has (`/authors`,
//! `/narrators`, `/hosts`), so the same person is one id whatever they do.

use crate::app::AppState;
use crate::db::access::visibility_of;
use crate::db::catalog::{self, TitleOrAdded};
use crate::families::FamilyContext;
use crate::v2::music::{Credit, Included};
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use axum::Json;
use axum::extract::{Path, Query, State};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use utoipa::{IntoParams, ToSchema};
use uuid::Uuid;

#[derive(Serialize, ToSchema)]
pub struct Book {
    pub id: Uuid,
    pub title: String,
    /// Authors, narrators, translators… in order.
    pub credits: Vec<Credit>,
    pub description: Option<String>,
    pub duration_ms: Option<i64>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub published_year: Option<i32>,
    pub visibility: String,
    pub owner_id: Uuid,
    pub source: String,
    /// Its cover (`GET /images/{id}`; add `w` for the size).
    pub image: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Only on `GET /books/{id}`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub files: Option<Vec<BookFile>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chapters: Option<Vec<Chapter>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub series: Option<Vec<SeriesEntry>>,
}

/// One audio file of a book, in playing order.
#[derive(Serialize, ToSchema)]
pub struct BookFile {
    pub id: Uuid,
    pub position: i32,
    pub title: Option<String>,
    pub duration_ms: Option<i64>,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
}

/// A chapter: where it starts in the whole book, across files.
#[derive(Serialize, ToSchema)]
pub struct Chapter {
    pub id: Uuid,
    pub position: i32,
    pub title: String,
    pub start_ms: i64,
    pub file_id: Option<Uuid>,
}

#[derive(Serialize, ToSchema)]
pub struct SeriesEntry {
    pub series_id: Uuid,
    pub name: String,
    /// Fractional when the publisher numbers a book between two others ("1.5").
    pub position: f64,
}

#[derive(Serialize, ToSchema)]
pub struct Podcast {
    pub id: Uuid,
    pub title: String,
    /// Its hosts.
    pub credits: Vec<Credit>,
    pub feed_url: String,
    pub description: Option<String>,
    pub link: Option<String>,
    pub language: Option<String>,
    /// `<podcast:guid>`, the feed's own stable id.
    pub podcast_guid: Option<String>,
    pub categories: Vec<String>,
    /// The show's art: on this server (`GET /images/{id}`), or the publisher's.
    pub image: Option<String>,
    pub visibility: String,
    pub owner_id: Uuid,
    pub last_refreshed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Serialize, ToSchema)]
pub struct Episode {
    pub id: Uuid,
    pub podcast_id: Uuid,
    /// The feed's id for it.
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_ms: Option<i64>,
    pub episode_number: Option<i32>,
    pub season_number: Option<i32>,
    /// The publisher's file.
    pub audio_url: Option<String>,
    /// Whether this server keeps a copy.
    pub stored: bool,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
    /// The episode's own art, on this server or the publisher's.
    pub image: Option<String>,
    pub transcript_url: Option<String>,
    pub transcript_type: Option<String>,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ShelfQuery {
    /// Credited to this contributor (an author, a narrator, a host).
    pub contributor_id: Option<Uuid>,
    /// Part of the title, any case.
    pub q: Option<String>,
    /// `title` (default) or `added`, newest first.
    pub sort: Option<String>,
    /// `contributors` to send them along.
    pub include: Option<String>,
    pub limit: Option<i64>,
    pub cursor: Option<String>,
}

impl ShelfQuery {
    fn by_added(&self) -> Result<bool, Problem> {
        match self.sort.as_deref() {
            None | Some("title") => Ok(false),
            Some("added") => Ok(true),
            Some(other) => Err(Problem::bad_request("invalid_sort", format!("`{other}`: sort by `title` or `added`")).field("sort", "invalid", other.to_string())),
        }
    }

    fn include_contributors(&self) -> Result<bool, Problem> {
        match self.include.as_deref().map(str::trim) {
            None | Some("") => Ok(false),
            Some("contributors") => Ok(true),
            Some(other) => Err(Problem::bad_request("invalid_include", format!("`{other}` cannot be included here; one of: contributors"))
                .field("include", "invalid", other.to_string())),
        }
    }

    fn page(&self, by_added: bool) -> Result<(i64, Option<TitleOrAdded>), Problem> {
        let page = PageQuery { limit: self.limit, cursor: self.cursor.clone() };
        let after = if by_added {
            page.after::<(DateTime<Utc>, Uuid)>()?.map(|(at, id)| TitleOrAdded::Added(at, id))
        } else {
            page.after::<(String, Uuid)>()?.map(|(t, id)| TitleOrAdded::Title(t, id))
        };
        Ok((page.limit()?, after))
    }
}

pub(crate) async fn credits_of(state: &AppState, kind: &str, ids: &[Uuid]) -> Result<HashMap<Uuid, Vec<Credit>>, Problem> {
    let mut out: HashMap<Uuid, Vec<Credit>> = HashMap::new();
    for row in catalog::credits(state.db(), kind, ids).await? {
        out.entry(row.item_id).or_default().push(Credit {
            contributor_id: row.contributor_id,
            role: row.role,
            position: row.position,
            credited_as: row.credited_as,
            join_phrase: row.join_phrase,
        });
    }
    Ok(out)
}

async fn contributors_of<'a>(state: &AppState, family: &FamilyContext, credits: impl Iterator<Item = &'a Credit>) -> Result<Option<Included>, Problem> {
    let ids: Vec<Uuid> = credits.map(|c| c.contributor_id).collect::<BTreeSet<_>>().into_iter().collect();
    let contributors = catalog::contributors_by_ids(state.db(), family.family_id, &ids)
        .await?
        .into_iter()
        .map(|k| crate::v2::music::contributor(state, k))
        .collect();
    Ok(Some(Included { contributors, ..Default::default() }))
}

pub(crate) fn book(state: &AppState, row: catalog::BookRow, credits: Vec<Credit>) -> Book {
    Book {
        id: row.id,
        title: row.title,
        credits,
        description: row.description,
        duration_ms: row.total_duration_secs.map(|s| i64::from(s) * 1000),
        isbn: row.isbn,
        publisher: row.publisher,
        published_year: row.published_year,
        visibility: visibility_of(row.family_id).to_string(),
        owner_id: row.user_id,
        source: row.source,
        image: crate::v2::images::maybe(state, row.cover_object_id),
        created_at: row.created_at,
        updated_at: row.updated_at,
        files: None,
        chapters: None,
        series: None,
    }
}

pub(crate) fn podcast(state: &AppState, row: catalog::PodcastRow, credits: Vec<Credit>) -> Podcast {
    Podcast {
        id: row.id,
        title: row.title,
        credits,
        feed_url: row.feed_url,
        description: row.description,
        link: row.link,
        language: row.language,
        podcast_guid: row.podcast_guid,
        categories: row.categories,
        image: crate::v2::images::maybe(state, row.image_object_id).or(row.image_url),
        visibility: visibility_of(row.family_id).to_string(),
        owner_id: row.user_id,
        last_refreshed_at: row.last_refreshed_at,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

pub(crate) fn episode(state: &AppState, row: catalog::EpisodeRow) -> Episode {
    Episode {
        id: row.id,
        podcast_id: row.podcast_id,
        guid: row.guid,
        title: row.title,
        description: row.description,
        published_at: row.published_at,
        duration_ms: row.duration_secs.map(|s| i64::from(s) * 1000),
        episode_number: row.episode_number,
        season_number: row.season_number,
        audio_url: row.audio_url,
        stored: row.stored,
        size_bytes: row.size_bytes,
        content_type: row.content_type,
        image: crate::v2::images::maybe(state, row.image_object_id).or(row.image_url),
        transcript_url: row.transcript_url,
        transcript_type: row.transcript_type,
        created_at: row.created_at,
    }
}

/// Books the caller can see.
#[utoipa::path(get, path = "/books", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(ShelfQuery),
    responses((status = 200, body = Page<Book>)))]
pub async fn list_books(family: FamilyContext, State(state): State<AppState>, Query(q): Query<ShelfQuery>) -> Result<Json<Page<Book>>, Problem> {
    let by_added = q.by_added()?;
    let include = q.include_contributors()?;
    let (limit, after) = q.page(by_added)?;
    let rows = catalog::list_books(state.db(), family.viewer(), q.contributor_id, q.q.as_deref(), by_added, after, limit + 1).await?;
    let page = if by_added {
        let p = Page::from_rows(rows, limit, |b| (b.created_at, b.id));
        (p.items, p.next)
    } else {
        let p = Page::from_rows(rows, limit, |b| (b.sort_key.clone(), b.id));
        (p.items, p.next)
    };
    let ids: Vec<Uuid> = page.0.iter().map(|b| b.id).collect();
    let mut credits = credits_of(&state, "book", &ids).await?;
    let items: Vec<Book> = page.0.into_iter().map(|b| {
        let c = credits.remove(&b.id).unwrap_or_default();
        book(&state, b, c)
    }).collect();
    let included = if include { contributors_of(&state, &family, items.iter().flat_map(|b| b.credits.iter())).await? } else { None };
    Ok(Json(Page { items, next: page.1, included }))
}

#[derive(Serialize, ToSchema)]
pub struct BookAnswer {
    #[serde(flatten)]
    pub book: Book,
    /// Its contributors, always: a book is shown with its people.
    pub included: Included,
}

/// One book, with its files, chapters, series and contributors.
#[utoipa::path(get, path = "/books/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = BookAnswer), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_book(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<BookAnswer>, Problem> {
    let row = catalog::find_book(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let answer = full_book(&state, row).await?;
    let included = contributors_of(&state, &family, answer.credits.iter()).await?.unwrap_or_default();
    Ok(Json(BookAnswer { book: answer, included }))
}

/// Podcasts the caller can see.
#[utoipa::path(get, path = "/podcasts", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(ShelfQuery),
    responses((status = 200, body = Page<Podcast>)))]
pub async fn list_podcasts(family: FamilyContext, State(state): State<AppState>, Query(q): Query<ShelfQuery>) -> Result<Json<Page<Podcast>>, Problem> {
    let by_added = q.by_added()?;
    let include = q.include_contributors()?;
    let (limit, after) = q.page(by_added)?;
    let rows = catalog::list_podcasts(state.db(), family.viewer(), q.contributor_id, q.q.as_deref(), by_added, after, limit + 1).await?;
    let page = if by_added {
        let p = Page::from_rows(rows, limit, |r| (r.created_at, r.id));
        (p.items, p.next)
    } else {
        let p = Page::from_rows(rows, limit, |r| (r.sort_key.clone(), r.id));
        (p.items, p.next)
    };
    let ids: Vec<Uuid> = page.0.iter().map(|p| p.id).collect();
    let mut credits = credits_of(&state, "podcast", &ids).await?;
    let items: Vec<Podcast> = page.0.into_iter().map(|p| {
        let c = credits.remove(&p.id).unwrap_or_default();
        podcast(&state, p, c)
    }).collect();
    let included = if include { contributors_of(&state, &family, items.iter().flat_map(|p| p.credits.iter())).await? } else { None };
    Ok(Json(Page { items, next: page.1, included }))
}

/// One podcast.
#[utoipa::path(get, path = "/podcasts/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Podcast), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_podcast(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Podcast>, Problem> {
    let row = catalog::find_podcast(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let credits = credits_of(&state, "podcast", &[id]).await?.remove(&id).unwrap_or_default();
    Ok(Json(podcast(&state, row, credits)))
}

/// A podcast's episodes, newest first.
#[utoipa::path(get, path = "/podcasts/{id}/episodes", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path), PageQuery),
    responses((status = 200, body = Page<Episode>), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn list_episodes(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<Episode>>, Problem> {
    let viewer = family.viewer();
    catalog::find_podcast(state.db(), viewer, id).await?.ok_or_else(Problem::not_found)?;
    let limit = page.limit()?;
    let rows = catalog::list_episodes(state.db(), viewer, id, page.after()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |e| (e.sort_at, e.id));
    Ok(Json(Page { items: page.items.into_iter().map(|e| episode(&state, e)).collect(), next: page.next, included: None }))
}

/// One episode.
#[utoipa::path(get, path = "/episodes/{id}", tag = "catalog", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Episode), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_episode(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Episode>, Problem> {
    let row = catalog::find_episode(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    Ok(Json(episode(&state, row)))
}

/// A book with its files, chapters and series.
pub(crate) async fn full_book(state: &AppState, row: catalog::BookRow) -> Result<Book, Problem> {
    let pool = state.db();
    let id = row.id;
    let credits = credits_of(state, "book", &[id]).await?.remove(&id).unwrap_or_default();
    let mut answer = book(state, row, credits);
    answer.files = Some(
        catalog::book_files(pool, id)
            .await?
            .into_iter()
            .map(|f| BookFile {
                id: f.id,
                position: f.position,
                title: f.title,
                duration_ms: f.duration_secs.map(|s| i64::from(s) * 1000),
                size_bytes: f.size_bytes,
                content_type: f.content_type,
            })
            .collect(),
    );
    answer.chapters = Some(
        crate::db::audiobooks::list_chapters(pool, id)
            .await?
            .into_iter()
            .map(|c| Chapter { id: c.id, position: c.position, title: c.title, start_ms: (c.start_time_secs * 1000.0).round() as i64, file_id: c.file_id })
            .collect(),
    );
    answer.series = Some(
        catalog::book_series(pool, id)
            .await?
            .into_iter()
            .map(|(series_id, name, position)| SeriesEntry { series_id, name, position })
            .collect(),
    );
    Ok(answer)
}
