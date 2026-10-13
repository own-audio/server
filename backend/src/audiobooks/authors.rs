// SPDX-License-Identifier: AGPL-3.0-or-later
/// Authors endpoints — CRUD, link/unlink to books, list author's books, tags,
/// and (below, "Author photos") a Wikimedia-fetched picture per author.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::FamilyContext;
use crate::metadata::wikimedia;
use axum::body::Body;
use axum::extract::{Json, Multipart, Path, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct AuthorResponse {
    pub id: String,
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
    pub image_url: Option<String>,
    pub book_count: i64,
    pub created_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateAuthorRequest {
    pub name: String,
    pub sort_name: Option<String>,
    pub bio: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct LinkAuthorRequest {
    pub author_id: String,
    #[serde(default = "default_author_role")]
    pub role: String,
}

/// The roles a book credits (DESIGN §7.2).
const BOOK_ROLES: &[&str] = &["author", "co_author", "narrator", "translator", "editor", "illustrator"];

pub(crate) const NAME_TAKEN: &str = "another contributor already has this name; merge the two instead";

fn default_author_role() -> String {
    "author".to_string()
}

#[derive(Serialize, ToSchema)]
pub struct BookAuthorResponse {
    pub author_id: String,
    pub author_name: String,
    pub role: String,
}

#[derive(Serialize, ToSchema)]
pub struct TagResponse {
    pub id: String,
    pub name: String,
}

#[derive(Deserialize, ToSchema)]
pub struct SetBookTagsRequest {
    pub tags: Vec<String>, // tag names — will get_or_create
}

#[derive(Serialize, ToSchema)]
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

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Authors
        .routes(routes!(list_authors, create_author))
        .routes(routes!(get_author, update_author, delete_author))
        .routes(routes!(list_author_books))
        .routes(routes!(get_author_image, upload_author_image, delete_author_image))
        .routes(routes!(get_author_image_info))
        // Book author links (nested under /audiobooks/{book_id}/authors)
        .routes(routes!(list_book_authors, link_author_to_book))
        .routes(routes!(unlink_author_from_book))
        // Tags
        .routes(routes!(list_all_tags))
        .routes(routes!(list_book_tags, set_book_tags))
}

// ── Handlers ────────────────────────────────────────────────────────────────

/// Every author on the server.
#[utoipa::path(get, path = "/", tag = "authors", security(("bearer" = [])),
    responses(
        (status = 200, body = Vec<AuthorResponse>)))]
async fn list_authors(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<AuthorResponse>>, AuthError> {
    let authors = db::authors::list_authors(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(authors.into_iter().map(|(a, n)| author_to_response(a, n)).collect()))
}

/// Create an author.
#[utoipa::path(post, path = "/", tag = "authors", security(("bearer" = [])),
    request_body = CreateAuthorRequest,
    responses(
        (status = 201, body = AuthorResponse)))]
async fn create_author(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateAuthorRequest>,
) -> Result<(StatusCode, Json<AuthorResponse>), AuthError> {
    if body.name.trim().is_empty() {
        return Err(AuthError::BadRequest("name is required".into()));
    }
    let author = db::authors::create_author(
        state.db(),
        family.family_id,
        &body.name,
        body.sort_name.as_deref(),
        body.bio.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;
    let book_count = db::authors::author_book_count(state.db(), family.viewer(), author.id)
        .await
        .map_err(AuthError::Internal)?;
    Ok((StatusCode::CREATED, Json(author_to_response(author, book_count))))
}

/// One author.
#[utoipa::path(get, path = "/{id}", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    responses(
        (status = 200, body = AuthorResponse),
        (status = 404, description = "No such author", body = crate::http::openapi::ErrorBody)))]
async fn get_author(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AuthorResponse>, AuthError> {
    let author = household_author(&state, &family, id).await?;
    let book_count = db::authors::author_book_count(state.db(), family.viewer(), id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(author_to_response(author, book_count)))
}

/// Edit an author.
#[utoipa::path(put, path = "/{id}", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    request_body = UpdateAuthorRequest,
    responses(
        (status = 200, body = AuthorResponse),
        (status = 409, description = "Another contributor already has this name: merge them instead", body = crate::http::openapi::ErrorBody)))]
async fn update_author(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateAuthorRequest>,
) -> Result<Json<AuthorResponse>, AuthError> {
    if body.name.trim().is_empty() {
        return Err(AuthError::BadRequest("name is required".into()));
    }
    let author = match db::authors::update_author(
        state.db(),
        family.family_id,
        id,
        &body.name,
        body.sort_name.as_deref(),
        body.bio.as_deref(),
        &crate::db::edits::Edit::new(family.user_id, "manual"),
    )
    .await
    .map_err(AuthError::Internal)?
    {
        Ok(author) => author,
        Err(db::entities::Renamed::Taken(_)) => return Err(AuthError::Conflict(NAME_TAKEN.into())),
        Err(_) => return Err(AuthError::ItemNotFound),
    };
    let book_count = db::authors::author_book_count(state.db(), family.viewer(), id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(author_to_response(author, book_count)))
}

/// Delete an author.
#[utoipa::path(delete, path = "/{id}", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    responses(
        (status = 204, description = "Deleted")))]
async fn delete_author(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    // A contributor is the household's, credited on everyone's books and
    // music: removing it is a family admin's call.
    family.require_family_admin()?;
    if !db::authors::delete_author(state.db(), family.family_id, id)
        .await
        .map_err(AuthError::Internal)?
    {
        return Err(AuthError::ItemNotFound);
    }
    Ok(StatusCode::NO_CONTENT)
}

/// The author's books that the caller can see.
#[utoipa::path(get, path = "/{id}/books", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    responses(
        (status = 200, description = "Books: `id`, `title`, `author`, `narrator`, `total_duration_secs`, `created_at`", body = Vec<Object>)))]
async fn list_author_books(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, AuthError> {
    household_author(&state, &family, id).await?;
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

/// The authors linked to a book, with their roles.
#[utoipa::path(get, path = "/book/{book_id}", tag = "authors", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 200, body = Vec<BookAuthorResponse>)))]
async fn list_book_authors(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<Vec<BookAuthorResponse>>, AuthError> {
    visible_book(&state, &family, book_id).await?;
    let links = db::authors::list_book_authors(state.db(), book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(
        links
            .into_iter()
            .map(|(link, name)| BookAuthorResponse { author_id: link.author_id.to_string(), author_name: name, role: link.role })
            .collect(),
    ))
}

/// Link an author to a book in a role.
#[utoipa::path(post, path = "/book/{book_id}", tag = "authors", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    request_body = LinkAuthorRequest,
    responses(
        (status = 204, description = "Linked"),
        (status = 400, description = "`author_id` is not a UUID", body = crate::http::openapi::ErrorBody)))]
async fn link_author_to_book(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    Json(body): Json<LinkAuthorRequest>,
) -> Result<StatusCode, AuthError> {
    let author_id: Uuid = body
        .author_id
        .parse()
        .map_err(|_| AuthError::BadRequest("invalid author_id".into()))?;
    if !BOOK_ROLES.contains(&body.role.as_str()) {
        return Err(AuthError::BadRequest(format!("role must be one of {}", BOOK_ROLES.join(", "))));
    }
    owned_book(&state, &family, book_id).await?;
    household_author(&state, &family, author_id).await?;
    db::authors::link_book_author(state.db(), book_id, author_id, &body.role)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Remove an author's role on a book.
#[utoipa::path(delete, path = "/book/{book_id}/{author_id}/{role}", tag = "authors", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id"), ("author_id" = Uuid, Path, description = "Author id"), ("role" = String, Path, description = "The role to remove, e.g. `author`")),
    responses(
        (status = 204, description = "Unlinked")))]
async fn unlink_author_from_book(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((book_id, author_id, role)): Path<(Uuid, Uuid, String)>,
) -> Result<StatusCode, AuthError> {
    owned_book(&state, &family, book_id).await?;
    db::authors::unlink_book_author(state.db(), book_id, author_id, &role)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The book, if the viewer can see it; `ItemNotFound` otherwise.
async fn visible_book(state: &AppState, family: &FamilyContext, book_id: Uuid) -> Result<(), AuthError> {
    db::audiobooks::find_book(state.db(), book_id, family.viewer())
        .await
        .map_err(AuthError::Internal)?
        .map(|_| ())
        .ok_or(AuthError::ItemNotFound)
}

/// The book, if the viewer owns it: changing who is credited on a book is
/// editing the book, which only its owner does (like `PUT /audiobooks/{id}`).
async fn owned_book(state: &AppState, family: &FamilyContext, book_id: Uuid) -> Result<(), AuthError> {
    db::audiobooks::find_book_owned(state.db(), book_id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .map(|_| ())
        .ok_or(AuthError::ItemNotFound)
}

/// A contributor of the viewer's household.
async fn household_author(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<crate::audiobooks::models::Author, AuthError> {
    db::authors::find_author(state.db(), family.family_id, id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)
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
#[utoipa::path(get, path = "/{id}/image", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    responses(
        (status = 200, description = "The photo, with its credit burned in", content_type = "image/*"),
        (status = 404, description = "No such author, or no usable photo", body = crate::http::openapi::ErrorBody)))]
async fn get_author_image(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Response, AuthError> {
    let author = household_author(&state, &family, id).await?;

    if let Some(cached) = db::music::find_artist_image(state.db(), id)
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
            let _ = db::music::upsert_artist_image(state.db(), id, None).await;
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
        let _ = db::music::upsert_artist_image(state.db(), id, None).await;
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
            let _ = db::music::upsert_artist_image(state.db(), id, None).await;
            return Err(AuthError::ItemNotFound);
        }
    }

    let content_type = image.content_type.to_string();
    let bytes = image.bytes.clone();
    store_author_image(&state, family.family_id, id, image).await;
    Ok(([(header::CONTENT_TYPE, content_type)], Body::from(bytes)).into_response())
}

/// Best-effort, like every other cover write in this codebase: the caller
/// already has the bytes to answer with, so a storage failure just costs a
/// re-fetch next time rather than the request.
async fn store_author_image(state: &AppState, family_id: Uuid, author_id: Uuid, image: wikimedia::ArtistImage) {
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
    // The household's contributor, so the household's storage — the same
    // photo the music side shows for a contributor of that name.
    let key = crate::storage::family_key(family_id, format!("music/artists/{author_id}.{}", image.extension));
    let Ok(mut tx) = state.db().begin().await else { return };
    let Ok(media_id) = super::store_temp_upload(state.storage(), &mut tx, &key, &upload).await else {
        return;
    };
    if db::music::upsert_fetched_artist_image(&mut *tx, author_id, Some(media_id), &image.attribution).await.is_err() {
        return;
    }
    let _ = tx.commit().await;
}

/// GET /api/v1/audiobooks/authors/:id/image-info — who to credit for the
/// picture, and under what licence. Separate from the image itself for the
/// same reason as music's: an `<img>` never sees response headers, and most
/// Commons licences require this to be shown, so it is not decoration.
#[utoipa::path(get, path = "/{id}/image-info", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    responses(
        (status = 200, body = AuthorImageInfoResponse),
        (status = 404, description = "No photo recorded for this author", body = crate::http::openapi::ErrorBody)))]
async fn get_author_image_info(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<AuthorImageInfoResponse>, AuthError> {
    household_author(&state, &family, id).await?;
    let row = db::music::find_artist_image_attribution(state.db(), id)
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
#[utoipa::path(post, path = "/{id}/image", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    request_body(content_type = "multipart/form-data", description = "fields: `image` (image)"),
    responses(
        (status = 201, description = "Stored"),
        (status = 400, description = "No `image` field, or a malformed upload", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "No such author", body = crate::http::openapi::ErrorBody)))]
async fn upload_author_image(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    mut multipart: Multipart,
) -> Result<StatusCode, AuthError> {
    household_author(&state, &family, id).await?;

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
    let object_key = crate::storage::family_key(family.family_id, format!("music/artists/{id}-user.{ext}"));

    let mut tx = state.db().begin().await.map_err(|e| AuthError::Internal(e.into()))?;
    let media_id = super::store_temp_upload(state.storage(), &mut tx, &object_key, &upload).await?;
    tx.commit().await.map_err(|e| AuthError::Internal(e.into()))?;

    db::music::set_user_artist_image(state.db(), id, media_id)
        .await
        .map_err(AuthError::Internal)?;

    Ok(StatusCode::CREATED)
}

/// DELETE /api/v1/audiobooks/authors/:id/image — forget this author's
/// picture, user-set or fetched, and let the automatic lookup run again.
#[utoipa::path(delete, path = "/{id}/image", tag = "authors", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Author id")),
    responses(
        (status = 204, description = "Forgotten")))]
async fn delete_author_image(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    household_author(&state, &family, id).await?;
    db::music::clear_artist_image(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Tags ────────────────────────────────────────────────────────────────────

/// Every tag on the server.
#[utoipa::path(get, path = "/tags", tag = "authors", security(("bearer" = [])),
    responses(
        (status = 200, body = Vec<TagResponse>)))]
async fn list_all_tags(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<TagResponse>>, AuthError> {
    let tags = db::authors::list_tags(state.db(), family.viewer())
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(tags.into_iter().map(|t| TagResponse { id: t.id.to_string(), name: t.name }).collect()))
}

/// A book's tags.
#[utoipa::path(get, path = "/tags/book/{book_id}", tag = "authors", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 200, body = Vec<TagResponse>)))]
async fn list_book_tags(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<Vec<TagResponse>>, AuthError> {
    visible_book(&state, &family, book_id).await?;
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

/// Replace a book's tags, creating any that do not exist yet.
#[utoipa::path(put, path = "/tags/book/{book_id}", tag = "authors", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    request_body = SetBookTagsRequest,
    responses(
        (status = 200, description = "The book's tags now", body = Vec<TagResponse>)))]
async fn set_book_tags(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
    Json(body): Json<SetBookTagsRequest>,
) -> Result<Json<Vec<TagResponse>>, AuthError> {
    owned_book(&state, &family, book_id).await?;
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
