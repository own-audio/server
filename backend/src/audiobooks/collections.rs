// SPDX-License-Identifier: AGPL-3.0-or-later
/// Collections & series endpoints — CRUD, add/remove books, favorites.
use crate::app::AppState;
use crate::auth::error::AuthError;
use crate::db;
use crate::families::FamilyContext;
use axum::Router;
use axum::extract::{Json, Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, post, put};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ── DTOs ────────────────────────────────────────────────────────────────────

#[derive(Serialize)]
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

#[derive(Deserialize)]
pub struct CreateCollectionRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub is_public: bool,
}

#[derive(Deserialize)]
pub struct UpdateCollectionRequest {
    pub name: String,
    pub description: Option<String>,
    #[serde(default)]
    pub is_public: bool,
}

#[derive(Serialize)]
pub struct SeriesResponse {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub books: Vec<SeriesBookEntry>,
    pub created_at: String,
}

#[derive(Serialize)]
pub struct SeriesBookEntry {
    pub book_id: String,
    pub book_title: String,
    pub position: f64,
}

#[derive(Deserialize)]
pub struct CreateSeriesRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateSeriesRequest {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Deserialize)]
pub struct AddBookToSeriesRequest {
    pub book_id: String,
    pub position: f64,
}

#[derive(Deserialize)]
pub struct AddBookRequest {
    pub book_id: String,
}

// ── Router ──────────────────────────────────────────────────────────────────

pub fn router() -> Router<AppState> {
    Router::new()
        // Collections
        .route("/collections", get(list_collections))
        .route("/collections", post(create_collection))
        .route("/collections/{id}", get(get_collection))
        .route("/collections/{id}", put(update_collection))
        .route("/collections/{id}", delete(delete_collection))
        .route("/collections/{id}/books", get(list_collection_books))
        .route("/collections/{id}/books", post(add_book_to_collection))
        .route("/collections/{id}/books/{book_id}", delete(remove_book_from_collection))
        // Series
        .route("/series", get(list_series))
        .route("/series", post(create_series))
        .route("/series/{id}", get(get_series))
        .route("/series/{id}", put(update_series))
        .route("/series/{id}", delete(delete_series))
        .route("/series/{id}/books", post(add_book_to_series))
        .route("/series/{id}/books/{book_id}", delete(remove_book_from_series))
        // Favorites
        .route("/favorites", get(list_favorites))
        .route("/favorites/{book_id}", post(add_favorite))
        .route("/favorites/{book_id}", delete(remove_favorite))
        .route("/favorites/{book_id}/check", get(check_favorite))
}

// ── Collection handlers ─────────────────────────────────────────────────────

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
