// SPDX-License-Identifier: AGPL-3.0-or-later
/// Audiobooks module — books, files, chapters, manifests, multi-file playback.
pub mod authors;
pub mod collections;
pub mod models;

use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::db::access;
use crate::families::FamilyContext;
use crate::http::multipart::read_text_field;
use axum::body::Body;
use axum::Router;
use axum::extract::{DefaultBodyLimit, Json, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use mime_guess::MimeGuess;
use serde::{Deserialize, Serialize};
use std::path::{Path as StdPath, PathBuf};
use tempfile::NamedTempFile;
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

// ── DTOs ──────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CreateBookRequest {
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
    pub source_url: Option<String>,
    /// `private` (default) or `family`.
    #[serde(default)]
    pub visibility: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateBookRequest {
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
}

/// Body of `POST /audiobooks/{id}/metadata/search`. The book id scopes
/// visibility only — the query itself is whatever the client sends, because the
/// books worth identifying are the ones whose stored title and author are wrong.
#[derive(Deserialize)]
pub struct IdentifySearchRequest {
    pub title: Option<String>,
    pub author: Option<String>,
    pub limit: Option<u8>,
}

/// Body of `POST /audiobooks/{id}/metadata/apply`. Only the volume id crosses
/// the wire: the server re-fetches the volume rather than trusting a client's
/// copy of a search result.
#[derive(Deserialize)]
pub struct ApplyIdentifyRequest {
    pub volume_id: String,
    /// Which parts of the match to actually write. Absent means all of them,
    /// so a client with no field picker still behaves sensibly.
    #[serde(default)]
    pub fields: IdentifyFields,
}

/// Per-field opt-out. `narrator` is not here because Google Books has none to
/// give (see `metadata::google_books`).
#[derive(Deserialize)]
pub struct IdentifyFields {
    #[serde(default = "yes")]
    pub title: bool,
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
    #[serde(default = "yes")]
    pub cover: bool,
}

fn yes() -> bool {
    true
}

impl Default for IdentifyFields {
    fn default() -> Self {
        Self {
            title: true,
            author: true,
            description: true,
            publisher: true,
            published_year: true,
            isbn: true,
            cover: true,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
struct UploadManifestEntry {
    relative_path: Option<String>,
    duration_secs: Option<i32>,
}

/// Body of `POST /audiobooks/from-uploads` — the direct-to-storage twin of
/// the multipart `POST /audiobooks/upload`.
#[derive(Deserialize)]
pub struct FromUploadsRequest {
    pub title: Option<String>,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
    /// `private` (default) or `family`.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Key returned by `/uploads/presign` for the cover, if one was uploaded.
    pub cover_object_key: Option<String>,
    /// The book's folder in the own.audio folder (`Audiobooks/…`), for a
    /// folder put there; each file's `relative_path` is then its path inside
    /// it and is kept as given. A single loose file: `path` is the file and
    /// its `relative_path` is empty. Without `path` the server gives the book
    /// a default folder and names its files.
    #[serde(default)]
    pub path: Option<String>,
    pub files: Vec<FromUploadsFile>,
}

#[derive(Deserialize)]
pub struct FromUploadsFile {
    /// Key returned by `/uploads/presign` and already PUT to storage.
    pub object_key: String,
    /// Path within the uploaded folder — decides play order, exactly as in
    /// the multipart manifest.
    pub relative_path: Option<String>,
    pub title: Option<String>,
    pub duration_secs: Option<i32>,
}

#[derive(Deserialize)]
struct UploadFileRequest {
    position: i32,
    relative_path: Option<String>,
    duration_secs: Option<i32>,
}

/// A created book and where it sits in the own.audio folder.
#[derive(Serialize)]
pub struct BookWithPath {
    #[serde(flatten)]
    pub book: BookResponse,
    pub path: Option<String>,
}

#[derive(Serialize)]
pub struct BookResponse {
    pub id: String,
    pub title: String,
    pub author: Option<String>,
    pub narrator: Option<String>,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub total_duration_secs: Option<i32>,
    pub source_url: Option<String>,
    /// Set once the book has been matched through the identify flow.
    pub google_books_volume_id: Option<String>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub published_year: Option<i32>,
    /// `private` or `family` — which folder this book lives in.
    pub visibility: String,
    /// False for family-shared books owned by someone else.
    pub is_owner: bool,
    /// Whose it is — for acting on a family member's item (a family shortcut).
    pub owner_id: String,
    pub created_at: String,
    pub updated_at: String,
    /// `upload` or `folder`; a folder book's files are read-only (API revision 3).
    pub source: String,
    pub read_only: bool,
}

#[derive(Deserialize)]
pub struct SetVisibilityRequest {
    /// `private` or `family`.
    pub visibility: String,
}

#[derive(Serialize)]
pub struct FileResponse {
    pub id: String,
    pub book_id: String,
    pub position: i32,
    pub title: Option<String>,
    pub duration_secs: Option<i32>,
    pub audio_object_id: String,
    pub size_bytes: Option<i64>,
    /// Nullable — the `media_checksum` job fills this in asynchronously after upload
    /// (mirror-plan B-2). A downloading client should verify against it when present and fall
    /// back to a size-only check when it isn't.
    pub sha256: Option<String>,
}

#[derive(Serialize)]
pub struct ChapterResponse {
    pub id: String,
    pub book_id: String,
    pub file_id: Option<String>,
    pub position: i32,
    pub title: String,
    pub start_time_secs: f64,
}

#[derive(Serialize)]
pub struct StreamResponse {
    pub url: String,
    pub expires_in_secs: u64,
}

struct TempUpload {
    file_name: String,
    temp_file: NamedTempFile,
    content_type: String,
    size_bytes: i64,
}

// ── Router ────────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct ReorderFilesRequest {
    pub file_ids: Vec<String>,
}

#[derive(Deserialize)]
pub struct UpdateFileRequest {
    pub title: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/", get(list_books))
        .route("/", post(create_book))
        .route("/upload", post(upload_book).layer(DefaultBodyLimit::disable()))
        .route("/from-uploads", post(create_book_from_uploads))
        .route("/{id}/files/from-uploads", post(append_files_from_uploads))
        .route("/{id}", get(get_book))
        .route("/{id}", put(update_book))
        .route("/{id}", delete(delete_book))
        .route("/{id}/cover", get(get_cover))
        .route("/{id}/upload-cover", post(upload_cover).layer(DefaultBodyLimit::disable()))
        .route("/{id}/files", get(list_files))
        .route("/{id}/upload-file", post(upload_file).layer(DefaultBodyLimit::disable()))
        .route("/{id}/files/{file_id}", put(update_file_title_handler))
        .route("/{id}/files/{file_id}", delete(delete_file_handler))
        .route("/{id}/files/{file_id}/stream", get(stream_file))
        .route("/{id}/files/reorder", put(reorder_files))
        .route("/{id}/chapters", get(list_chapters))
        .route("/{id}/visibility", put(set_book_visibility))
        .route("/{id}/metadata/search", post(search_book_metadata))
        .route("/{id}/metadata/apply", post(apply_book_metadata))
        // Sub-module routers
        .nest("/authors", authors::router())
        .nest("/organize", collections::router())
}

// ── Handlers ──────────────────────────────────────────────────────────────

/// GET /api/v1/audiobooks/
async fn list_books(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<BookResponse>>, AuthError> {
    let books = db::audiobooks::list_books(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(books.into_iter().map(|b| book_to_response(b, family.user_id)).collect()))
}

/// POST /api/v1/audiobooks/
async fn create_book(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateBookRequest>,
) -> Result<(StatusCode, Json<BookResponse>), AuthError> {
    let family_id =
        access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    let book = db::audiobooks::insert_book(
        state.db(),
        family.user_id,
        family_id,
        &body.title,
        body.author.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    // Optionally patch extra fields if provided
    if body.narrator.is_some() || body.description.is_some() || body.source_url.is_some() {
        sqlx::query(
            "UPDATE audiobook_books
             SET narrator=$2, description=$3, source_url=$4
             WHERE id=$1",
        )
        .bind(book.id)
        .bind(body.narrator.as_deref())
        .bind(body.description.as_deref())
        .bind(body.source_url.as_deref())
        .execute(state.db())
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

        let fresh = db::audiobooks::find_book(state.db(), book.id, family.viewer())
            .await
            .map_err(AuthError::Internal)?
            .ok_or_else(|| AuthError::Internal(anyhow::anyhow!("book vanished after insert")))?;

        return Ok((StatusCode::CREATED, Json(book_to_response(fresh, family.user_id))));
    }

    Ok((StatusCode::CREATED, Json(book_to_response(book, family.user_id))))
}

async fn upload_book(
    family: FamilyContext,
    State(state): State<AppState>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<BookResponse>), AuthError> {
    family.require_can_upload()?;
    let mut title: Option<String> = None;
    let mut author: Option<String> = None;
    let mut narrator: Option<String> = None;
    let mut description: Option<String> = None;
    let mut manifest: Vec<UploadManifestEntry> = Vec::new();
    let mut visibility: Option<String> = None;
    let mut audio_uploads: Vec<TempUpload> = Vec::new();
    let mut cover_upload: Option<TempUpload> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();

        match name.as_str() {
            "title" => title = Some(read_text_field(field).await?),
            "author" => author = Some(read_text_field(field).await?),
            "narrator" => narrator = Some(read_text_field(field).await?),
            "description" => description = Some(read_text_field(field).await?),
            "manifest" => {
                let raw = read_text_field(field).await?;
                manifest = serde_json::from_str::<Vec<UploadManifestEntry>>(&raw)
                    .map_err(|e| AuthError::BadRequest(format!("invalid upload manifest: {e}")))?;
            }
            "visibility" => visibility = Some(read_text_field(field).await?),
            "cover" => cover_upload = Some(save_field_to_temp(field).await?),
            "files" => audio_uploads.push(save_field_to_temp(field).await?),
            _ => {
                let _ = field;
            }
        }
    }

    if audio_uploads.is_empty() {
        return Err(AuthError::BadRequest("at least one audio file is required".to_string()));
    }

    // Which folder the upload lands in. Absent ⇒ private (fail closed).
    let upload_family_id =
        access::family_id_for_visibility(visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    if manifest.len() < audio_uploads.len() {
        manifest.resize(
            audio_uploads.len(),
            UploadManifestEntry {
                relative_path: None,
                duration_secs: None,
            },
        );
    }

    let mut uploads: Vec<(UploadManifestEntry, TempUpload)> = manifest
        .into_iter()
        .zip(audio_uploads)
        .collect();

    uploads.sort_by_key(|a| sort_path(&a.0.relative_path, &a.1.file_name));

    let normalized_title = title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| infer_upload_title(&uploads));

    if normalized_title.is_empty() {
        return Err(AuthError::BadRequest("book title could not be inferred from the upload".to_string()));
    }

    let total_duration_secs = uploads
        .iter()
        .filter_map(|(meta, _)| meta.duration_secs)
        .reduce(|acc, value| acc + value);

    let mut tx = state
        .db()
        .begin()
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;

    let book = db::audiobooks::insert_book_full(
        &mut *tx,
        family.user_id,
        upload_family_id,
        &normalized_title,
        trim_optional(author.as_deref()),
        trim_optional(narrator.as_deref()),
        trim_optional(description.as_deref()),
        total_duration_secs,
    )
    .await
    .map_err(AuthError::Internal)?;

    for (index, (meta, upload)) in uploads.iter().enumerate() {
        let object_key = crate::storage::family_key(
            family.family_id,
            format!(
                "audiobooks/{}/files/{:03}-{}",
                book.id,
                index + 1,
                sanitize_filename_component(
                    meta.relative_path.as_deref().unwrap_or(&upload.file_name)
                )
            ),
        );
        let media_id = store_temp_upload(
            state.storage(),
            &mut tx,
            &object_key,
            upload,
        )
        .await?;

        let relative_name = meta
            .relative_path
            .as_deref()
            .unwrap_or(&upload.file_name);

        sqlx::query(
            "INSERT INTO audiobook_files (book_id, position, title, duration_secs, audio_object_id)
             VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(book.id)
        .bind((index + 1) as i32)
        .bind(infer_file_title(relative_name))
        .bind(meta.duration_secs)
        .bind(media_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    if let Some(cover_upload) = cover_upload.as_ref() {
        let ext = file_extension(&cover_upload.file_name).unwrap_or("jpg");
        let object_key =
            crate::storage::family_key(family.family_id, format!("audiobooks/{}/cover.{ext}", book.id));
        let media_id = store_temp_upload(state.storage(), &mut tx, &object_key, cover_upload).await?;
        sqlx::query(
            "UPDATE audiobook_books
             SET cover_object_id = $2, updated_at = CURRENT_TIMESTAMP
             WHERE id = $1",
        )
        .bind(book.id)
        .bind(media_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    tx.commit()
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    let _ = crate::filesync::paths::ensure_book(state.db(), book.id)
        .await
        .inspect_err(|e| tracing::warn!(book_id = %book.id, "no path for the new book: {e:#}"));

    let fresh = db::audiobooks::find_book(state.db(), book.id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok((StatusCode::CREATED, Json(book_to_response(fresh, family.user_id))))
}

/// POST /api/v1/audiobooks/from-uploads
///
/// Creates a book from objects the client already PUT straight to storage via
/// `/uploads/presign` + `/uploads/complete`. Same result as `/upload`, without
/// the bytes ever crossing this server — see `crate::uploads` for why that
/// matters in production.
async fn create_book_from_uploads(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<FromUploadsRequest>,
) -> Result<(StatusCode, Json<BookWithPath>), AuthError> {
    family.require_can_upload()?;
    if body.files.is_empty() {
        return Err(AuthError::BadRequest("at least one file is required".to_string()));
    }

    // A folder from the own.audio folder keeps its names exactly (§2 item 10).
    let folder = body
        .path
        .as_deref()
        .map(|p| crate::filesync::paths::normalize(p, crate::filesync::paths::Kind::Audiobook))
        .transpose()
        .map_err(AuthError::BadRequest)?;
    let loose_file = folder.is_some()
        && body.files.len() == 1
        && body.files[0].relative_path.as_deref().is_none_or(|r| r.trim().is_empty());
    let mut kept_names: Vec<Option<String>> = Vec::with_capacity(body.files.len());
    if folder.is_some() {
        let mut seen = std::collections::HashSet::new();
        for file in &body.files {
            let name = if loose_file {
                String::new()
            } else {
                let raw = file.relative_path.as_deref().ok_or_else(|| {
                    AuthError::BadRequest("every file needs a relative_path when path is given".to_string())
                })?;
                crate::filesync::paths::normalize_relative(raw).map_err(AuthError::BadRequest)?
            };
            if !seen.insert(name.to_lowercase()) {
                return Err(AuthError::BadRequest(format!("'{name}' appears twice")));
            }
            kept_names.push(Some(name));
        }
    } else {
        kept_names.resize(body.files.len(), None);
    }

    let upload_family_id =
        access::family_id_for_visibility(body.visibility.as_deref(), family.family_id)
            .map_err(|e| AuthError::BadRequest(e.to_string()))?;

    // Same ordering rule as the multipart path: send order is not trusted,
    // relative_path decides. Getting this wrong silently shuffles a
    // 30-chapter book.
    let mut files: Vec<(FromUploadsFile, Option<String>)> = body.files.into_iter().zip(kept_names).collect();
    for (file, _) in &files {
        crate::uploads::require_own_key(&family, &file.object_key)?;
    }
    files.sort_by(|(a, _), (b, _)| {
        sort_path(&a.relative_path, &a.object_key).cmp(&sort_path(&b.relative_path, &b.object_key))
    });

    let normalized_title = body
        .title
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            let first = &files[0].0;
            infer_file_title(first.relative_path.as_deref().unwrap_or(&first.object_key))
        });

    let total_duration_secs = files
        .iter()
        .filter_map(|(file, _)| file.duration_secs)
        .reduce(|acc, value| acc + value);

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;

    let book = db::audiobooks::insert_book_full(
        &mut *tx,
        family.user_id,
        upload_family_id,
        &normalized_title,
        trim_optional(body.author.as_deref()),
        trim_optional(body.narrator.as_deref()),
        trim_optional(body.description.as_deref()),
        total_duration_secs,
    )
    .await
    .map_err(AuthError::Internal)?;

    for (index, (file, kept_name)) in files.iter().enumerate() {
        let media_id = register_uploaded_object(&state, &mut tx, &file.object_key).await?;
        let relative_name = file.relative_path.as_deref().unwrap_or(&file.object_key);

        sqlx::query(
            "INSERT INTO audiobook_files (book_id, position, title, duration_secs, audio_object_id, relative_path)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(book.id)
        .bind((index + 1) as i32)
        .bind(
            file.title
                .as_deref()
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| infer_file_title(relative_name)),
        )
        .bind(file.duration_secs)
        .bind(media_id)
        .bind(kept_name.as_deref())
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    if let Some(cover_key) = body.cover_object_key.as_deref() {
        crate::uploads::require_own_key(&family, cover_key)?;
        let media_id = register_uploaded_object(&state, &mut tx, cover_key).await?;
        sqlx::query(
            "UPDATE audiobook_books
             SET cover_object_id = $2, updated_at = CURRENT_TIMESTAMP
             WHERE id = $1",
        )
        .bind(book.id)
        .bind(media_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    }

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;

    let path = match folder {
        Some(wanted) => crate::filesync::paths::claim(
            state.db(),
            family.user_id,
            crate::filesync::paths::Kind::Audiobook,
            book.id,
            &wanted,
        )
        .await
        .map(Some),
        None => crate::filesync::paths::ensure_book(state.db(), book.id).await,
    }
    .map_err(AuthError::Internal)?;
    if let Some(path) = &path {
        crate::filesync::companion::apply_default_cover(state.db(), family.user_id, path)
            .await
            .map_err(AuthError::Internal)?;
    }

    let fresh = db::audiobooks::find_book(state.db(), book.id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok((StatusCode::CREATED, Json(BookWithPath { book: book_to_response(fresh, family.user_id), path })))
}

#[derive(Deserialize)]
pub struct AppendFilesRequest {
    pub files: Vec<FromUploadsFile>,
}

#[derive(Serialize)]
pub struct AppendedFile {
    pub id: String,
    pub relative_path: String,
}

/// POST /api/v1/audiobooks/{id}/files/from-uploads
///
/// Add files uploaded straight to storage to a book the caller owns — the own.audio folder
/// hands a new book's files over in batches (the system creates only so many at once), and
/// a chapter can be added to a book later. Each file keeps its `relative_path` (required);
/// the book's play order is then sorted by path again. A file whose path the book already
/// has is not added twice: a retried upload answers with the existing file.
async fn append_files_from_uploads(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AppendFilesRequest>,
) -> Result<Json<Vec<AppendedFile>>, AuthError> {
    family.require_can_upload()?;
    db::audiobooks::find_book_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    if body.files.is_empty() {
        return Err(AuthError::BadRequest("at least one file is required".to_string()));
    }
    let mut incoming = Vec::with_capacity(body.files.len());
    for file in body.files {
        crate::uploads::require_own_key(&family, &file.object_key)?;
        let raw = file.relative_path.as_deref().ok_or_else(|| {
            AuthError::BadRequest("every file needs a relative_path".to_string())
        })?;
        let relative = crate::filesync::paths::normalize_relative(raw).map_err(AuthError::BadRequest)?;
        incoming.push((relative, file));
    }

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let existing: Vec<(Uuid, Option<String>, Option<String>, i32)> = sqlx::query_as(
        "SELECT id, relative_path, title, position FROM audiobook_files WHERE book_id = $1 ORDER BY position FOR UPDATE",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;
    let mut next = existing.iter().map(|f| f.3).max().unwrap_or(0);
    let mut answer = Vec::with_capacity(incoming.len());
    for (relative, file) in &incoming {
        let key = relative.to_lowercase();
        if let Some(found) = existing.iter().find(|f| f.1.as_deref().map(str::to_lowercase).as_deref() == Some(key.as_str())) {
            answer.push(AppendedFile { id: found.0.to_string(), relative_path: relative.clone() });
            continue;
        }
        let media_id = register_uploaded_object(&state, &mut tx, &file.object_key).await?;
        next += 1;
        let title = file
            .title
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| infer_file_title(relative));
        let file_id: Uuid = sqlx::query_scalar(
            "INSERT INTO audiobook_files (book_id, position, title, duration_secs, audio_object_id, relative_path)
             VALUES ($1, $2, $3, $4, $5, $6) RETURNING id",
        )
        .bind(id)
        .bind(next)
        .bind(title)
        .bind(file.duration_secs)
        .bind(media_id)
        .bind(relative)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
        answer.push(AppendedFile { id: file_id.to_string(), relative_path: relative.clone() });
    }

    // Play order follows the paths again, as it does for a new book. Negative positions
    // first: (book_id, position) is unique.
    let mut all: Vec<(Uuid, String)> = sqlx::query_as::<_, (Uuid, Option<String>, Option<String>)>(
        "SELECT id, relative_path, title FROM audiobook_files WHERE book_id = $1",
    )
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    .into_iter()
    .map(|(file_id, relative, title)| (file_id, sort_path(&relative, title.as_deref().unwrap_or(""))))
    .collect();
    all.sort_by(|a, b| a.1.cmp(&b.1));
    for (index, (file_id, _)) in all.iter().enumerate() {
        sqlx::query("UPDATE audiobook_files SET position = $2 WHERE id = $1")
            .bind(file_id)
            .bind(-(index as i32) - 1)
            .execute(&mut *tx)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
    }
    sqlx::query("UPDATE audiobook_files SET position = -position WHERE book_id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    sqlx::query("UPDATE audiobook_books SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e| AuthError::Internal(e.into()))?;
    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    db::audiobooks::recalculate_total_duration(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(answer))
}

/// Confirm an uploaded object really landed and record it, inside the caller's
/// transaction. Re-heading here (rather than trusting a prior
/// `/uploads/complete`) is what stops a book being created around keys that
/// were never uploaded.
async fn register_uploaded_object(
    state: &AppState,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object_key: &str,
) -> Result<Uuid, AuthError> {
    let (size_bytes, content_type) = state
        .storage()
        .head_object(object_key)
        .await
        .map_err(AuthError::Internal)?
        .ok_or_else(|| {
            AuthError::BadRequest(format!("no object was uploaded under key '{object_key}'"))
        })?;

    db::media::upsert_object(
        &mut **tx,
        state.storage().bucket(),
        object_key,
        &content_type,
        Some(size_bytes),
    )
    .await
    .map_err(AuthError::Internal)
}

/// GET /api/v1/audiobooks/:id
async fn get_book(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<BookResponse>, AuthError> {
    let book = db::audiobooks::find_book(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(book_to_response(book, family.user_id)))
}

async fn update_book(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateBookRequest>,
) -> Result<Json<BookResponse>, AuthError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AuthError::BadRequest("title is required".to_string()));
    }

    db::audiobooks::find_book_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::audiobooks::update_book(
        state.db(),
        id,
        family.user_id,
        title,
        trim_optional(body.author.as_deref()),
        trim_optional(body.narrator.as_deref()),
        trim_optional(body.description.as_deref()),
    )
    .await
    .map_err(AuthError::Internal)?;

    let fresh = db::audiobooks::find_book_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(book_to_response(fresh, family.user_id)))
}

/// DELETE /api/v1/audiobooks/:id — moves the book to the trash (30 days).
/// The owner, or a family admin when it is shared with their family.
async fn delete_book(
    family: FamilyContext,
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    crate::trash::move_to_trash(&state, family.viewer(), Some(&headers), crate::db::trash::Kind::Audiobook, id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/audiobooks/:id/files
async fn list_files(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<FileResponse>>, AuthError> {
    // Verify ownership
    db::audiobooks::find_book(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let files = db::audiobooks::list_files(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(files.into_iter().map(file_to_response).collect()))
}

async fn stream_file(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((book_id, file_id)): Path<(Uuid, Uuid)>,
) -> Result<Json<StreamResponse>, AuthError> {
    db::audiobooks::find_book(state.db(), book_id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let file = db::audiobooks::find_file(state.db(), book_id, file_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let key: String = sqlx::query_scalar(
        "SELECT object_key FROM media_objects WHERE id = $1",
    )
    .bind(file.audio_object_id)
    .fetch_one(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

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

async fn upload_file(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    family.require_can_upload()?;
    db::audiobooks::find_book_owned(state.db(), book_id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let mut meta: Option<UploadFileRequest> = None;
    let mut upload: Option<TempUpload> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "position" => {
                let position = read_text_field(field).await?
                    .parse::<i32>()
                    .map_err(|e| AuthError::BadRequest(format!("invalid position: {e}")))?;
                let current = meta.get_or_insert(UploadFileRequest {
                    position,
                    relative_path: None,
                    duration_secs: None,
                });
                current.position = position;
            }
            "relative_path" => {
                let relative_path = read_text_field(field).await?;
                let current = meta.get_or_insert(UploadFileRequest {
                    position: 0,
                    relative_path: None,
                    duration_secs: None,
                });
                current.relative_path = Some(relative_path);
            }
            "duration_secs" => {
                let raw = read_text_field(field).await?;
                let duration_secs = raw.parse::<i32>()
                    .map_err(|e| AuthError::BadRequest(format!("invalid duration_secs: {e}")))?;
                let current = meta.get_or_insert(UploadFileRequest {
                    position: 0,
                    relative_path: None,
                    duration_secs: None,
                });
                current.duration_secs = Some(duration_secs);
            }
            "file" => upload = Some(save_field_to_temp(field).await?),
            _ => {
                let _ = field;
            }
        }
    }

    let meta = meta.ok_or_else(|| AuthError::BadRequest("missing file metadata".to_string()))?;
    if meta.position <= 0 {
        return Err(AuthError::BadRequest("position must be >= 1".to_string()));
    }
    let upload = upload.ok_or_else(|| AuthError::BadRequest("missing file payload".to_string()))?;

    let relative_name = meta.relative_path.as_deref().unwrap_or(&upload.file_name);
    let object_key = crate::storage::family_key(
        family.family_id,
        format!(
            "audiobooks/{}/files/{:03}-{}",
            book_id,
            meta.position,
            sanitize_filename_component(relative_name)
        ),
    );

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;

    sqlx::query(
        "INSERT INTO audiobook_files (book_id, position, title, duration_secs, audio_object_id)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (book_id, position) DO UPDATE
             SET title = EXCLUDED.title,
                 duration_secs = EXCLUDED.duration_secs,
                 audio_object_id = EXCLUDED.audio_object_id",
    )
    .bind(book_id)
    .bind(meta.position)
    .bind(infer_file_title(relative_name))
    .bind(meta.duration_secs)
    .bind(media_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    db::audiobooks::recalculate_total_duration(state.db(), book_id)
        .await
        .map_err(AuthError::Internal)?;
    let _ = crate::filesync::paths::ensure_book(state.db(), book_id)
        .await
        .inspect_err(|e| tracing::warn!(%book_id, "no path for the new file: {e:#}"));

    Ok(StatusCode::CREATED)
}

async fn upload_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    family.require_can_upload()?;
    db::audiobooks::find_book_owned(state.db(), book_id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let mut upload: Option<TempUpload> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        let name = field.name().unwrap_or_default().to_string();
        if name == "cover" {
            upload = Some(save_field_to_temp(field).await?);
        }
    }

    let upload = upload.ok_or_else(|| AuthError::BadRequest("missing cover payload".to_string()))?;
    let ext = file_extension(&upload.file_name).unwrap_or("jpg");
    let object_key =
        crate::storage::family_key(family.family_id, format!("audiobooks/{book_id}/cover.{ext}"));

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;

    sqlx::query(
        "UPDATE audiobook_books
         SET cover_object_id = $2,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1",
    )
    .bind(book_id)
    .bind(media_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| AuthError::Internal(e.into()))?;

    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;
    Ok(StatusCode::CREATED)
}

// ── Identify (Google Books) ───────────────────────────────────────────────

/// POST /api/v1/audiobooks/:id/metadata/search — Google Books candidates for
/// this book.
///
/// The id scopes visibility, nothing more: the search runs on the title and
/// author in the body, which the client seeds from the book but lets the user
/// edit first. Anyone who can see the book can search — it writes nothing.
async fn search_book_metadata(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<IdentifySearchRequest>,
) -> Result<Json<Vec<crate::metadata::google_books::BookCandidate>>, AuthError> {
    db::audiobooks::find_book(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let title = trim_optional(body.title.as_deref());
    let author = trim_optional(body.author.as_deref());
    if title.is_none() && author.is_none() {
        return Err(AuthError::BadRequest(
            "at least one of title, author is required".to_string(),
        ));
    }

    let candidates = crate::metadata::google_books::search(
        title,
        author,
        body.limit.unwrap_or(10),
        books_api_key(&state),
    )
    .await
    .map_err(identify_error)?;

    Ok(Json(candidates))
}

/// POST /api/v1/audiobooks/:id/metadata/apply — owner-only, mirroring
/// `update_book`'s auth.
///
/// Re-fetches the volume by id rather than trusting the candidate the client is
/// holding, then writes only the fields the client asked for. The cover is
/// last and best-effort: the metadata has already been written by then, and a
/// cover that won't download is not a reason to fail the match.
async fn apply_book_metadata(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ApplyIdentifyRequest>,
) -> Result<Json<BookResponse>, AuthError> {
    // Owner, or a family admin when the book is shared with the family. Tidying the metadata of
    // shared content is administration; an admin could already search for a match here but not
    // keep it, which left the answer visible and unusable.
    let book = db::audiobooks::find_book_manageable(
        state.db(),
        id,
        family.user_id,
        family.family_id,
        family.is_family_admin(),
    )
    .await
    .map_err(AuthError::Internal)?
    .ok_or(AuthError::ItemNotFound)?;

    // Writing the cover means writing to storage, which is the one part of an
    // apply a member without upload rights may not do.
    if body.fields.cover {
        family.require_can_upload()?;
    }

    let volume = crate::metadata::google_books::volume(&body.volume_id, books_api_key(&state))
        .await
        .map_err(identify_error)?;

    let fields = &body.fields;
    db::audiobooks::apply_google_books_metadata(
        state.db(),
        id,
        // The book's owner, not whoever is applying: the write is scoped by owner, and an admin
        // tidying someone else's shared book must not have to own it to do so.
        book.user_id,
        &volume.volume_id,
        fields.title.then_some(volume.title.as_str()),
        fields.author.then_some(volume.author.as_deref()).flatten(),
        fields.description.then_some(volume.description.as_deref()).flatten(),
        fields.publisher.then_some(volume.publisher.as_deref()).flatten(),
        fields.published_year.then_some(volume.published_year).flatten(),
        fields.isbn.then_some(volume.isbn.as_deref()).flatten(),
    )
    .await
    .map_err(AuthError::Internal)?;

    if fields.cover {
        if let Some(cover_url) = volume.cover_url.as_deref() {
            apply_book_cover(&state, &family, id, cover_url).await;
        }
    }

    let fresh = db::audiobooks::find_book_owned(state.db(), id, book.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(book_to_response(fresh, family.user_id)))
}

/// A spent Google quota is the one failure a deployment without an API key hits constantly, and
/// it is fixable — by whoever runs the server, in the Cloud console. Surfacing it as a plain 500
/// hides that behind "something went wrong", so it gets its own message and a `400` the clients
/// already know how to display. Everything else stays an opaque internal error.
fn identify_error(error: anyhow::Error) -> AuthError {
    if error.is::<crate::metadata::google_books::QuotaExhausted>()
        || error
            .source()
            .is_some_and(|source| source.is::<crate::metadata::google_books::QuotaExhausted>())
    {
        return AuthError::BadRequest(
            "Google Books turned this server away — its shared keyless quota is used up. \
             Set GOOGLE_CLOUD__BOOKS_API_KEY on the server to fix it."
                .to_string(),
        );
    }
    AuthError::Internal(error)
}

/// Optional: without it Google bills the request to a per-IP anonymous quota
/// shared with every other keyless caller (see `metadata::google_books`).
fn books_api_key(state: &AppState) -> Option<&str> {
    state
        .config()
        .google_cloud
        .as_ref()
        .and_then(|g| g.books_api_key.as_deref())
        .filter(|key| !key.trim().is_empty())
}

/// Best-effort, exactly like the music side's `apply_cover_art`: every failure
/// here leaves the book with the cover it already had and logs, because the
/// metadata apply this belongs to has already succeeded.
async fn apply_book_cover(
    state: &AppState,
    family: &FamilyContext,
    book_id: Uuid,
    cover_url: &str,
) {
    let Some((bytes, format)) = crate::metadata::google_books::fetch_cover(cover_url).await else {
        tracing::debug!(book_id = %book_id, "google books cover was unusable; leaving the book's cover alone");
        return;
    };

    let ext = format.extension();
    let Ok(temp_file) = NamedTempFile::new() else {
        return;
    };
    if tokio::fs::write(temp_file.path(), &bytes).await.is_err() {
        return;
    }
    let upload = TempUpload {
        file_name: format!("cover.{ext}"),
        temp_file,
        content_type: format.content_type().to_string(),
        size_bytes: bytes.len() as i64,
    };

    // Same key shape as `upload_cover`, so an identified cover replaces a
    // hand-uploaded one instead of leaving two objects behind.
    let object_key =
        crate::storage::family_key(family.family_id, format!("audiobooks/{book_id}/cover.{ext}"));

    let Ok(mut tx) = state.db().begin().await else {
        return;
    };
    let Ok(media_id) = store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await
    else {
        return;
    };
    if sqlx::query(
        "UPDATE audiobook_books
         SET cover_object_id = $2, updated_at = CURRENT_TIMESTAMP
         WHERE id = $1",
    )
    .bind(book_id)
    .bind(media_id)
    .execute(&mut *tx)
    .await
    .is_err()
    {
        return;
    }
    let _ = tx.commit().await;
}

/// GET /api/v1/audiobooks/:id/chapters
async fn list_chapters(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<ChapterResponse>>, AuthError> {
    // Verify ownership
    db::audiobooks::find_book(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let chapters = db::audiobooks::list_chapters(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(chapters.into_iter().map(chapter_to_response).collect()))
}

/// PUT /api/v1/audiobooks/:id/files/:file_id — rename a single file. Only `title` is
/// editable; position/duration/storage are unaffected, matching `update_book`'s own
/// "metadata only" shape.
async fn update_file_title_handler(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((id, file_id)): Path<(Uuid, Uuid)>,
    Json(body): Json<UpdateFileRequest>,
) -> Result<Json<FileResponse>, AuthError> {
    let title = body.title.trim();
    if title.is_empty() {
        return Err(AuthError::BadRequest("title is required".to_string()));
    }

    // Owner, or a family admin when the book is shared — same rule as identifying it (§ above).
    // Renaming a file is metadata tidying, not a bigger power than fixing the book's own title.
    let book = db::audiobooks::find_book_manageable(
        state.db(),
        id,
        family.user_id,
        family.family_id,
        family.is_family_admin(),
    )
    .await
    .map_err(AuthError::Internal)?
    .ok_or(AuthError::ItemNotFound)?;

    db::audiobooks::update_file_title(state.db(), id, book.user_id, file_id, title)
        .await
        .map_err(AuthError::Internal)?;

    let file = db::audiobooks::find_file(state.db(), id, file_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(file_to_response(file)))
}

/// PUT /api/v1/audiobooks/:id/files/reorder — owner, or a family admin when the book is shared
/// (same rule as renaming or deleting a file; used to be owner-only, which would have 403'd a
/// family admin dragging files on a shared book the new web UI now lets them reorder).
async fn reorder_files(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<ReorderFilesRequest>,
) -> Result<Json<Vec<FileResponse>>, AuthError> {
    let book = db::audiobooks::find_book_manageable(
        state.db(),
        id,
        family.user_id,
        family.family_id,
        family.is_family_admin(),
    )
    .await
    .map_err(AuthError::Internal)?
    .ok_or(AuthError::ItemNotFound)?;

    let file_ids: Vec<Uuid> = body
        .file_ids
        .iter()
        .map(|s| s.parse::<Uuid>())
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| AuthError::BadRequest(format!("invalid file id: {e}")))?;

    db::audiobooks::reorder_files(state.db(), id, book.user_id, &file_ids)
        .await
        .map_err(AuthError::Internal)?;

    let files = db::audiobooks::list_files(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(Json(files.into_iter().map(file_to_response).collect()))
}

/// DELETE /api/v1/audiobooks/:id/files/:file_id — removes one file from a multi-file book (a
/// botched or duplicate upload), not the whole book. Owner, or a family admin when the book is
/// shared — same rule as renaming a file.
async fn delete_file_handler(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((id, file_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AuthError> {
    let book = db::audiobooks::find_book_manageable(
        state.db(),
        id,
        family.user_id,
        family.family_id,
        family.is_family_admin(),
    )
    .await
    .map_err(AuthError::Internal)?
    .ok_or(AuthError::ItemNotFound)?;

    let object_id = db::audiobooks::delete_file(state.db(), id, book.user_id, file_id)
        .await
        .map_err(AuthError::Internal)?;

    if let Some(object_id) = object_id {
        crate::trash::delete_orphans(state.db(), state.storage(), &[object_id]).await;
    }

    Ok(StatusCode::NO_CONTENT)
}

async fn get_cover(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AuthError> {
    db::audiobooks::find_book(state.db(), id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let row = sqlx::query_as::<_, (String, String)>(
        "SELECT mo.object_key, mo.content_type
         FROM media_objects mo
         JOIN audiobook_books ab ON ab.cover_object_id = mo.id
         WHERE ab.id = $1",
    )
    .bind(id)
    .fetch_optional(state.db())
    .await
    .map_err(|e| AuthError::Internal(e.into()))?
    .ok_or(AuthError::ItemNotFound)?;

    let bytes = state.storage().get(&row.0).await.map_err(AuthError::Internal)?;

    Ok(([(header::CONTENT_TYPE, row.1)], Body::from(bytes)).into_response())
}

/// PUT /api/v1/audiobooks/:id/visibility — move a book between the private
/// and family folders. Owner only.
async fn set_book_visibility(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<SetVisibilityRequest>,
) -> Result<Json<BookResponse>, AuthError> {
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
        access::AUDIOBOOK,
        id,
        shared,
    )
    .await
    .map_err(AuthError::Internal)?;

    if !updated {
        return Err(AuthError::ItemNotFound);
    }

    let fresh = db::audiobooks::find_book_owned(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(book_to_response(fresh, family.user_id)))
}

// ── Mapping helpers ───────────────────────────────────────────────────────

fn book_to_response(b: models::AudiobookBook, viewer_id: Uuid) -> BookResponse {
    BookResponse {
        id: b.id.to_string(),
        title: b.title,
        author: b.author,
        narrator: b.narrator,
        description: b.description,
        cover_url: b
            .cover_object_id
            .map(|_| format!("/api/v1/audiobooks/{}/cover", b.id)),
        total_duration_secs: b.total_duration_secs,
        source_url: b.source_url,
        google_books_volume_id: b.google_books_volume_id,
        isbn: b.isbn,
        publisher: b.publisher,
        published_year: b.published_year,
        visibility: access::visibility_of(b.family_id).to_string(),
        is_owner: b.user_id == viewer_id,
        owner_id: b.user_id.to_string(),
        created_at: b.created_at.to_rfc3339(),
        updated_at: b.updated_at.to_rfc3339(),
        read_only: b.source.as_deref() == Some("folder"),
        source: b.source.unwrap_or_else(|| "upload".to_string()),
    }
}

fn file_to_response(f: models::AudiobookFile) -> FileResponse {
    FileResponse {
        id: f.id.to_string(),
        book_id: f.book_id.to_string(),
        position: f.position,
        title: f.title,
        duration_secs: f.duration_secs,
        audio_object_id: f.audio_object_id.to_string(),
        size_bytes: f.size_bytes,
        sha256: f.sha256,
    }
}

fn chapter_to_response(c: models::AudiobookChapter) -> ChapterResponse {
    ChapterResponse {
        id: c.id.to_string(),
        book_id: c.book_id.to_string(),
        file_id: c.file_id.map(|u| u.to_string()),
        position: c.position,
        title: c.title,
        start_time_secs: c.start_time_secs,
    }
}

const STREAM_EXPIRY_SECS: u64 = 4 * 3600;

fn trim_optional(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn sort_path(relative_path: &Option<String>, file_name: &str) -> String {
    relative_path
        .as_deref()
        .unwrap_or(file_name)
        .replace('\\', "/")
        .to_lowercase()
}

fn infer_upload_title(uploads: &[(UploadManifestEntry, TempUpload)]) -> String {
    if let Some(common_root) = uploads
        .iter()
        .filter_map(|(meta, _)| meta.relative_path.as_deref())
        .filter_map(|path| path.replace('\\', "/").split('/').next().map(str::to_string))
        .reduce(|acc, value| if acc == value { acc } else { String::new() })
        .filter(|value| !value.is_empty())
    {
        return common_root;
    }

    uploads
        .first()
        .map(|(meta, upload)| infer_file_title(meta.relative_path.as_deref().unwrap_or(&upload.file_name)))
        .unwrap_or_else(|| "Audiobook".to_string())
}

fn infer_file_title(path: &str) -> String {
    StdPath::new(path)
        .file_stem()
        .and_then(|stem| stem.to_str())
        .map(|stem| stem.replace(['_', '-'], " "))
        .map(|stem| stem.trim().to_string())
        .filter(|stem| !stem.is_empty())
        .unwrap_or_else(|| path.to_string())
}

pub(crate) fn sanitize_filename_component(value: &str) -> String {
    let cleaned = value
        .replace(['\\', '/'], "-")
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '_') {
                ch
            } else {
                '-'
            }
        })
        .collect::<String>();

    if cleaned.is_empty() {
        "audio.bin".to_string()
    } else {
        cleaned
    }
}

fn file_extension(file_name: &str) -> Option<&str> {
    StdPath::new(file_name)
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.trim_matches('.'))
        .filter(|ext| !ext.is_empty())
}

async fn save_field_to_temp(mut field: axum::extract::multipart::Field<'_>) -> Result<TempUpload, AuthError> {
    let file_name = field
        .file_name()
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| "upload.bin".to_string());
    // "application/octet-stream" says nothing (the Mac sends it for AVIF); the name decides.
    let content_type = field
        .content_type()
        .filter(|t| *t != "application/octet-stream")
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| infer_content_type(&file_name));

    let temp_file = NamedTempFile::new().map_err(|e| AuthError::Internal(e.into()))?;
    let std_file = temp_file
        .reopen()
        .map_err(|e| AuthError::Internal(e.into()))?;
    let mut writer = tokio::fs::File::from_std(std_file);
    let mut size_bytes = 0_i64;

    while let Some(chunk) = field
        .chunk()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid upload chunk: {e}")))?
    {
        size_bytes += chunk.len() as i64;
        writer
            .write_all(&chunk)
            .await
            .map_err(|e| AuthError::Internal(e.into()))?;
    }

    writer.flush().await.map_err(|e| AuthError::Internal(e.into()))?;

    Ok(TempUpload {
        file_name,
        temp_file,
        content_type,
        size_bytes,
    })
}

fn infer_content_type(file_name: &str) -> String {
    MimeGuess::from_path(file_name)
        .first_or_octet_stream()
        .essence_str()
        .to_string()
}

async fn store_temp_upload(
    storage: &crate::storage::ObjectStore,
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    object_key: &str,
    upload: &TempUpload,
) -> Result<Uuid, AuthError> {
    let bucket = storage.bucket().to_string();
    let path: PathBuf = upload.temp_file.path().to_path_buf();

    storage
        .put_path(object_key, &path, &upload.content_type)
        .await
        .map_err(AuthError::Internal)?;

    db::media::upsert_object(
        &mut **tx,
        &bucket,
        object_key,
        &upload.content_type,
        Some(upload.size_bytes),
    )
    .await
    .map_err(AuthError::Internal)
}

