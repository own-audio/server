// SPDX-License-Identifier: AGPL-3.0-or-later
/// Authors endpoints — CRUD, link/unlink to books, list author's books, tags,
/// and (below, "Author photos") a Wikimedia-fetched picture per author.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::FamilyContext;
use crate::metadata::wikimedia;
use axum::Router;
use axum::body::Body;
use axum::extract::{Json, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct AuthorResponse {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
    pub image_url: Option<String>,
    pub book_count: i64,
    pub created_at: String,
}

#[derive(Deserialize)]
pub struct CreateAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
}

#[derive(Deserialize)]
pub struct LinkAuthorRequest {
    pub author_id: String,
    #[serde(default = "default_author_role")]
    pub role: String,
}

fn default_author_role() -> String {
    "author".to_string()
}

#[derive(Serialize)]
pub struct BookAuthorResponse {
    pub author_id: String,
    pub author_name: String,
    pub role: String,
}

#[derive(Serialize)]
pub struct TagResponse {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize)]
pub struct SetBookTagsRequest {
    pub tags: Vec<String>, // tag names — will get_or_create
}

#[derive(Serialize)]
pub struct AuthorImageInfoResponse {
    /// "Wikimedia Commons", or `None` for a picture set by hand.
    pub source: Option<String>,
    pub author: Option<String>,
    pub license: Option<String>,
    pub license_url: Option<String>,
    pub source_url: Option<String>,
    pub is_user_set: bool,
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        // Authors
        .route("/", get(list_authors))
        .route("/", post(create_author))
        .route("/{id}", get(get_author))
        .route("/{id}", put(update_author))
        .route("/{id}", delete(delete_author))
        .route("/{id}/books", get(list_author_books))
        .route("/{id}/image", get(get_author_image).post(upload_author_image).delete(delete_author_image))
        .route("/{id}/image-info", get(get_author_image_info))
        // Book author links (nested under /audiobooks/{book_id}/authors)
        .route("/book/{book_id}", get(list_book_authors))
        .route("/book/{book_id}", post(link_author_to_book))
        .route("/book/{book_id}/{author_id}/{role}", delete(unlink_author_from_book))
        // Tags
        .route("/tags", get(list_all_tags))
        .route("/tags/book/{book_id}", get(list_book_tags))
        .route("/tags/book/{book_id}", put(set_book_tags))
}

// ── Handlers ────────────────────────────────────────────────────────────────

async fn list_authors(
    _family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<AuthorResponse>>, AuthError> {
    let authors = db::authors::list_authors(state.db())
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::with_capacity(authors.len());
    for a in authors {
        let book_count = db::authors::author_book_count(state.db(), a.id)
            .await
            .map_err(AuthError::Internal)?;
        responses.push(author_to_response(a, book_count));
    }
    Ok(Json(responses))
}

async fn create_author(
    _family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateAuthorRequest>,
) -> Result<(StatusCode, Json<AuthorResponse>), AuthError> {
    let author = db::authors::create_author(
        state.db(),
        &body.name,
        body.sort_name.as_deref(),
        body.bio.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    let resp = author_to_response(author, 0);
    Ok((StatusCode::CREATED, Json(resp)))
}

async fn get_author(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AuthorResponse>, AuthError> {
    let author = db::authors::find_author(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let book_count = db::authors::author_book_count(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    let resp = author_to_response(author, book_count);
    Ok(Json(resp))
}

async fn update_author(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAuthorRequest>,
) -> Result<Json<AuthorResponse>, AuthError> {
    let author = db::authors::update_author(
        state.db(),
        id,
        &body.name,
        body.sort_name.as_deref(),
        body.bio.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;

    let book_count = db::authors::author_book_count(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    let resp = author_to_response(author, book_count);
    Ok(Json(resp))
}

async fn delete_author(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::authors::delete_author(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_author_books(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, AuthError> {
    let book_ids = db::authors::list_author_books(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    let mut books = Vec::new();
    for bid in book_ids {
        if let Some(book) = db::audiobooks::find_book(state.db(), bid, family.viewer()).await.map_err(AuthError::Internal)? {
            books.push(serde_json::json!({
                "id": book.id.to_string(),
                "title": book.title,
                "author": book.author,
                "narrator": book.narrator,
                "total_duration_secs": book.total_duration_secs,
                "created_at": book.created_at.to_rfc3339(),
            }));
        }
    }
    Ok(Json(books))
}

async fn list_book_authors(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<Vec<BookAuthorResponse>>, AuthError> {
    let links = db::authors::list_book_authors(state.db(), book_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::new();
    for link in links {
        if let Some(author) = db::authors::find_author(state.db(), link.author_id).await.map_err(AuthError::Internal)? {
            responses.push(BookAuthorResponse {
                author_id: author.id.to_string(),
                author_name: author.name,
                role: link.role,
            });
        }
    }
    Ok(Json(responses))
}

async fn link_author_to_book(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    Json(body): Json<LinkAuthorRequest>,
) -> Result<StatusCode, AuthError> {
    let author_id: Uuid = body
        .author_id
        .parse()
        .map_err(|_| AuthError::BadRequest("invalid author_id".into()))?;

    db::authors::link_book_author(state.db(), book_id, author_id, &body.role)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

async fn unlink_author_from_book(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path((book_id, author_id, role)): Path<(Uuid, Uuid, String)>,
) -> Result<StatusCode, AuthError> {
    db::authors::unlink_book_author(state.db(), book_id, author_id, &role)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Author photos ────────────────────────────────────────────────────────────
//
// Same shape as music's artist-image endpoints (music/mod.rs), fetched from
// Wikimedia Commons on first request and cached — see metadata/wikimedia.rs's
// module doc for why Commons is the only source examined that licenses for
// this. Keyed by author id instead of a free-text name, since an author here
// already is a row, not borrowed identity the way a music artist is.

/// GET /api/v1/audiobooks/authors/:id/image — the author's photo.
///
/// A miss is cached too (see db::authors::MISS_TTL_DAYS): most libraries hold
/// at least one author no catalogue has heard of, and without a negative
/// entry every render of the "by author" view would re-ask the network.
async fn get_author_image(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AuthError> {
    let author = db::authors::find_author(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    if let Some(cached) = db::authors::find_author_image(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
    {
        // A row with no object is the cached miss — answer it without
        // touching the network again.
        let Some((object_key, content_type)) = cached else {
            return Err(AuthError::ItemNotFound);
        };
        let bytes = state.storage().get(&object_key).await.map_err(AuthError::Internal)?;
        return Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response());
    }

    let mut image = match wikimedia::resolve_author(&author.name).await {
        Ok(image) => image,
        Err(wikimedia::Miss::Missing) => {
            let _ = db::authors::upsert_author_image(state.db(), id, None).await;
            return Err(AuthError::ItemNotFound);
        }
        // Nothing was learned, so nothing is remembered — the next render asks again.
        Err(wikimedia::Miss::Unavailable) => return Err(AuthError::ItemNotFound),
    };

    // The credit is painted into the picture itself, not carried beside it —
    // most Commons licences require attribution wherever the work appears,
    // and a client rendering just an <img> would otherwise drop it silently.
    let Some(credit) = crate::metadata::watermark::credit_line(
        image.attribution.author.as_deref(),
        image.attribution.license.as_deref(),
    ) else {
        let _ = db::authors::upsert_author_image(state.db(), id, None).await;
        return Err(AuthError::ItemNotFound);
    };

    match crate::metadata::watermark::burn(&image.bytes, &credit) {
        Ok(bytes) => {
            image.bytes = bytes;
            image.extension = "jpg";
            image.content_type = "image/jpeg";
        }
        Err(error) => {
            tracing::warn!(author = %author.name, %error, "could not watermark author image; dropping it");
            let _ = db::authors::upsert_author_image(state.db(), id, None).await;
            return Err(AuthError::ItemNotFound);
        }
    }

    let content_type = image.content_type.to_string();
    let bytes = image.bytes.clone();
    store_author_image(&state, id, image).await;
    Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response())
}

/// Best-effort, like every other cover write in this codebase: the caller
/// already has the bytes to answer with, so a storage failure just costs a
/// re-fetch next time rather than the request.
async fn store_author_image(state: &AppState, author_id: Uuid, image: wikimedia::ArtistImage) {
    let Ok(temp_file) = tempfile::NamedTempFile::new() else { return };
    if tokio::fs::write(temp_file.path(), &image.bytes).await.is_err() {
        return;
    }
    let upload = super::TempUpload {
        file_name: format!("author.{}", image.extension),
        temp_file,
        content_type: image.content_type.to_string(),
        size_bytes: image.bytes.len() as i64,
    };
    // Global, not family-scoped — an author is one row shared by the whole
    // instance, so its picture is not billed to any one family's storage.
    let key = format!("audiobooks/authors/{author_id}.{}", image.extension);

    let Ok(mut tx) = state.db().begin().await else { return };
    let Ok(media_id) = super::store_temp_upload(state.storage(), &mut tx, &key, &upload).await else {
        return;
    };
    if tx.commit().await.is_err() {
        return;
    }
    let _ = db::authors::upsert_fetched_author_image(state.db(), author_id, media_id, &image.attribution).await;
}

/// GET /api/v1/audiobooks/authors/:id/image-info — who to credit for the
/// picture, and under what licence. Separate from the image itself for the
/// same reason as music's: an `<img>` never sees response headers, and most
/// Commons licences require this to be shown, so it is not decoration.
async fn get_author_image_info(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AuthorImageInfoResponse>, AuthError> {
    let row = db::authors::find_author_image_attribution(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    Ok(Json(AuthorImageInfoResponse {
        source: row.0,
        author: row.1,
        license: row.2,
        license_url: row.3,
        source_url: row.4,
        is_user_set: row.5,
    }))
}

/// POST /api/v1/audiobooks/authors/:id/image — the user's own picture for an
/// author, mirroring `upload_cover` for a book. Marked `is_user_set`, which is
/// what makes it permanent: the automatic lookup skips any author carrying
/// that flag, so a deliberate choice is never quietly replaced later.
async fn upload_author_image(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    db::authors::find_author(state.db(), id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let mut upload: Option<super::TempUpload> = None;
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AuthError::BadRequest(format!("invalid multipart payload: {e}")))?
    {
        if field.name().unwrap_or_default() == "image" {
            upload = Some(super::save_field_to_temp(field).await?);
        }
    }
    let upload = upload.ok_or_else(|| AuthError::BadRequest("missing image payload".to_string()))?;

    let ext = super::file_extension(&upload.file_name).unwrap_or("jpg");
    let object_key = format!("audiobooks/authors/{id}-user.{ext}");

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = super::store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;
    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;

    db::authors::set_user_author_image(state.db(), id, media_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::CREATED)
}

/// DELETE /api/v1/audiobooks/authors/:id/image — forget this author's
/// picture, user-set or fetched, and let the automatic lookup run again.
async fn delete_author_image(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::authors::clear_author_image(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Tags ────────────────────────────────────────────────────────────────────

async fn list_all_tags(
    _family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<TagResponse>>, AuthError> {
    let tags = db::authors::list_tags(state.db())
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(
        tags.into_iter()
            .map(|t| TagResponse {
                id: t.id.to_string(),
                name: t.name,
            })
            .collect(),
    ))
}

async fn list_book_tags(
    _family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<Vec<TagResponse>>, AuthError> {
    let tags = db::authors::list_book_tags(state.db(), book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(
        tags.into_iter()
            .map(|t| TagResponse {
                id: t.id.to_string(),
                name: t.name,
            })
            .collect(),
    ))
}

async fn set_book_tags(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    Json(body): Json<SetBookTagsRequest>,
) -> Result<Json<Vec<TagResponse>>, AuthError> {
    // Get or create each tag
    let mut tag_ids = Vec::new();
    for name in &body.tags {
        let tag = db::authors::get_or_create_tag(state.db(), name)
            .await
            .map_err(AuthError::Internal)?;
        tag_ids.push(tag.id);
    }

    db::authors::set_book_tags(state.db(), book_id, &tag_ids)
        .await
        .map_err(AuthError::Internal)?;

    // Return updated tags
    list_book_tags(family, State(state), Path(book_id)).await
}

// ── Helpers ─────────────────────────────────────────────────────────────────

/// `image_url` always points at this module's own lazy-fetch-and-cache route
/// (`get_author_image`) rather than a presigned S3 URL — a client can point an
/// `<img>` at it whether or not a picture has been found yet: a cache hit
/// serves it, a first request triggers the Wikimedia lookup, and neither
/// found nor still-loading has to be told apart here.
fn author_to_response(a: crate::audiobooks::models::Author, book_count: i64) -> AuthorResponse {
    AuthorResponse {
        image_url: Some(format!("/api/v1/audiobooks/authors/{}/image", a.id)),
        id: a.id.to_string(),
        name: a.name,
        sort_name: a.sort_name,
        bio: a.bio,
        book_count,
        created_at: a.created_at.to_rfc3339(),
    }
}
