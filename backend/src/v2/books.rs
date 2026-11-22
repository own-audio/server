// SPDX-License-Identifier: AGPL-3.0-or-later
//! Editing a book (docs/api-v2-completion-plan.md, phase 5): its details, its files, its
//! cover, its tags, and identifying it from the household's book source.
//!
//! Who may: v1's rules. The owner edits the details, adds files, sets the cover and the tags;
//! renaming, deleting and reordering files, and identifying, are also a household admin's on
//! a book shared with the household (tidying shared content is administration). A book the
//! caller cannot see is `404`; one they see but may not change is `403`. Adding files and
//! putting a cover from an upload also need the right to add to the library.
//!
//! Every change is one transaction, so `/sync/changes` sends the book once.

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::db::catalog::{self, BookRow};
use crate::families::FamilyContext;
use crate::v2::edit::present;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use crate::v2::spoken::{self, Book, BookFile};
use crate::v2::uploads::NewBookFile;
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

/// The most files one request adds.
const MAX_FILES: usize = 500;
/// The most tags one book carries.
const MAX_TAGS: usize = 50;
const MAX_TAG_CHARS: usize = 100;
/// A cover put from an upload.
const MAX_COVER_BYTES: i64 = 20 * 1024 * 1024;

/// The book as the caller sees it (`404` otherwise) and theirs (`403` otherwise).
async fn owned(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<BookRow, Problem> {
    let row = catalog::find_book(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    if row.user_id != family.user_id {
        return Err(Problem::forbidden().detail("only its owner changes this"));
    }
    Ok(row)
}

/// The book as the caller sees it, theirs, or shared with the household they administer.
async fn managed(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<BookRow, Problem> {
    let row = catalog::find_book(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    let shared_here = row.family_id == Some(family.family_id);
    if row.user_id != family.user_id && !(family.is_family_admin() && shared_here) {
        return Err(Problem::forbidden().detail("only its owner, or a household admin for a shared book, changes this"));
    }
    Ok(row)
}

pub(crate) async fn answer(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<Book, Problem> {
    let row = catalog::find_book(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)?;
    spoken::full_book(state, family.viewer(), row).await
}

fn text(field: &str, value: Option<&str>, max: usize) -> Result<Option<String>, Problem> {
    let value = value.map(str::trim).filter(|v| !v.is_empty());
    if value.is_some_and(|v| v.chars().count() > max) {
        return Err(Problem::bad_request("invalid_body", format!("{field} is at most {max} characters"))
            .field(format!("/{field}"), "too_long", format!("at most {max} characters")));
    }
    Ok(value.map(str::to_string))
}

/// An upload that is the caller's household's, or why not.
fn upload_problem(e: AuthError, field: &str) -> Problem {
    match e {
        AuthError::NotFound => Problem::not_found().detail("no such upload in this household"),
        AuthError::BadRequest(m) => Problem::bad_request("invalid_upload", m).field(field.to_string(), "invalid", ""),
        other => other.into(),
    }
}

// ── Details ──────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BookPatch {
    pub title: Option<String>,
    /// `null` clears it; so for the others.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub publisher: Option<Option<String>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<i32>)]
    pub published_year: Option<Option<i32>>,
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub isbn: Option<Option<String>>,
    /// BCP 47 (`en`, `pt-BR`); `null` clears it (revision 19). A book's `origin` is the server's
    /// and cannot be set here.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub language: Option<Option<String>>,
}

/// Correct a book's title, description, publisher, year, ISBN or language. Its people are its credits
/// (`PUT /books/{id}/credits`).
#[utoipa::path(patch, path = "/books/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = BookPatch,
    responses((status = 200, description = "The book now", body = Book),
        (status = 400, description = "`invalid_body`: an empty title, a field too long, a year out of range, a language that is not a BCP 47 tag", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the book's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_book(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<BookPatch>) -> Result<Json<Book>, Problem> {
    let row = owned(&state, &family, id).await?;
    let language = match body.language {
        Some(l) => Some(language_tag(l.as_deref())?),
        None => None,
    };
    let title = match body.title.as_deref() {
        Some(t) => text("title", Some(t), 500)?.ok_or_else(|| Problem::bad_request("invalid_body", "a title cannot be empty").field("/title", "required", "a title"))?,
        None => row.title,
    };
    let description = match body.description {
        Some(d) => text("description", d.as_deref(), 50_000)?,
        None => row.description,
    };
    let publisher = match body.publisher {
        Some(p) => text("publisher", p.as_deref(), 500)?,
        None => row.publisher,
    };
    let published_year = match body.published_year {
        Some(Some(y)) if !(1..=9999).contains(&y) => {
            return Err(Problem::bad_request("invalid_body", "a year is 1 to 9999").field("/published_year", "out_of_range", "1 to 9999"));
        }
        Some(y) => y,
        None => row.published_year,
    };
    let isbn = match body.isbn {
        Some(i) => text("isbn", i.as_deref(), 32)?,
        None => row.isbn,
    };
    db::shelves::update_book_details(state.db(), id, &title, description.as_deref(), publisher.as_deref(), published_year, isbn.as_deref()).await?;
    if let Some(language) = language {
        db::audiobooks::set_book_language(state.db(), id, language.as_deref()).await?;
    }
    Ok(Json(answer(&state, &family, id).await?))
}

/// A language as BCP 47 writes it, loosely: letters, digits and hyphens, a 2–3 letter language
/// first (`en`, `pt-BR`, `zh-Hant`). Blank is none.
fn language_tag(raw: Option<&str>) -> Result<Option<String>, Problem> {
    let Some(tag) = raw.map(str::trim).filter(|t| !t.is_empty()) else {
        return Ok(None);
    };
    let mut parts = tag.split('-');
    let first = parts.next().unwrap_or_default();
    let ok = tag.len() <= 35
        && (2..=3).contains(&first.len())
        && first.chars().all(|c| c.is_ascii_alphabetic())
        && parts.all(|p| (1..=8).contains(&p.len()) && p.chars().all(|c| c.is_ascii_alphanumeric()));
    if !ok {
        return Err(Problem::bad_request("invalid_body", "a language is a BCP 47 tag, such as en or pt-BR").field("/language", "invalid", "a BCP 47 tag"));
    }
    Ok(Some(tag.to_string()))
}

// ── Files ────────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct NewBookFiles {
    /// Each from `POST /uploads` with kind `book_file`; `relative_path` is required here and
    /// decides where it plays. A path the book has already answers its existing file.
    pub files: Vec<NewBookFile>,
}

/// Add files to a book, many at once. The play order follows the files' paths again, as
/// for a new book. Send an `Idempotency-Key` to retry safely.
#[utoipa::path(post, path = "/books/{id}/files", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = NewBookFiles,
    responses((status = 200, description = "The book now, with its files", body = Book),
        (status = 400, description = "`invalid_body` (no files, or one without `relative_path`), `too_many`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 402, description = "`storage_refused`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the book's owner, or not allowed to add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible, or an upload of another household", body = crate::v2::problem::ProblemBody)))]
pub async fn add_files(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<NewBookFiles>) -> Result<Json<Book>, Problem> {
    owned(&state, &family, id).await?;
    family.require_can_upload()?;
    if body.files.is_empty() {
        return Err(Problem::bad_request("invalid_body", "at least one file").field("/files", "required", "one or more"));
    }
    if body.files.len() > MAX_FILES {
        return Err(Problem::bad_request("too_many", format!("at most {MAX_FILES} files at a time")));
    }
    let mut files = Vec::with_capacity(body.files.len());
    for (i, f) in body.files.into_iter().enumerate() {
        if f.relative_path.as_deref().is_none_or(|p| p.trim().is_empty()) {
            return Err(Problem::bad_request("invalid_body", "every file needs its relative_path")
                .field(format!("/files/{i}/relative_path"), "required", "its path inside the book"));
        }
        files.push(crate::audiobooks::FromUploadsFile { object_key: f.upload_id, relative_path: f.relative_path, title: f.title, duration_secs: None });
    }
    crate::audiobooks::append_files(&state, &family, id, files).await.map_err(|e| upload_problem(e, "/files"))?;
    db::media::queue_book_checksums(state.db(), id).await;
    Ok(Json(answer(&state, &family, id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct FilePatch {
    pub title: String,
}

/// Rename one of a book's files.
#[utoipa::path(patch, path = "/books/{id}/files/{file_id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path), ("file_id" = Uuid, Path)), request_body = FilePatch,
    responses((status = 200, description = "The file now", body = BookFile),
        (status = 400, description = "`invalid_body`: an empty title", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Neither the owner nor a household admin of a shared book", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown book or file", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_file(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((id, file_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<FilePatch>,
) -> Result<Json<BookFile>, Problem> {
    let book = managed(&state, &family, id).await?;
    let title = text("title", Some(&body.title), 500)?.ok_or_else(|| Problem::bad_request("invalid_body", "a title cannot be empty").field("/title", "required", "a title"))?;
    db::audiobooks::find_file(state.db(), id, file_id).await?.ok_or_else(Problem::not_found)?;
    db::audiobooks::update_file_title(state.db(), id, book.user_id, file_id, &title).await?;
    answer(&state, &family, id)
        .await?
        .files
        .unwrap_or_default()
        .into_iter()
        .find(|f| f.id == file_id)
        .map(Json)
        .ok_or_else(Problem::not_found)
}

/// Remove one file from a book (a botched or doubled upload). The last file stays: delete the
/// book instead (`DELETE /books/{id}`), which the trash can bring back.
#[utoipa::path(delete, path = "/books/{id}/files/{file_id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path), ("file_id" = Uuid, Path)),
    responses((status = 204, description = "Removed"),
        (status = 403, description = "Neither the owner nor a household admin of a shared book", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown book or file", body = crate::v2::problem::ProblemBody),
        (status = 409, description = "`last_file`: a book keeps at least one file", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_file(family: FamilyContext, State(state): State<AppState>, Path((id, file_id)): Path<(Uuid, Uuid)>) -> Result<StatusCode, Problem> {
    let book = managed(&state, &family, id).await?;
    db::audiobooks::find_file(state.db(), id, file_id).await?.ok_or_else(Problem::not_found)?;
    if catalog::book_files(state.db(), id).await?.len() <= 1 {
        return Err(Problem::new(StatusCode::CONFLICT, "last_file").detail("a book keeps at least one file; delete the book instead"));
    }
    if let Some(object) = db::audiobooks::delete_file(state.db(), id, book.user_id, file_id).await? {
        crate::trash::delete_orphans(state.db(), state.storage(), &[object]).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct OrderFiles {
    /// File ids in their new playing order. Files left out follow them in the order they had.
    pub file_ids: Vec<Uuid>,
}

/// Put a book's files in a new playing order.
#[utoipa::path(put, path = "/books/{id}/files/order", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = OrderFiles,
    responses((status = 204, description = "Reordered"),
        (status = 400, description = "`invalid_file`: an id twice, or not this book's", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Neither the owner nor a household admin of a shared book", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn order_files(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<OrderFiles>) -> Result<StatusCode, Problem> {
    managed(&state, &family, id).await?;
    if !db::shelves::order_book_files(state.db(), id, &body.file_ids).await? {
        return Err(Problem::bad_request("invalid_file", "every id once, each a file of this book").field("/file_ids", "invalid", "unknown or repeated file"));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Identity ─────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BookSearch {
    /// Seeded from the book, edited by the person asking; at least one of the two.
    pub title: Option<String>,
    pub author: Option<String>,
    /// 1 to 40; 10 when absent.
    pub limit: Option<u8>,
}

#[derive(Serialize, ToSchema)]
pub struct BookCandidates {
    /// Best first, from the household's book source (Open Library, Apple or Google Books).
    pub items: Vec<BookMatch>,
}

/// A book the source knows that may be this one.
#[derive(Serialize, ToSchema)]
pub struct BookMatch {
    /// Send it back to `PUT /books/{id}/identity` as it came; its prefix names the source.
    pub volume_id: String,
    /// `openlibrary`, `apple` or `google`.
    pub source: String,
    pub title: String,
    /// Shown to tell two editions apart; a book keeps no subtitle.
    pub subtitle: Option<String>,
    /// Several joined with `, `.
    pub author: Option<String>,
    pub publisher: Option<String>,
    pub published_year: Option<i32>,
    pub description: Option<String>,
    pub isbn: Option<String>,
    pub page_count: Option<i32>,
    pub categories: Vec<String>,
    /// The source's picture; may fail to load, which means no cover.
    pub cover_url: Option<String>,
    /// How well it fits the words searched for, 0 to 100.
    pub score: u8,
}

impl From<crate::metadata::google_books::BookCandidate> for BookMatch {
    fn from(c: crate::metadata::google_books::BookCandidate) -> Self {
        BookMatch {
            volume_id: c.volume_id,
            source: c.source.as_str().to_string(),
            title: c.title,
            subtitle: c.subtitle,
            author: c.author,
            publisher: c.publisher,
            published_year: c.published_year,
            description: c.description,
            isbn: c.isbn,
            page_count: c.page_count,
            categories: c.categories,
            cover_url: c.cover_url,
            score: c.score,
        }
    }
}

#[derive(Deserialize, ToSchema)]
pub struct BookIdentity {
    /// From a search's match.
    pub volume_id: String,
    /// What to take from the match; everything when absent.
    #[serde(default)]
    pub fields: IdentityFields,
}

/// Each `true` unless sent as `false`.
#[derive(Deserialize, ToSchema)]
pub struct IdentityFields {
    #[serde(default = "yes")]
    pub title: bool,
    /// The author line, which the book's author credits follow.
    #[serde(default = "yes")]
    pub author: bool,
    #[serde(default = "yes")]
    pub description: bool,
    #[serde(default = "yes")]
    pub publisher: bool,
    #[serde(default = "yes")]
    pub published_year: bool,
    #[serde(default = "yes")]
    pub isbn: bool,
    /// The source's cover; needs the right to add to the library.
    #[serde(default = "yes")]
    pub cover: bool,
}

fn yes() -> bool {
    true
}

impl Default for IdentityFields {
    fn default() -> Self {
        IdentityFields { title: true, author: true, description: true, publisher: true, published_year: true, isbn: true, cover: true }
    }
}

/// A book source's failure, as a problem: a spent quota or a refused key is the server's
/// to fix (`503 source_refused`), anything else the source not answering (`502`).
fn source_problem(error: anyhow::Error) -> Problem {
    use crate::metadata::google_books::{KeyRejected, QuotaExhausted};
    if let Some(not_an_id) = error.downcast_ref::<crate::metadata::books::NotAnId>() {
        return Problem::bad_request("invalid_volume_id", not_an_id.to_string()).field("/volume_id", "invalid", "an id a search gave");
    }
    if error.chain().any(|e| e.is::<QuotaExhausted>() || e.is::<KeyRejected>()) {
        return Problem::new(StatusCode::SERVICE_UNAVAILABLE, "source_refused")
            .detail("Google Books turned this server away: its quota is spent or its key is refused. A household admin can choose another book source.");
    }
    tracing::warn!("book source: {error:#}");
    Problem::new(StatusCode::BAD_GATEWAY, "source_unavailable").detail("the book source did not answer")
}

pub(crate) fn search_terms(body: &BookSearch) -> Result<(Option<&str>, Option<&str>, u8), Problem> {
    let title = body.title.as_deref().map(str::trim).filter(|t| !t.is_empty());
    let author = body.author.as_deref().map(str::trim).filter(|a| !a.is_empty());
    if title.is_none() && author.is_none() {
        return Err(Problem::bad_request("invalid_body", "a title or an author to search for").field("/title", "required", "title or author"));
    }
    let limit = body.limit.unwrap_or(10);
    if !(1..=40).contains(&limit) {
        return Err(Problem::bad_request("invalid_limit", "limit is 1 to 40").field("/limit", "out_of_range", "1 to 40"));
    }
    Ok((title, author, limit))
}

/// Search the household's book source for this book, with the caller's own words. Writes
/// nothing; `PUT /books/{id}/identity` keeps a match.
#[utoipa::path(post, path = "/books/{id}/identity/search", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = BookSearch,
    responses((status = 200, body = BookCandidates),
        (status = 400, description = "`invalid_body`: neither title nor author; `invalid_limit`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Neither the owner nor a household admin of a shared book", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`source_unavailable`: the book source did not answer", body = crate::v2::problem::ProblemBody),
        (status = 503, description = "`source_refused`: Google Books' quota is spent or its key refused", body = crate::v2::problem::ProblemBody)))]
pub async fn search_identity(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<BookSearch>) -> Result<Json<BookCandidates>, Problem> {
    managed(&state, &family, id).await?;
    let (title, author, limit) = search_terms(&body)?;
    let (source, key) = crate::families::book_source::for_identify(&state, &family).await?;
    let items = crate::metadata::books::search(source, title, author, limit, key.as_deref()).await.map_err(source_problem)?;
    Ok(Json(BookCandidates { items: items.into_iter().map(Into::into).collect() }))
}

/// Identify a book as a search's match: the fields asked for (all, when `fields` is absent)
/// come from the source, fetched again by id; the cover is put on in the background of the
/// answer, best effort, and needs the right to add to the library. The files are never touched.
#[utoipa::path(put, path = "/books/{id}/identity", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = BookIdentity,
    responses((status = 200, description = "The book now", body = Book),
        (status = 400, description = "`invalid_volume_id`: not an id a search gave", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Neither the owner nor a household admin of a shared book; or the cover without the right to add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody),
        (status = 502, description = "`source_unavailable`", body = crate::v2::problem::ProblemBody),
        (status = 503, description = "`source_refused`", body = crate::v2::problem::ProblemBody)))]
pub async fn put_identity(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<BookIdentity>,
) -> Result<Json<Book>, Problem> {
    let book = managed(&state, &family, id).await?;
    if body.fields.cover {
        family.require_can_upload()?;
    }
    let (_, key) = crate::families::book_source::for_identify(&state, &family).await?;
    let volume = crate::metadata::books::fetch(body.volume_id.trim(), key.as_deref()).await.map_err(source_problem)?;
    let fields = &body.fields;
    // Written as the book's owner: a household admin tidying a shared book does not own it.
    db::audiobooks::apply_google_books_metadata(
        state.db(),
        id,
        book.user_id,
        &volume.volume_id,
        fields.title.then_some(volume.title.as_str()),
        fields.author.then_some(volume.author.as_deref()).flatten(),
        fields.description.then_some(volume.description.as_deref()).flatten(),
        fields.publisher.then_some(volume.publisher.as_deref()).flatten(),
        fields.published_year.then_some(volume.published_year).flatten(),
        fields.isbn.then_some(volume.isbn.as_deref()).flatten(),
    )
    .await?;
    if fields.cover {
        if let Some(cover_url) = volume.cover_url.as_deref() {
            crate::audiobooks::apply_book_cover(&state, &family, id, volume.source, cover_url).await;
        }
    }
    Ok(Json(answer(&state, &family, id).await?))
}

// ── Cover ────────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct ImageUpload {
    /// From `POST /uploads` (kind `book_cover`), an image…
    pub upload_id: Option<String>,
    /// …or a companion image of one's own (`POST /files`): "use as cover". One of the two.
    pub file_id: Option<Uuid>,
}

/// Put a cover on a book, from an upload. A new picture gets a new link; the old one stops.
#[utoipa::path(put, path = "/books/{id}/image", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = ImageUpload,
    responses((status = 200, description = "The book now", body = Book),
        (status = 400, description = "`not_an_image`, `invalid_upload`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the book's owner, or not allowed to add to the library", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible, or no such upload in this household", body = crate::v2::problem::ProblemBody),
        (status = 413, description = "`too_large`: over 20 MB", body = crate::v2::problem::ProblemBody)))]
pub async fn put_image(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<ImageUpload>) -> Result<Json<Book>, Problem> {
    owned(&state, &family, id).await?;
    family.require_can_upload()?;
    let image = image_of(&state, &family, &body).await?;
    if let Some(old) = db::shelves::set_book_cover(state.db(), id, Some(image)).await? {
        crate::trash::delete_orphans(state.db(), state.storage(), &[old]).await;
    }
    Ok(Json(answer(&state, &family, id).await?))
}

/// The picture a `PUT …/image` names: an upload, or one of the caller's companion images.
pub(crate) async fn image_of(state: &AppState, family: &FamilyContext, body: &ImageUpload) -> Result<Uuid, Problem> {
    match (body.upload_id.as_deref().map(str::trim).filter(|u| !u.is_empty()), body.file_id) {
        (Some(upload), None) => adopt_image(state, family, upload).await,
        (None, Some(file)) => {
            let (object, path) = db::files::own_file(state.db(), family.user_id, file)
                .await?
                .ok_or_else(|| Problem::not_found().detail("no such file of yours"))?;
            if !crate::filesync::paths::is_image(&path) {
                return Err(Problem::bad_request("not_an_image", "only an image can be a cover").field("/file_id", "invalid", ""));
            }
            Ok(object)
        }
        _ => Err(Problem::bad_request("invalid_body", "give `upload_id` or `file_id`, one of the two").field("/upload_id", "required", "")),
    }
}

/// An uploaded image of the caller's household, registered.
pub(crate) async fn adopt_image(state: &AppState, family: &FamilyContext, upload_id: &str) -> Result<Uuid, Problem> {
    let adopted = crate::uploads::adopt(state, family, upload_id.trim()).await.map_err(|e| upload_problem(e, "/upload_id"))?;
    if !adopted.content_type.starts_with("image/") {
        return Err(Problem::bad_request("not_an_image", format!("the upload is {}", adopted.content_type)).field("/upload_id", "invalid", ""));
    }
    if adopted.size_bytes > MAX_COVER_BYTES {
        return Err(Problem::new(StatusCode::PAYLOAD_TOO_LARGE, "too_large").detail("a picture is at most 20 MB"));
    }
    Ok(adopted.media_object_id)
}

/// Take a book's cover away.
#[utoipa::path(delete, path = "/books/{id}/image", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Removed, or there was none"),
        (status = 403, description = "Not the book's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_image(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    owned(&state, &family, id).await?;
    if let Some(old) = db::shelves::set_book_cover(state.db(), id, None).await? {
        crate::trash::delete_orphans(state.db(), state.storage(), &[old]).await;
    }
    Ok(StatusCode::NO_CONTENT)
}

// ── Tags ─────────────────────────────────────────────────────────────────────

#[derive(Deserialize, ToSchema)]
pub struct BookTags {
    /// The book's tags, complete; `[]` takes them all away. Names are kept as the first
    /// book to use them spelled them, and match in any case.
    pub tags: Vec<String>,
}

pub(crate) fn tag_names(tags: &[String]) -> Result<Vec<String>, Problem> {
    let mut out: Vec<String> = Vec::new();
    for (i, raw) in tags.iter().enumerate() {
        let name = raw.trim();
        if name.is_empty() || name.chars().count() > MAX_TAG_CHARS {
            return Err(Problem::bad_request("invalid_tag", format!("a tag is 1 to {MAX_TAG_CHARS} characters"))
                .field(format!("/tags/{i}"), "invalid", format!("1 to {MAX_TAG_CHARS} characters")));
        }
        if !out.iter().any(|o| o.to_lowercase() == name.to_lowercase()) {
            out.push(name.to_string());
        }
    }
    if out.len() > MAX_TAGS {
        return Err(Problem::bad_request("too_many", format!("at most {MAX_TAGS} tags on a book")));
    }
    Ok(out)
}

/// Replace a book's tags.
#[utoipa::path(put, path = "/books/{id}/tags", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = BookTags,
    responses((status = 200, description = "The book now", body = Book),
        (status = 400, description = "`invalid_tag`, `too_many`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not the book's owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn put_tags(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<BookTags>) -> Result<Json<Book>, Problem> {
    owned(&state, &family, id).await?;
    let names = tag_names(&body.tags)?;
    db::shelves::set_book_tags(state.db(), id, &names).await?;
    Ok(Json(answer(&state, &family, id).await?))
}

#[derive(Serialize, ToSchema)]
pub struct BookTag {
    pub id: Uuid,
    pub name: String,
    /// Books the caller can see that carry it.
    pub book_count: i64,
}

/// The tags on books the caller can see, by name, with how many.
#[utoipa::path(get, path = "/book-tags", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<BookTag>)))]
pub async fn list_tags(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<BookTag>>, Problem> {
    let limit = page.limit()?;
    let rows = db::shelves::list_tags(state.db(), family.viewer(), page.after::<(String, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |t| (t.sort_key.clone(), t.id));
    Ok(Json(Page {
        items: page.items.into_iter().map(|t| BookTag { id: t.id, name: t.name, book_count: t.book_count }).collect(),
        next: page.next,
        included: None,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_language_is_a_loose_bcp47_tag() {
        assert_eq!(language_tag(Some(" pt-BR ")).unwrap().as_deref(), Some("pt-BR"));
        assert_eq!(language_tag(Some("zh-Hant")).unwrap().as_deref(), Some("zh-Hant"));
        assert_eq!(language_tag(Some("  ")).unwrap(), None);
        assert_eq!(language_tag(None).unwrap(), None);
        for bad in ["e", "english", "en_US", "en--US", "12", "en-toolongpart"] {
            assert_eq!(language_tag(Some(bad)).unwrap_err().code(), "invalid_body", "{bad}");
        }
    }

    #[test]
    fn an_origin_names_only_what_it_has() {
        use crate::audiobooks::models::BookOrigin;
        let translated = BookOrigin {
            kind: BookOrigin::MACHINE_TRANSLATED.into(),
            source_language: Some("cs".into()),
            translator: Some("google".into()),
            voice: Some("Sarah".into()),
        };
        assert_eq!(
            serde_json::to_value(&translated).unwrap(),
            serde_json::json!({"kind": "machine_translated", "source_language": "cs", "translator": "google", "voice": "Sarah"})
        );
        let narrated: BookOrigin = serde_json::from_value(serde_json::json!({"kind": "narrated"})).unwrap();
        assert_eq!(serde_json::to_value(&narrated).unwrap(), serde_json::json!({"kind": "narrated"}));
    }

    #[test]
    fn tags_are_trimmed_kept_once_and_bounded() {
        let names = tag_names(&[" Sci-Fi ".into(), "sci-fi".into(), "Classics".into()]).unwrap();
        assert_eq!(names, vec!["Sci-Fi".to_string(), "Classics".to_string()]);
        assert_eq!(tag_names(&["  ".into()]).unwrap_err().code(), "invalid_tag");
        assert_eq!(tag_names(&["x".repeat(MAX_TAG_CHARS + 1)]).unwrap_err().code(), "invalid_tag");
        let many: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i}")).collect();
        assert_eq!(tag_names(&many).unwrap_err().code(), "too_many");
        assert!(tag_names(&[]).unwrap().is_empty());
    }

    #[test]
    fn a_search_needs_words_and_a_sane_limit() {
        let body = |t: Option<&str>, a: Option<&str>, l: Option<u8>| BookSearch { title: t.map(Into::into), author: a.map(Into::into), limit: l };
        assert_eq!(search_terms(&body(None, Some(" "), None)).unwrap_err().code(), "invalid_body");
        assert_eq!(search_terms(&body(Some("Emma"), None, Some(0))).unwrap_err().code(), "invalid_limit");
        assert_eq!(search_terms(&body(Some("Emma"), None, Some(41))).unwrap_err().code(), "invalid_limit");
        assert_eq!(search_terms(&body(Some(" Emma "), None, None)).unwrap(), (Some("Emma"), None, 10));
    }

    #[test]
    fn a_source_failure_is_told_apart() {
        let not_an_id = anyhow::Error::from(crate::metadata::books::NotAnId("ol:x".into()));
        assert_eq!(source_problem(not_an_id).code(), "invalid_volume_id");
        let quota = anyhow::Error::from(crate::metadata::google_books::QuotaExhausted).context("google books: search");
        assert_eq!(source_problem(quota).code(), "source_refused");
        assert_eq!(source_problem(anyhow::anyhow!("connection refused")).code(), "source_unavailable");
    }
}
