// SPDX-License-Identifier: AGPL-3.0-or-later
//! Series, collections and book tags as `/api/v2` reads and writes them
//! (docs/api-v2-completion-plan.md, phase 5).
//!
//! A series or a collection is its owner's, private or shared with the household, and is
//! seen through the viewer's `audiobook` policy like a book (`db::access`). The books in it
//! are kept whoever can see them; each reader is shown, and counted, only the books they can
//! see. Tags are names shared by the whole server (migration 0015); a reader learns of a tag
//! only through a book they can see.
//!
//! Every query binds the viewer as `$1` user, `$2` family, `$3` family admin, then its own
//! parameters from `$4`, as `db::catalog` does.
use crate::db::access::{Viewer, visible_for};
use crate::db::catalog::{BookRow, books};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! viewer {
    ($q:expr, $viewer:expr) => {
        $q.bind($viewer.user_id).bind($viewer.family_id).bind($viewer.is_family_admin)
    };
}

/// The visibility rule for a series or a collection aliased `t`.
fn shelf() -> String {
    visible_for("'audiobook'")
}

/// The visibility rule for a book aliased `b` inside a query whose own `t` is a series or a
/// collection.
fn book_seen() -> String {
    books().replace("t.", "b.")
}

// ── Series ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SeriesRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub created_at: DateTime<Utc>,
    /// `lower(name)`, for the cursor.
    pub sort_key: String,
}

const SERIES_COLS: &str = "t.id, t.user_id, t.family_id, t.name, t.description, t.created_at, lower(t.name) AS sort_key";

/// Visible series in name order after `after`.
pub async fn list_series(pool: &PgPool, viewer: Viewer, after: Option<(String, Uuid)>, limit: i64) -> anyhow::Result<Vec<SeriesRow>> {
    let sql = format!(
        "SELECT {SERIES_COLS} FROM audiobook_series t
          WHERE {} AND ($4::text IS NULL OR (lower(t.name), t.id) > ($4, $5))
          ORDER BY lower(t.name), t.id LIMIT $6",
        shelf()
    );
    let (name, id) = after.unzip();
    viewer!(sqlx::query_as::<_, SeriesRow>(&sql), viewer)
        .bind(name)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list series")
}

/// The visible series among `ids`, in id order.
pub async fn series_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<SeriesRow>> {
    let sql = format!("SELECT {SERIES_COLS} FROM audiobook_series t WHERE t.id = ANY($4) AND {} ORDER BY t.id", shelf());
    viewer!(sqlx::query_as::<_, SeriesRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: series by ids")
}

pub async fn find_series(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<SeriesRow>> {
    Ok(series_by_ids(pool, viewer, &[id]).await?.into_iter().next())
}

/// The books of these series the viewer can see: (series, book, place), in order.
pub async fn series_books(pool: &PgPool, viewer: Viewer, series_ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, Uuid, f64)>> {
    let sql = format!(
        "SELECT sb.series_id, sb.book_id, sb.position FROM audiobook_series_books sb JOIN audiobook_books b ON b.id = sb.book_id
          WHERE sb.series_id = ANY($4) AND {}
          ORDER BY sb.series_id, sb.position, sb.book_id",
        book_seen()
    );
    viewer!(sqlx::query_as::<_, (Uuid, Uuid, f64)>(&sql), viewer)
        .bind(series_ids)
        .fetch_all(pool)
        .await
        .context("db: series books")
}

/// The series a book is in that the viewer can see: (series id, name, place).
pub async fn book_series(pool: &PgPool, viewer: Viewer, book_id: Uuid) -> anyhow::Result<Vec<(Uuid, String, f64)>> {
    let sql = format!(
        "SELECT t.id, t.name, sb.position FROM audiobook_series_books sb JOIN audiobook_series t ON t.id = sb.series_id
          WHERE sb.book_id = $4 AND {} ORDER BY lower(t.name), t.id",
        shelf()
    );
    viewer!(sqlx::query_as::<_, (Uuid, String, f64)>(&sql), viewer)
        .bind(book_id)
        .fetch_all(pool)
        .await
        .context("db: book series")
}

pub async fn insert_series(pool: &PgPool, user_id: Uuid, family_id: Option<Uuid>, name: &str, description: Option<&str>) -> anyhow::Result<Uuid> {
    sqlx::query_scalar("INSERT INTO audiobook_series (user_id, family_id, name, description) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(user_id)
        .bind(family_id)
        .bind(name)
        .bind(description)
        .fetch_one(pool)
        .await
        .context("db: insert series")
}

pub async fn update_series(pool: &PgPool, id: Uuid, name: &str, description: Option<&str>, family_id: Option<Uuid>) -> anyhow::Result<()> {
    sqlx::query("UPDATE audiobook_series SET name = $2, description = $3, family_id = $4 WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(family_id)
        .execute(pool)
        .await
        .context("db: update series")?;
    Ok(())
}

pub async fn delete_series(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_series WHERE id = $1").bind(id).execute(pool).await.context("db: delete series")?;
    Ok(())
}

/// Replaces the books of a series the viewer can see with `books` (book, place), in one
/// transaction, so the change feed has one row for the series. Books the viewer cannot see
/// stay where they are. `Err(book)` names the first book that is not one the viewer can see.
pub async fn set_series_books(pool: &PgPool, viewer: Viewer, series_id: Uuid, books_in: &[(Uuid, f64)]) -> anyhow::Result<Result<(), Uuid>> {
    let mut tx = pool.begin().await.context("db: begin series books")?;
    // Taken in turn with every other writer of this series.
    sqlx::query("SELECT 1 FROM audiobook_series WHERE id = $1 FOR UPDATE")
        .bind(series_id)
        .execute(&mut *tx)
        .await
        .context("db: lock series")?;
    let ids: Vec<Uuid> = books_in.iter().map(|(id, _)| *id).collect();
    let sql = format!("SELECT b.id FROM audiobook_books b WHERE b.id = ANY($4) AND {}", book_seen());
    let seen: Vec<Uuid> = viewer!(sqlx::query_scalar::<_, Uuid>(&sql), viewer)
        .bind(&ids)
        .fetch_all(&mut *tx)
        .await
        .context("db: series books seen")?;
    if let Some(unknown) = ids.iter().find(|id| !seen.contains(id)) {
        return Ok(Err(*unknown));
    }
    let sql = format!(
        "DELETE FROM audiobook_series_books sb USING audiobook_books b
          WHERE sb.series_id = $4 AND b.id = sb.book_id AND NOT (sb.book_id = ANY($5)) AND {}",
        book_seen()
    );
    viewer!(sqlx::query(&sql), viewer)
        .bind(series_id)
        .bind(&ids)
        .execute(&mut *tx)
        .await
        .context("db: series books removed")?;
    let places: Vec<f64> = books_in.iter().map(|(_, p)| *p).collect();
    sqlx::query(
        "INSERT INTO audiobook_series_books (series_id, book_id, position)
         SELECT $1, x.book_id, x.position FROM unnest($2::uuid[], $3::float8[]) AS x(book_id, position)
         ON CONFLICT (series_id, book_id) DO UPDATE SET position = EXCLUDED.position
          WHERE audiobook_series_books.position IS DISTINCT FROM EXCLUDED.position",
    )
    .bind(series_id)
    .bind(&ids)
    .bind(&places)
    .execute(&mut *tx)
    .await
    .context("db: series books placed")?;
    tx.commit().await.context("db: commit series books")?;
    Ok(Ok(()))
}

// ── Collections ──────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CollectionRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub cover_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// What the viewer sees of it.
    pub book_count: i64,
    /// The first visible book's cover, for a collection without its own.
    pub first_cover: Option<Uuid>,
    pub sort_key: String,
}

/// Visible collections in name order after `after`, or those among `ids`.
pub async fn list_collections(
    pool: &PgPool,
    viewer: Viewer,
    ids: Option<&[Uuid]>,
    after: Option<(String, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<CollectionRow>> {
    // The collection is `t` for the visibility rule; each book inside the count is `b`.
    let sql = format!(
        "WITH c AS (
            SELECT t.id, t.user_id, t.family_id, t.name, t.description, t.cover_object_id, t.created_at, t.updated_at
              FROM audiobook_collections t
             WHERE {} AND ($4::uuid[] IS NULL OR t.id = ANY($4))
               AND ($5::text IS NULL OR (lower(t.name), t.id) > ($5, $6))
             ORDER BY lower(t.name), t.id
             LIMIT $7)
         SELECT c.*, s.n AS book_count, s.cover AS first_cover, lower(c.name) AS sort_key
           FROM c
           LEFT JOIN LATERAL (
                SELECT count(*) AS n,
                       (array_agg(b.cover_object_id ORDER BY cb.position, cb.book_id) FILTER (WHERE b.cover_object_id IS NOT NULL))[1] AS cover
                  FROM audiobook_collection_books cb JOIN audiobook_books b ON b.id = cb.book_id
                 WHERE cb.collection_id = c.id AND {}) s ON true
          ORDER BY lower(c.name), c.id",
        shelf(),
        book_seen()
    );
    let (name, id) = after.unzip();
    viewer!(sqlx::query_as::<_, CollectionRow>(&sql), viewer)
        .bind(ids)
        .bind(name)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list collections")
}

pub async fn find_collection(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<CollectionRow>> {
    Ok(list_collections(pool, viewer, Some(&[id]), None, 1).await?.into_iter().next())
}

/// A collection's books the viewer can see, in order, after (place, book).
pub async fn collection_books(
    pool: &PgPool,
    viewer: Viewer,
    collection_id: Uuid,
    after: Option<(i32, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<(i32, DateTime<Utc>, BookRow)>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        entry_position: i32,
        added_at: DateTime<Utc>,
        #[sqlx(flatten)]
        book: BookRow,
    }
    // The book is `t` here, as `BOOK_COLS` and the visibility rule expect.
    let sql = format!(
        "SELECT cb.position AS entry_position, cb.added_at, {}
           FROM audiobook_collection_books cb JOIN audiobook_books t ON t.id = cb.book_id
          WHERE cb.collection_id = $4 AND ($5::int IS NULL OR (cb.position, cb.book_id) > ($5, $6)) AND {}
          ORDER BY cb.position, cb.book_id
          LIMIT $7",
        crate::db::catalog::BOOK_COLS,
        books()
    );
    let (position, book) = after.unzip();
    let rows = viewer!(sqlx::query_as::<_, Row>(&sql), viewer)
        .bind(collection_id)
        .bind(position)
        .bind(book)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: collection books")?;
    Ok(rows.into_iter().map(|r| (r.entry_position, r.added_at, r.book)).collect())
}

pub async fn insert_collection(pool: &PgPool, user_id: Uuid, family_id: Option<Uuid>, name: &str, description: Option<&str>) -> anyhow::Result<Uuid> {
    sqlx::query_scalar("INSERT INTO audiobook_collections (user_id, family_id, name, description) VALUES ($1, $2, $3, $4) RETURNING id")
        .bind(user_id)
        .bind(family_id)
        .bind(name)
        .bind(description)
        .fetch_one(pool)
        .await
        .context("db: insert collection")
}

pub async fn update_collection(pool: &PgPool, id: Uuid, name: &str, description: Option<&str>, family_id: Option<Uuid>) -> anyhow::Result<()> {
    sqlx::query("UPDATE audiobook_collections SET name = $2, description = $3, family_id = $4, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(name)
        .bind(description)
        .bind(family_id)
        .execute(pool)
        .await
        .context("db: update collection")?;
    Ok(())
}

pub async fn delete_collection(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM audiobook_collections WHERE id = $1").bind(id).execute(pool).await.context("db: delete collection")?;
    Ok(())
}

/// Appends the books the viewer can see, in order, after the collection's last; a book
/// already in it stays where it is. One transaction. How many were added.
pub async fn add_collection_books(pool: &PgPool, viewer: Viewer, collection_id: Uuid, book_ids: &[Uuid]) -> anyhow::Result<u64> {
    let mut tx = pool.begin().await.context("db: begin collection add")?;
    sqlx::query("SELECT 1 FROM audiobook_collections WHERE id = $1 FOR UPDATE")
        .bind(collection_id)
        .execute(&mut *tx)
        .await
        .context("db: lock collection")?;
    let sql = format!(
        "INSERT INTO audiobook_collection_books (collection_id, book_id, position)
         SELECT $4, x.id,
                COALESCE((SELECT max(position) FROM audiobook_collection_books WHERE collection_id = $4), 0)
                  + row_number() OVER (ORDER BY min(x.ord))
           FROM unnest($5::uuid[]) WITH ORDINALITY AS x(id, ord)
           JOIN audiobook_books b ON b.id = x.id
          WHERE {} AND NOT EXISTS (SELECT 1 FROM audiobook_collection_books e WHERE e.collection_id = $4 AND e.book_id = x.id)
          GROUP BY x.id
         ON CONFLICT DO NOTHING",
        book_seen()
    );
    let added = viewer!(sqlx::query(&sql), viewer)
        .bind(collection_id)
        .bind(book_ids)
        .execute(&mut *tx)
        .await
        .context("db: add collection books")?
        .rows_affected();
    if added > 0 {
        sqlx::query("UPDATE audiobook_collections SET updated_at = now() WHERE id = $1")
            .bind(collection_id)
            .execute(&mut *tx)
            .await
            .context("db: touch collection")?;
    }
    tx.commit().await.context("db: commit collection add")?;
    Ok(added)
}

pub async fn remove_collection_book(pool: &PgPool, collection_id: Uuid, book_id: Uuid) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin collection remove")?;
    let done = sqlx::query("DELETE FROM audiobook_collection_books WHERE collection_id = $1 AND book_id = $2")
        .bind(collection_id)
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .context("db: remove collection book")?
        .rows_affected()
        > 0;
    if done {
        sqlx::query("UPDATE audiobook_collections SET updated_at = now() WHERE id = $1")
            .bind(collection_id)
            .execute(&mut *tx)
            .await
            .context("db: touch collection")?;
    }
    tx.commit().await.context("db: commit collection remove")?;
    Ok(done)
}

// ── Tags ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TagRow {
    pub id: Uuid,
    pub name: String,
    /// Books the viewer can see that carry it.
    pub book_count: i64,
    pub sort_key: String,
}

/// Tags on books the viewer can see, with how many, in name order after `after`.
pub async fn list_tags(pool: &PgPool, viewer: Viewer, after: Option<(String, Uuid)>, limit: i64) -> anyhow::Result<Vec<TagRow>> {
    let sql = format!(
        "SELECT g.id, g.name, count(*) AS book_count, lower(g.name) AS sort_key
           FROM audiobook_tags g
           JOIN audiobook_book_tags bt ON bt.tag_id = g.id
           JOIN audiobook_books t ON t.id = bt.book_id
          WHERE {} AND ($4::text IS NULL OR (lower(g.name), g.id) > ($4, $5))
          GROUP BY g.id
          ORDER BY lower(g.name), g.id
          LIMIT $6",
        books()
    );
    let (name, id) = after.unzip();
    viewer!(sqlx::query_as::<_, TagRow>(&sql), viewer)
        .bind(name)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list book tags")
}

/// A book's tags by name.
pub async fn book_tags(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<String>> {
    sqlx::query_scalar(
        "SELECT g.name FROM audiobook_book_tags bt JOIN audiobook_tags g ON g.id = bt.tag_id
          WHERE bt.book_id = $1 ORDER BY lower(g.name), g.id",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: book tags")
}

/// Replaces a book's tags with `names` (already trimmed and each once, any case), making
/// the tags that do not exist yet. One transaction, so one change of the book.
pub async fn set_book_tags(pool: &PgPool, book_id: Uuid, names: &[String]) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin book tags")?;
    // An existing tag keeps its spelling; the unique index is on `lower(name)`.
    sqlx::query(
        "INSERT INTO audiobook_tags (name) SELECT n FROM unnest($1::text[]) AS n
          WHERE NOT EXISTS (SELECT 1 FROM audiobook_tags g WHERE lower(g.name) = lower(n))
         ON CONFLICT DO NOTHING",
    )
    .bind(names)
    .execute(&mut *tx)
    .await
    .context("db: make tags")?;
    let lowered: Vec<String> = names.iter().map(|n| n.to_lowercase()).collect();
    sqlx::query(
        "DELETE FROM audiobook_book_tags bt USING audiobook_tags g
          WHERE bt.book_id = $1 AND g.id = bt.tag_id AND NOT (lower(g.name) = ANY($2))",
    )
    .bind(book_id)
    .bind(&lowered)
    .execute(&mut *tx)
    .await
    .context("db: drop book tags")?;
    sqlx::query(
        "INSERT INTO audiobook_book_tags (book_id, tag_id)
         SELECT $1, g.id FROM audiobook_tags g WHERE lower(g.name) = ANY($2)
         ON CONFLICT DO NOTHING",
    )
    .bind(book_id)
    .bind(&lowered)
    .execute(&mut *tx)
    .await
    .context("db: add book tags")?;
    tx.commit().await.context("db: commit book tags")?;
    Ok(())
}

// ── A book's files ───────────────────────────────────────────────────────────

/// Puts `first` at the front of a book's files in that order, the others after them as they
/// were, numbered 1…n, in one transaction. `false` when an id is not this book's or comes twice.
pub async fn order_book_files(pool: &PgPool, book_id: Uuid, first: &[Uuid]) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin file order")?;
    sqlx::query("SELECT 1 FROM audiobook_books WHERE id = $1 FOR UPDATE")
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .context("db: lock book")?;
    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM audiobook_files WHERE book_id = $1 AND id = ANY($2)")
        .bind(book_id)
        .bind(first)
        .fetch_one(&mut *tx)
        .await
        .context("db: check book files")?;
    let distinct: std::collections::HashSet<&Uuid> = first.iter().collect();
    if known != first.len() as i64 || distinct.len() != first.len() {
        return Ok(false);
    }
    // Negative first: (book_id, position) is unique per statement.
    sqlx::query("UPDATE audiobook_files SET position = -position WHERE book_id = $1")
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .context("db: clear file positions")?;
    sqlx::query(
        "UPDATE audiobook_files f SET position = o.n
           FROM (SELECT f2.id, row_number() OVER (ORDER BY x.ord NULLS LAST, -f2.position) AS n
                   FROM audiobook_files f2
                   LEFT JOIN unnest($2::uuid[]) WITH ORDINALITY AS x(id, ord) ON x.id = f2.id
                  WHERE f2.book_id = $1) o
          WHERE f.id = o.id",
    )
    .bind(book_id)
    .bind(first)
    .execute(&mut *tx)
    .await
    .context("db: order book files")?;
    sqlx::query("UPDATE audiobook_books SET updated_at = now() WHERE id = $1")
        .bind(book_id)
        .execute(&mut *tx)
        .await
        .context("db: touch book")?;
    tx.commit().await.context("db: commit file order")?;
    Ok(true)
}

/// Title, description, publisher, year and ISBN of a book, as given.
pub async fn update_book_details(
    pool: &PgPool,
    id: Uuid,
    title: &str,
    description: Option<&str>,
    publisher: Option<&str>,
    published_year: Option<i32>,
    isbn: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE audiobook_books SET title = $2, description = $3, publisher = $4, published_year = $5, isbn = $6,
                updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(title)
    .bind(description)
    .bind(publisher)
    .bind(published_year)
    .bind(isbn)
    .execute(pool)
    .await
    .context("db: update book details")?;
    Ok(())
}

/// Sets or clears a book's cover; the one it had, for the caller to let go of.
pub async fn set_book_cover(pool: &PgPool, id: Uuid, cover: Option<Uuid>) -> anyhow::Result<Option<Uuid>> {
    let mut tx = pool.begin().await.context("db: begin book cover")?;
    let old: Option<Option<Uuid>> = sqlx::query_scalar("SELECT cover_object_id FROM audiobook_books WHERE id = $1 FOR UPDATE")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .context("db: book cover")?;
    sqlx::query("UPDATE audiobook_books SET cover_object_id = $2, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(cover)
        .execute(&mut *tx)
        .await
        .context("db: set book cover")?;
    tx.commit().await.context("db: commit book cover")?;
    Ok(old.flatten().filter(|o| Some(*o) != cover))
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_book_inside_a_shelf_query_is_b() {
        let rule = super::book_seen();
        assert!(!rule.contains("t."), "{rule}");
        assert!(rule.contains("b.user_id") && rule.contains("b.family_id") && rule.contains("b.id"), "{rule}");
    }
}
