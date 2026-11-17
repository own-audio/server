// SPDX-License-Identifier: AGPL-3.0-or-later
//! Series and collections of books (docs/api-v2-completion-plan.md, phase 5).
//!
//! Both are their owner's, private or shared with the household (`visibility`, as everything
//! else), and seen through the reader's audiobook policy, like a book. Anyone who can see one
//! reads it; only its owner changes it; its owner, or a household admin for one shared with
//! the household, deletes it. A reader is shown, and counted, only the books they can see;
//! the books they cannot see stay where they are when the owner changes the list.
//!
//! - A series is ordered by each book's place in it (`1`, `2`, `2.5`…), and its record holds
//!   its books, since a series is short. `PUT /series/{id}/books` replaces the list in one go.
//! - A collection is the playlist shape for books: its record counts its books, which are a
//!   paged read in the order they were added.
//!
//! Both are kinds of the change feed (`series`, `collection`); a book's record names the series
//! it is in, so a series' name or place changes the book too.

use crate::app::AppState;
use crate::db;
use crate::db::access::{self, visibility_of};
use crate::db::shelves::{CollectionRow, SeriesRow};
use crate::families::FamilyContext;
use crate::v2::edit::present;
use crate::v2::page::{Page, PageQuery};
use crate::v2::problem::Problem;
use crate::v2::spoken::{self, Book};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;
use uuid::Uuid;

/// The most books one request names.
const MAX_BOOKS: usize = 1000;

fn name_of(name: &str) -> Result<String, Problem> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return Err(Problem::bad_request("invalid_body", "a name is 1 to 200 characters").field("/name", "invalid", "1 to 200 characters"));
    }
    Ok(name.to_string())
}

fn description_of(description: Option<&str>) -> Result<Option<String>, Problem> {
    let description = description.map(str::trim).filter(|d| !d.is_empty());
    if description.is_some_and(|d| d.chars().count() > 2000) {
        return Err(Problem::bad_request("invalid_body", "a description is at most 2000 characters").field("/description", "invalid", "at most 2000 characters"));
    }
    Ok(description.map(str::to_string))
}

fn family_for(visibility: Option<&str>, family: &FamilyContext) -> Result<Option<Uuid>, Problem> {
    access::family_id_for_visibility(visibility, family.family_id)
        .map_err(|e| Problem::bad_request("invalid_body", e).field("/visibility", "invalid", "private or family"))
}

fn too_many(n: usize) -> Result<(), Problem> {
    if n > MAX_BOOKS {
        return Err(Problem::bad_request("too_many", format!("at most {MAX_BOOKS} books at a time")));
    }
    Ok(())
}

/// May change it: its owner.
fn owner_of(owner_id: Uuid, family: &FamilyContext, what: &str) -> Result<(), Problem> {
    if owner_id == family.user_id { Ok(()) } else { Err(Problem::forbidden().detail(format!("only its owner changes a {what}"))) }
}

/// May delete it: its owner, or a household admin for one shared with the household.
fn deleter_of(owner_id: Uuid, family_id: Option<Uuid>, family: &FamilyContext, what: &str) -> Result<(), Problem> {
    if owner_id == family.user_id || (family.is_family_admin() && family_id == Some(family.family_id)) {
        Ok(())
    } else {
        Err(Problem::forbidden().detail(format!("only its owner, or a household admin for a shared one, deletes a {what}")))
    }
}

// ── Series ───────────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Series {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// `private` or `family`.
    pub visibility: String,
    pub owner_id: Uuid,
    /// The books the caller can see, by their place.
    pub books: Vec<SeriesBook>,
    pub created_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize, ToSchema, Clone)]
pub struct SeriesBook {
    pub book_id: Uuid,
    /// Its place: fractional when the publisher numbers a book between two others (`1.5`).
    /// When sending, leave it out to number the books 1, 2, 3… as listed.
    #[serde(default)]
    #[schema(value_type = f64)]
    pub position: Option<f64>,
}

/// These series as the viewer sees them, each with its visible books.
pub(crate) async fn series_records(state: &AppState, family: &FamilyContext, rows: Vec<SeriesRow>) -> Result<Vec<Series>, Problem> {
    let ids: Vec<Uuid> = rows.iter().map(|s| s.id).collect();
    let mut books: HashMap<Uuid, Vec<SeriesBook>> = HashMap::new();
    for (series_id, book_id, position) in db::shelves::series_books(state.db(), family.viewer(), &ids).await? {
        books.entry(series_id).or_default().push(SeriesBook { book_id, position: Some(position) });
    }
    Ok(rows
        .into_iter()
        .map(|s| Series {
            books: books.remove(&s.id).unwrap_or_default(),
            id: s.id,
            name: s.name,
            description: s.description,
            visibility: visibility_of(s.family_id).to_string(),
            owner_id: s.user_id,
            created_at: s.created_at,
        })
        .collect())
}

async fn find_series(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<SeriesRow, Problem> {
    db::shelves::find_series(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)
}

async fn series_answer(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<Series, Problem> {
    let row = find_series(state, family, id).await?;
    series_records(state, family, vec![row]).await?.pop().ok_or_else(Problem::not_found)
}

/// The books as (book, place), numbered from 1 where a place is left out; each book once.
fn places(books: &[SeriesBook]) -> Result<Vec<(Uuid, f64)>, Problem> {
    too_many(books.len())?;
    let mut out: Vec<(Uuid, f64)> = Vec::with_capacity(books.len());
    for (i, b) in books.iter().enumerate() {
        let position = b.position.unwrap_or((i + 1) as f64);
        if !position.is_finite() || !(0.0..=1_000_000.0).contains(&position) {
            return Err(Problem::bad_request("invalid_body", "a place is a number from 0 to 1,000,000")
                .field(format!("/books/{i}/position"), "out_of_range", "0 to 1000000"));
        }
        if out.iter().any(|(id, _)| *id == b.book_id) {
            return Err(Problem::bad_request("invalid_book", "a book is in a series once").field(format!("/books/{i}/book_id"), "repeated", b.book_id.to_string()));
        }
        out.push((b.book_id, position));
    }
    Ok(out)
}

async fn place_books(state: &AppState, family: &FamilyContext, id: Uuid, books: &[(Uuid, f64)]) -> Result<(), Problem> {
    if let Err(unknown) = db::shelves::set_series_books(state.db(), family.viewer(), id, books).await? {
        let i = books.iter().position(|(b, _)| *b == unknown).unwrap_or(0);
        return Err(Problem::bad_request("invalid_book", "a book that is not one you can see").field(format!("/books/{i}/book_id"), "unknown", unknown.to_string()));
    }
    Ok(())
}

/// Series the caller can see, by name.
#[utoipa::path(get, path = "/series", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<Series>)))]
pub async fn list_series(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<Series>>, Problem> {
    let limit = page.limit()?;
    let rows = db::shelves::list_series(state.db(), family.viewer(), page.after::<(String, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |s| (s.sort_key.clone(), s.id));
    Ok(Json(Page { items: series_records(&state, &family, page.items).await?, next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct NewSeries {
    pub name: String,
    pub description: Option<String>,
    /// `private` (the default) or `family`.
    pub visibility: Option<String>,
    /// Books to start with, in order.
    #[serde(default)]
    pub books: Vec<SeriesBook>,
}

/// Make a series. Send an `Idempotency-Key` to retry safely.
#[utoipa::path(post, path = "/series", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("can_upload"))),
    request_body = NewSeries,
    responses((status = 201, body = Series),
        (status = 400, description = "`invalid_body`, `invalid_book` (one you cannot see, or twice), `too_many`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not allowed to add to the library", body = crate::v2::problem::ProblemBody)))]
pub async fn create_series(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewSeries>) -> Result<(StatusCode, Json<Series>), Problem> {
    family.require_can_upload()?;
    let name = name_of(&body.name)?;
    let description = description_of(body.description.as_deref())?;
    let family_id = family_for(body.visibility.as_deref(), &family)?;
    let books = places(&body.books)?;
    let id = db::shelves::insert_series(state.db(), family.user_id, family_id, &name, description.as_deref()).await?;
    if !books.is_empty() {
        if let Err(problem) = place_books(&state, &family, id, &books).await {
            db::shelves::delete_series(state.db(), id).await?;
            return Err(problem);
        }
    }
    Ok((StatusCode::CREATED, Json(series_answer(&state, &family, id).await?)))
}

/// One series.
#[utoipa::path(get, path = "/series/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Series), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_series(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Series>, Problem> {
    Ok(Json(series_answer(&state, &family, id).await?))
}

#[derive(Deserialize, ToSchema)]
pub struct SeriesPatch {
    pub name: Option<String>,
    /// `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// `private` or `family`.
    pub visibility: Option<String>,
}

/// Rename a series, describe it, share it or stop sharing it.
#[utoipa::path(patch, path = "/series/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = SeriesPatch,
    responses((status = 200, body = Series),
        (status = 400, description = "`invalid_body`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_series(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<SeriesPatch>) -> Result<Json<Series>, Problem> {
    let current = find_series(&state, &family, id).await?;
    owner_of(current.user_id, &family, "series")?;
    let name = match body.name.as_deref() {
        Some(n) => name_of(n)?,
        None => current.name,
    };
    let description = match body.description {
        Some(d) => description_of(d.as_deref())?,
        None => current.description,
    };
    let family_id = match body.visibility.as_deref() {
        Some(v) => family_for(Some(v), &family)?,
        None => current.family_id,
    };
    db::shelves::update_series(state.db(), id, &name, description.as_deref(), family_id).await?;
    Ok(Json(series_answer(&state, &family, id).await?))
}

/// Delete a series; its books stay.
#[utoipa::path(delete, path = "/series/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Deleted"),
        (status = 403, description = "Neither its owner nor, for a shared one, a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_series(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    let current = find_series(&state, &family, id).await?;
    deleter_of(current.user_id, current.family_id, &family, "series")?;
    db::shelves::delete_series(state.db(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize, ToSchema)]
pub struct SeriesBooks {
    /// The series' books, complete, in order. Books you cannot see stay as they are.
    pub books: Vec<SeriesBook>,
}

/// Replace a series' books and their places in one call.
#[utoipa::path(put, path = "/series/{id}/books", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = SeriesBooks,
    responses((status = 200, description = "The series now", body = Series),
        (status = 400, description = "`invalid_book` (one you cannot see, or twice), `invalid_body`, `too_many`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn put_series_books(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<SeriesBooks>) -> Result<Json<Series>, Problem> {
    let current = find_series(&state, &family, id).await?;
    owner_of(current.user_id, &family, "series")?;
    let books = places(&body.books)?;
    place_books(&state, &family, id, &books).await?;
    Ok(Json(series_answer(&state, &family, id).await?))
}

// ── Collections ──────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct Collection {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    /// `private` or `family`.
    pub visibility: String,
    pub owner_id: Uuid,
    /// Of the books the caller can see.
    pub book_count: i64,
    /// Its first book's cover (`GET /images/{id}`; add `w` for the size).
    pub image: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub(crate) fn collection(state: &AppState, row: CollectionRow) -> Collection {
    Collection {
        id: row.id,
        name: row.name,
        description: row.description,
        visibility: visibility_of(row.family_id).to_string(),
        owner_id: row.user_id,
        book_count: row.book_count,
        image: crate::v2::images::maybe(state, row.cover_object_id.or(row.first_cover)),
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

async fn find_collection(state: &AppState, family: &FamilyContext, id: Uuid) -> Result<CollectionRow, Problem> {
    db::shelves::find_collection(state.db(), family.viewer(), id).await?.ok_or_else(Problem::not_found)
}

/// Collections the caller can see: their own and those shared with them, by name.
#[utoipa::path(get, path = "/collections", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(PageQuery),
    responses((status = 200, body = Page<Collection>)))]
pub async fn list_collections(family: FamilyContext, State(state): State<AppState>, Query(page): Query<PageQuery>) -> Result<Json<Page<Collection>>, Problem> {
    let limit = page.limit()?;
    let rows = db::shelves::list_collections(state.db(), family.viewer(), None, page.after::<(String, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |c| (c.sort_key.clone(), c.id));
    Ok(Json(Page { items: page.items.into_iter().map(|c| collection(&state, c)).collect(), next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct NewCollection {
    pub name: String,
    pub description: Option<String>,
    /// `private` (the default) or `family`.
    pub visibility: Option<String>,
    /// Books to start with, in order; those the caller cannot see are left out.
    #[serde(default)]
    pub book_ids: Vec<Uuid>,
}

/// Make a collection. Send an `Idempotency-Key` to retry safely.
#[utoipa::path(post, path = "/collections", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    request_body = NewCollection,
    responses((status = 201, body = Collection),
        (status = 400, description = "`invalid_body`, `too_many`", body = crate::v2::problem::ProblemBody)))]
pub async fn create_collection(family: FamilyContext, State(state): State<AppState>, Json(body): Json<NewCollection>) -> Result<(StatusCode, Json<Collection>), Problem> {
    let name = name_of(&body.name)?;
    let description = description_of(body.description.as_deref())?;
    let family_id = family_for(body.visibility.as_deref(), &family)?;
    too_many(body.book_ids.len())?;
    let id = db::shelves::insert_collection(state.db(), family.user_id, family_id, &name, description.as_deref()).await?;
    if !body.book_ids.is_empty() {
        db::shelves::add_collection_books(state.db(), family.viewer(), id, &body.book_ids).await?;
    }
    Ok((StatusCode::CREATED, Json(collection(&state, find_collection(&state, &family, id).await?))))
}

/// One collection.
#[utoipa::path(get, path = "/collections/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path)),
    responses((status = 200, body = Collection), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn get_collection(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<Json<Collection>, Problem> {
    Ok(Json(collection(&state, find_collection(&state, &family, id).await?)))
}

#[derive(Deserialize, ToSchema)]
pub struct CollectionPatch {
    pub name: Option<String>,
    /// `null` clears it.
    #[serde(default, deserialize_with = "present")]
    #[schema(value_type = Option<String>)]
    pub description: Option<Option<String>>,
    /// `private` or `family`.
    pub visibility: Option<String>,
}

/// Rename a collection, describe it, share it or stop sharing it.
#[utoipa::path(patch, path = "/collections/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = CollectionPatch,
    responses((status = 200, body = Collection),
        (status = 400, description = "`invalid_body`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn patch_collection(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<CollectionPatch>) -> Result<Json<Collection>, Problem> {
    let current = find_collection(&state, &family, id).await?;
    owner_of(current.user_id, &family, "collection")?;
    let name = match body.name.as_deref() {
        Some(n) => name_of(n)?,
        None => current.name,
    };
    let description = match body.description {
        Some(d) => description_of(d.as_deref())?,
        None => current.description,
    };
    let family_id = match body.visibility.as_deref() {
        Some(v) => family_for(Some(v), &family)?,
        None => current.family_id,
    };
    db::shelves::update_collection(state.db(), id, &name, description.as_deref(), family_id).await?;
    Ok(Json(collection(&state, find_collection(&state, &family, id).await?)))
}

/// Delete a collection; its books stay.
#[utoipa::path(delete, path = "/collections/{id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)),
    responses((status = 204, description = "Deleted"),
        (status = 403, description = "Neither its owner nor, for a shared one, a household admin", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn delete_collection(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>) -> Result<StatusCode, Problem> {
    let current = find_collection(&state, &family, id).await?;
    deleter_of(current.user_id, current.family_id, &family, "collection")?;
    db::shelves::delete_collection(state.db(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// One book of a collection.
#[derive(Serialize, ToSchema)]
pub struct CollectionEntry {
    /// Its place, in the order books were added; places of books the caller cannot see are skipped.
    pub position: i32,
    pub added_at: DateTime<Utc>,
    pub book: Book,
}

/// A collection's books in order.
#[utoipa::path(get, path = "/collections/{id}/books", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("member"))),
    params(("id" = Uuid, Path), PageQuery),
    responses((status = 200, body = Page<CollectionEntry>), (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn list_collection_books(
    family: FamilyContext,
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    Query(page): Query<PageQuery>,
) -> Result<Json<Page<CollectionEntry>>, Problem> {
    find_collection(&state, &family, id).await?;
    let limit = page.limit()?;
    let rows = db::shelves::collection_books(state.db(), family.viewer(), id, page.after::<(i32, Uuid)>()?, limit + 1).await?;
    let page = Page::from_rows(rows, limit, |(position, _, b)| (*position, b.id));
    let ids: Vec<Uuid> = page.items.iter().map(|(_, _, b)| b.id).collect();
    let mut credits = spoken::credits_of(&state, "book", &ids).await?;
    let items = page
        .items
        .into_iter()
        .map(|(position, added_at, row)| {
            let c = credits.remove(&row.id).unwrap_or_default();
            CollectionEntry { position, added_at, book: spoken::book(&state, row, c) }
        })
        .collect();
    Ok(Json(Page { items, next: page.next, included: None }))
}

#[derive(Deserialize, ToSchema)]
pub struct AddBooks {
    /// Appended in this order; a book the caller cannot see, or one already in it, is left out.
    pub book_ids: Vec<Uuid>,
}

/// Add books to the end of a collection.
#[utoipa::path(post, path = "/collections/{id}/books", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path)), request_body = AddBooks,
    responses((status = 200, description = "The collection now", body = Collection),
        (status = 400, description = "`too_many`", body = crate::v2::problem::ProblemBody),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown or not visible", body = crate::v2::problem::ProblemBody)))]
pub async fn add_collection_books(family: FamilyContext, State(state): State<AppState>, Path(id): Path<Uuid>, Json(body): Json<AddBooks>) -> Result<Json<Collection>, Problem> {
    let current = find_collection(&state, &family, id).await?;
    owner_of(current.user_id, &family, "collection")?;
    too_many(body.book_ids.len())?;
    db::shelves::add_collection_books(state.db(), family.viewer(), id, &body.book_ids).await?;
    Ok(Json(collection(&state, find_collection(&state, &family, id).await?)))
}

/// Take a book out of a collection.
#[utoipa::path(delete, path = "/collections/{id}/books/{book_id}", tag = "books", security(("bearer" = [])),
    extensions(("x-permission" = json!("owner"))),
    params(("id" = Uuid, Path), ("book_id" = Uuid, Path)),
    responses((status = 204, description = "Removed"),
        (status = 403, description = "Not its owner", body = crate::v2::problem::ProblemBody),
        (status = 404, description = "Unknown collection, or the book is not in it", body = crate::v2::problem::ProblemBody)))]
pub async fn remove_collection_book(family: FamilyContext, State(state): State<AppState>, Path((id, book_id)): Path<(Uuid, Uuid)>) -> Result<StatusCode, Problem> {
    let current = find_collection(&state, &family, id).await?;
    owner_of(current.user_id, &family, "collection")?;
    if !db::shelves::remove_collection_book(state.db(), id, book_id).await? {
        return Err(Problem::not_found());
    }
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(position: Option<f64>) -> SeriesBook {
        SeriesBook { book_id: Uuid::new_v4(), position }
    }

    #[test]
    fn places_number_what_is_left_out_and_refuse_the_rest() {
        let books = vec![b(None), b(Some(2.5)), b(None)];
        let got: Vec<f64> = places(&books).unwrap().into_iter().map(|(_, p)| p).collect();
        assert_eq!(got, vec![1.0, 2.5, 3.0]);
        assert_eq!(places(&[b(Some(f64::NAN))]).unwrap_err().code(), "invalid_body");
        assert_eq!(places(&[b(Some(-1.0))]).unwrap_err().code(), "invalid_body");
        let twice = b(None);
        assert_eq!(places(&[twice.clone(), twice]).unwrap_err().code(), "invalid_book");
        assert_eq!(places(&vec![b(None); MAX_BOOKS + 1]).unwrap_err().code(), "too_many");
    }

    #[test]
    fn names_are_bounded() {
        assert_eq!(name_of("  ").unwrap_err().code(), "invalid_body");
        assert_eq!(name_of(&"x".repeat(201)).unwrap_err().code(), "invalid_body");
        assert_eq!(name_of(" The Expanse ").unwrap(), "The Expanse");
    }
}
