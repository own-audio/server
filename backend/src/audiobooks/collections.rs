// SPDX-License-Identifier: AGPL-3.0-or-later
/// Collections & series endpoints — CRUD, add/remove books, favorites.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::FamilyContext;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::{router::OpenApiRouter, routes};
use uuid::Uuid;

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct CollectionResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub cover_url: Option<String>,
    pub is_public: bool,
    pub book_count: i64,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateCollectionRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub is_public: bool,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateCollectionRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub is_public: bool,
}

#[derive(Serialize, ToSchema)]
pub struct SeriesResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub books: Vec<SeriesBookEntry>,
    pub created_at: String,
}

#[derive(Serialize, ToSchema)]
pub struct SeriesBookEntry {
    pub book_id: String,
    pub book_title: String,
    pub position: f64,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateSeriesRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateSeriesRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct AddBookToSeriesRequest {
    pub book_id: String,
    pub position: f64,
}

#[derive(Deserialize, ToSchema)]
pub struct AddBookRequest {
    pub book_id: String,
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // Collections
        .routes(routes!(list_collections, create_collection))
        .routes(routes!(get_collection, update_collection, delete_collection))
        .routes(routes!(list_collection_books, add_book_to_collection))
        .routes(routes!(remove_book_from_collection))
        // Series
        .routes(routes!(list_series, create_series))
        .routes(routes!(get_series, update_series, delete_series))
        .routes(routes!(add_book_to_series))
        .routes(routes!(remove_book_from_series))
        // Favorites
        .routes(routes!(list_favorites))
        .routes(routes!(add_favorite, remove_favorite))
        .routes(routes!(check_favorite))
}

// ── Collection handlers ─────────────────────────────────────────────────────

/// The caller's collections.
#[utoipa::path(get, path = "/collections", tag = "collections", security(("bearer" = [])),
    responses(
        (status = 200, body = Vec<CollectionResponse>)))]
async fn list_collections(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<CollectionResponse>>, AuthError> {
    let collections = db::collections::list_collections(state.db(), family.user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::with_capacity(collections.len());
    for c in collections {
        let count = db::collections::collection_book_count(state.db(), c.id)
            .await
            .map_err(AuthError::Internal)?;
        responses.push(collection_to_response(c, count, &state).await?);
    }
    Ok(Json(responses))
}

/// Create a collection.
#[utoipa::path(post, path = "/collections", tag = "collections", security(("bearer" = [])),
    request_body = CreateCollectionRequest,
    responses(
        (status = 201, body = CollectionResponse)))]
async fn create_collection(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateCollectionRequest>,
) -> Result<(StatusCode, Json<CollectionResponse>), AuthError> {
    let c = db::collections::create_collection(
        state.db(),
        family.user_id,
        &body.name,
        body.description.as_deref(),
        body.is_public,
    )
    .await
    .map_err(AuthError::Internal)?;
    let resp = collection_to_response(c, 0, &state).await?;
    Ok((StatusCode::CREATED, Json(resp)))
}

/// One of the caller's collections.
#[utoipa::path(get, path = "/collections/{id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Collection id")),
    responses(
        (status = 200, body = CollectionResponse),
        (status = 404, description = "No such collection of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn get_collection(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<CollectionResponse>, AuthError> {
    let c = db::collections::find_collection(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let count = db::collections::collection_book_count(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    let resp = collection_to_response(c, count, &state).await?;
    Ok(Json(resp))
}

/// Edit a collection's name, description and visibility.
#[utoipa::path(put, path = "/collections/{id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Collection id")),
    request_body = UpdateCollectionRequest,
    responses(
        (status = 200, body = CollectionResponse)))]
async fn update_collection(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateCollectionRequest>,
) -> Result<Json<CollectionResponse>, AuthError> {
    let c = db::collections::update_collection(
        state.db(),
        id,
        family.user_id,
        &body.name,
        body.description.as_deref(),
        body.is_public,
    )
    .await
    .map_err(AuthError::Internal)?;
    let count = db::collections::collection_book_count(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;
    let resp = collection_to_response(c, count, &state).await?;
    Ok(Json(resp))
}

/// Delete a collection; its books are untouched.
#[utoipa::path(delete, path = "/collections/{id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Collection id")),
    responses(
        (status = 204, description = "Deleted")))]
async fn delete_collection(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::collections::delete_collection(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// The books in a collection that the caller can see.
#[utoipa::path(get, path = "/collections/{id}/books", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Collection id")),
    responses(
        (status = 200, description = "Books: `id`, `title`, `author`, `narrator`, `cover_url` (presigned, or null), `total_duration_secs`, `created_at`", body = Vec<Object>),
        (status = 404, description = "No such collection of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn list_collection_books(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<Vec<serde_json::Value>>, AuthError> {
    // Verify ownership
    db::collections::find_collection(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let book_ids = db::collections::list_collection_book_ids(state.db(), id)
        .await
        .map_err(AuthError::Internal)?;

    let mut books = Vec::new();
    for bid in book_ids {
        if let Some(book) = db::audiobooks::find_book(state.db(), bid, family.viewer())
            .await
            .map_err(AuthError::Internal)?
        {
            let cover_url = resolve_cover_url(&state, book.cover_object_id).await?;
            books.push(serde_json::json!({
                "id": book.id.to_string(),
                "title": book.title,
                "author": book.author,
                "narrator": book.narrator,
                "cover_url": cover_url,
                "total_duration_secs": book.total_duration_secs,
                "created_at": book.created_at.to_rfc3339(),
            }));
        }
    }
    Ok(Json(books))
}

/// Add a book to a collection.
#[utoipa::path(post, path = "/collections/{id}/books", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Collection id")),
    request_body = AddBookRequest,
    responses(
        (status = 204, description = "Added"),
        (status = 400, description = "`book_id` is not a UUID", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "No such collection of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn add_book_to_collection(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AddBookRequest>,
) -> Result<StatusCode, AuthError> {
    // Verify collection ownership
    db::collections::find_collection(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let book_id: Uuid = body
        .book_id
        .parse()
        .map_err(|_| AuthError::BadRequest("invalid book_id".into()))?;

    db::collections::add_book_to_collection(state.db(), id, book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a book out of a collection.
#[utoipa::path(delete, path = "/collections/{id}/books/{book_id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Collection id"), ("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such collection of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn remove_book_from_collection(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((id, book_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AuthError> {
    db::collections::find_collection(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::collections::remove_book_from_collection(state.db(), id, book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Series handlers ─────────────────────────────────────────────────────────

/// The caller's series, each with its books in order.
#[utoipa::path(get, path = "/series", tag = "collections", security(("bearer" = [])),
    responses(
        (status = 200, body = Vec<SeriesResponse>)))]
async fn list_series(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<SeriesResponse>>, AuthError> {
    let series_list = db::collections::list_series(state.db(), family.user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut responses = Vec::new();
    for s in series_list {
        let resp = series_to_response(&state, s, family.viewer()).await?;
        responses.push(resp);
    }
    Ok(Json(responses))
}

/// Create a series.
#[utoipa::path(post, path = "/series", tag = "collections", security(("bearer" = [])),
    request_body = CreateSeriesRequest,
    responses(
        (status = 201, body = SeriesResponse)))]
async fn create_series(
    family: FamilyContext,
    State(state): State<AppState>,
    Json(body): Json<CreateSeriesRequest>,
) -> Result<(StatusCode, Json<SeriesResponse>), AuthError> {
    let s = db::collections::create_series(
        state.db(),
        family.user_id,
        &body.name,
        body.description.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;
    let resp = series_to_response(&state, s, family.viewer()).await?;
    Ok((StatusCode::CREATED, Json(resp)))
}

/// One of the caller's series.
#[utoipa::path(get, path = "/series/{id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Series id")),
    responses(
        (status = 200, body = SeriesResponse),
        (status = 404, description = "No such series of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn get_series(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<Json<SeriesResponse>, AuthError> {
    let s = db::collections::find_series(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;
    let resp = series_to_response(&state, s, family.viewer()).await?;
    Ok(Json(resp))
}

/// Edit a series' name and description.
#[utoipa::path(put, path = "/series/{id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Series id")),
    request_body = UpdateSeriesRequest,
    responses(
        (status = 200, body = SeriesResponse)))]
async fn update_series(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<UpdateSeriesRequest>,
) -> Result<Json<SeriesResponse>, AuthError> {
    let s = db::collections::update_series(
        state.db(),
        id,
        family.user_id,
        &body.name,
        body.description.as_deref(),
    )
    .await
    .map_err(AuthError::Internal)?;
    let resp = series_to_response(&state, s, family.viewer()).await?;
    Ok(Json(resp))
}

/// Delete a series; its books are untouched.
#[utoipa::path(delete, path = "/series/{id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Series id")),
    responses(
        (status = 204, description = "Deleted")))]
async fn delete_series(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::collections::delete_series(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Add a book to a series at a position.
#[utoipa::path(post, path = "/series/{id}/books", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Series id")),
    request_body = AddBookToSeriesRequest,
    responses(
        (status = 204, description = "Added"),
        (status = 400, description = "`book_id` is not a UUID", body = crate::http::openapi::ErrorBody),
        (status = 404, description = "No such series of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn add_book_to_series(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Json(body): Json<AddBookToSeriesRequest>,
) -> Result<StatusCode, AuthError> {
    db::collections::find_series(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    let book_id: Uuid = body
        .book_id
        .parse()
        .map_err(|_| AuthError::BadRequest("invalid book_id".into()))?;

    db::collections::add_book_to_series(state.db(), id, book_id, body.position)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Take a book out of a series.
#[utoipa::path(delete, path = "/series/{id}/books/{book_id}", tag = "collections", security(("bearer" = [])),
    params(("id" = Uuid, Path, description = "Series id"), ("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 204, description = "Removed"),
        (status = 404, description = "No such series of the caller's", body = crate::http::openapi::ErrorBody)))]
async fn remove_book_from_series(
    family: FamilyContext,
    State(state): State<AppState>,
    Path((id, book_id)): Path<(Uuid, Uuid)>,
) -> Result<StatusCode, AuthError> {
    db::collections::find_series(state.db(), id, family.user_id)
        .await
        .map_err(AuthError::Internal)?
        .ok_or(AuthError::ItemNotFound)?;

    db::collections::remove_book_from_series(state.db(), id, book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

// ── Favorites handlers ──────────────────────────────────────────────────────

/// The caller's favourite books that they can still see.
#[utoipa::path(get, path = "/favorites", tag = "collections", security(("bearer" = [])),
    responses(
        (status = 200, description = "Books: `id`, `title`, `author`, `cover_url` (presigned, or null), `total_duration_secs`, `favorited_at`", body = Vec<Object>)))]
async fn list_favorites(
    family: FamilyContext,
    State(state): State<AppState>,
) -> Result<Json<Vec<serde_json::Value>>, AuthError> {
    let favs = db::collections::list_favorites(state.db(), family.user_id)
        .await
        .map_err(AuthError::Internal)?;

    let mut books = Vec::new();
    for fav in favs {
        if let Some(book) = db::audiobooks::find_book(state.db(), fav.book_id, family.viewer())
            .await
            .map_err(AuthError::Internal)?
        {
            let cover_url = resolve_cover_url(&state, book.cover_object_id).await?;
            books.push(serde_json::json!({
                "id": book.id.to_string(),
                "title": book.title,
                "author": book.author,
                "cover_url": cover_url,
                "total_duration_secs": book.total_duration_secs,
                "favorited_at": fav.created_at.to_rfc3339(),
            }));
        }
    }
    Ok(Json(books))
}

/// Mark a book as a favourite.
#[utoipa::path(post, path = "/favorites/{book_id}", tag = "collections", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 204, description = "Added")))]
async fn add_favorite(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::collections::add_favorite(state.db(), family.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Unmark a favourite book.
#[utoipa::path(delete, path = "/favorites/{book_id}", tag = "collections", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 204, description = "Removed")))]
async fn remove_favorite(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<StatusCode, AuthError> {
    db::collections::remove_favorite(state.db(), family.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(StatusCode::NO_CONTENT)
}

/// Whether a book is one of the caller's favourites.
#[utoipa::path(get, path = "/favorites/{book_id}/check", tag = "collections", security(("bearer" = [])),
    params(("book_id" = Uuid, Path, description = "Book id")),
    responses(
        (status = 200, description = "`is_favorite` (boolean)", body = Object)))]
async fn check_favorite(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(book_id): Path<Uuid>,
) -> Result<Json<serde_json::Value>, AuthError> {
    let is_fav = db::collections::is_favorite(state.db(), family.user_id, book_id)
        .await
        .map_err(AuthError::Internal)?;
    Ok(Json(serde_json::json!({ "is_favorite": is_fav })))
}

// ── Helpers ─────────────────────────────────────────────────────────────────

async fn collection_to_response(
    c: crate::audiobooks::models::Collection,
    book_count: i64,
    state: &AppState,
) -> Result<CollectionResponse, AuthError> {
    let cover_url = resolve_cover_url(state, c.cover_object_id).await?;
    Ok(CollectionResponse {
        id: c.id.to_string(),
        name: c.name,
        description: c.description,
        cover_url,
        is_public: c.is_public,
        book_count,
        created_at: c.created_at.to_rfc3339(),
        updated_at: c.updated_at.to_rfc3339(),
    })
}

async fn series_to_response(
    state: &AppState,
    s: crate::audiobooks::models::Series,
    viewer: crate::db::access::Viewer,
) -> Result<SeriesResponse, AuthError> {
    let series_books = db::collections::list_series_books(state.db(), s.id)
        .await
        .map_err(AuthError::Internal)?;

    let mut books = Vec::new();
    for sb in series_books {
        let title = if let Some(book) = db::audiobooks::find_book(state.db(), sb.book_id, viewer)
            .await
            .map_err(AuthError::Internal)?
        {
            book.title
        } else {
            "Unknown".to_string()
        };
        books.push(SeriesBookEntry {
            book_id: sb.book_id.to_string(),
            book_title: title,
            position: sb.position,
        });
    }

    Ok(SeriesResponse {
        id: s.id.to_string(),
        name: s.name,
        description: s.description,
        books,
        created_at: s.created_at.to_rfc3339(),
    })
}

async fn resolve_cover_url(
    state: &AppState,
    cover_object_id: Option<Uuid>,
) -> Result<Option<String>, AuthError> {
    if let Some(oid) = cover_object_id {
        let key: Option<String> =
            sqlx::query_scalar("SELECT object_key FROM media_objects WHERE id = $1")
                .bind(oid)
                .fetch_optional(state.db())
                .await
                .map_err(|e| AuthError::Internal(e.into()))?;
        if let Some(key) = key {
            Ok(Some(
                state
                    .storage()
                    .presigned_get(&key, 3600)
                    .await
                    .map_err(AuthError::Internal)?,
            ))
        } else {
            Ok(None)
        }
    } else {
        Ok(None)
    }
}
