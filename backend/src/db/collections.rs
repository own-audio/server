// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use crate::audiobooks::models::{Collection, Series, SeriesBook, Favorite};

// ── Collections ─────────────────────────────────────────────────────────────

pub async fn list_collections(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<Collection>> {
    sqlx::query_as::<_, Collection>(
        "SELECT id, user_id, family_id, name, description, cover_object_id, is_public, created_at, updated_at
         FROM audiobook_collections WHERE user_id = $1 ORDER BY name ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list collections")
}

pub async fn find_collection(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<Collection>> {
    sqlx::query_as::<_, Collection>(
        "SELECT id, user_id, family_id, name, description, cover_object_id, is_public, created_at, updated_at
         FROM audiobook_collections WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find collection")
}

pub async fn create_collection(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
    is_public: bool,
) -> anyhow::Result<Collection> {
    sqlx::query_as::<_, Collection>(
        "INSERT INTO audiobook_collections (user_id, name, description, is_public)
         VALUES ($1, $2, $3, $4)
         RETURNING id, user_id, family_id, name, description, cover_object_id, is_public, created_at, updated_at",
    )
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(is_public)
    .fetch_one(pool)
    .await
    .context("db: create collection")
}

pub async fn update_collection(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
    is_public: bool,
) -> anyhow::Result<Collection> {
    sqlx::query_as::<_, Collection>(
        "UPDATE audiobook_collections
         SET name = $3, description = $4, is_public = $5, updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2
         RETURNING id, user_id, family_id, name, description, cover_object_id, is_public, created_at, updated_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(is_public)
    .fetch_one(pool)
    .await
    .context("db: update collection")
}

pub async fn delete_collection(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_collections WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete collection")?;
    Ok(())
}

pub async fn list_collection_book_ids(pool: &PgPool, collection_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT book_id FROM audiobook_collection_books
         WHERE collection_id = $1 ORDER BY position ASC, added_at ASC",
    )
    .bind(collection_id)
    .fetch_all(pool)
    .await
    .context("db: list collection books")
}

pub async fn add_book_to_collection(
    pool: &PgPool,
    collection_id: Uuid,
    book_id: Uuid,
) -> anyhow::Result<()> {
    // Get next position
    let next_pos = sqlx::query_scalar::<_, Option<i32>>(
        "SELECT MAX(position) FROM audiobook_collection_books WHERE collection_id = $1",
    )
    .bind(collection_id)
    .fetch_one(pool)
    .await
    .context("db: collection max position")?
    .unwrap_or(0)
        + 1;

    sqlx::query(
        "INSERT INTO audiobook_collection_books (collection_id, book_id, position)
         VALUES ($1, $2, $3) ON CONFLICT DO NOTHING",
    )
    .bind(collection_id)
    .bind(book_id)
    .bind(next_pos)
    .execute(pool)
    .await
    .context("db: add book to collection")?;
    Ok(())
}

pub async fn remove_book_from_collection(
    pool: &PgPool,
    collection_id: Uuid,
    book_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM audiobook_collection_books WHERE collection_id = $1 AND book_id = $2",
    )
    .bind(collection_id)
    .bind(book_id)
    .execute(pool)
    .await
    .context("db: remove book from collection")?;
    Ok(())
}

pub async fn collection_book_count(pool: &PgPool, collection_id: Uuid) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audiobook_collection_books WHERE collection_id = $1",
    )
    .bind(collection_id)
    .fetch_one(pool)
    .await
    .context("db: collection book count")
}

// ── Series ──────────────────────────────────────────────────────────────────

pub async fn list_series(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<Series>> {
    sqlx::query_as::<_, Series>(
        "SELECT id, user_id, family_id, name, description, created_at
         FROM audiobook_series WHERE user_id = $1 ORDER BY name ASC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list series")
}

pub async fn find_series(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<Option<Series>> {
    sqlx::query_as::<_, Series>(
        "SELECT id, user_id, family_id, name, description, created_at
         FROM audiobook_series WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find series")
}

pub async fn create_series(
    pool: &PgPool,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
) -> anyhow::Result<Series> {
    sqlx::query_as::<_, Series>(
        "INSERT INTO audiobook_series (user_id, name, description)
         VALUES ($1, $2, $3)
         RETURNING id, user_id, family_id, name, description, created_at",
    )
    .bind(user_id)
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await
    .context("db: create series")
}

pub async fn update_series(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
) -> anyhow::Result<Series> {
    sqlx::query_as::<_, Series>(
        "UPDATE audiobook_series SET name = $3, description = $4
         WHERE id = $1 AND user_id = $2
         RETURNING id, user_id, family_id, name, description, created_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .fetch_one(pool)
    .await
    .context("db: update series")
}

pub async fn delete_series(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_series WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete series")?;
    Ok(())
}

pub async fn list_series_books(pool: &PgPool, series_id: Uuid) -> anyhow::Result<Vec<SeriesBook>> {
    sqlx::query_as::<_, SeriesBook>(
        "SELECT series_id, book_id, position
         FROM audiobook_series_books
         WHERE series_id = $1 ORDER BY position ASC",
    )
    .bind(series_id)
    .fetch_all(pool)
    .await
    .context("db: list series books")
}

pub async fn add_book_to_series(
    pool: &PgPool,
    series_id: Uuid,
    book_id: Uuid,
    position: f64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audiobook_series_books (series_id, book_id, position)
         VALUES ($1, $2, $3) ON CONFLICT (series_id, book_id) DO UPDATE SET position = $3",
    )
    .bind(series_id)
    .bind(book_id)
    .bind(position)
    .execute(pool)
    .await
    .context("db: add book to series")?;
    Ok(())
}

pub async fn remove_book_from_series(
    pool: &PgPool,
    series_id: Uuid,
    book_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_series_books WHERE series_id = $1 AND book_id = $2")
        .bind(series_id)
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: remove book from series")?;
    Ok(())
}

// ── Favorites ───────────────────────────────────────────────────────────────

pub async fn list_favorites(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<Favorite>> {
    sqlx::query_as::<_, Favorite>(
        "SELECT user_id, book_id, created_at FROM audiobook_favorites
         WHERE user_id = $1 ORDER BY created_at DESC",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list favorites")
}

pub async fn is_favorite(pool: &PgPool, user_id: Uuid, book_id: Uuid) -> anyhow::Result<bool> {
    let count = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*) FROM audiobook_favorites WHERE user_id = $1 AND book_id = $2",
    )
    .bind(user_id)
    .bind(book_id)
    .fetch_one(pool)
    .await
    .context("db: check favorite")?;
    Ok(count > 0)
}

pub async fn add_favorite(pool: &PgPool, user_id: Uuid, book_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO audiobook_favorites (user_id, book_id) VALUES ($1, $2) ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(book_id)
    .execute(pool)
    .await
    .context("db: add favorite")?;
    Ok(())
}

pub async fn remove_favorite(pool: &PgPool, user_id: Uuid, book_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_favorites WHERE user_id = $1 AND book_id = $2")
        .bind(user_id)
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: remove favorite")?;
    Ok(())
}
