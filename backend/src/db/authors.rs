// SPDX-License-Identifier: AGPL-3.0-or-later
use anyhow::Context;
use sqlx::PgPool;
use uuid::Uuid;

use crate::audiobooks::models::{Author, BookAuthor, Tag};
use crate::db::access::{AUDIOBOOK, VISIBLE, Viewer};

// ── Authors ─────────────────────────────────────────────────────────────────
//
// Authors, narrators and translators are household contributors (migrations
// 0100, 0102), credited on books with a role. Every function here is scoped
// to a household; there is no server-wide author list any more.

const AUTHOR_COLS: &str = "k.id, k.name, k.sort_name, k.biography AS bio, k.image_object_id, k.created_at, k.updated_at";

/// Contributors credited on a book the viewer can see, with how many such books.
pub async fn list_authors(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<(Author, i64)>> {
    let sql = format!(
        "SELECT {AUTHOR_COLS}, COUNT(DISTINCT t.id) AS book_count
           FROM audiobook_books t
           JOIN credits c ON c.item_kind = 'book' AND c.item_id = t.id
           JOIN contributors k ON k.id = c.contributor_id
          WHERE {VISIBLE}
          GROUP BY k.id
          ORDER BY lower(COALESCE(k.sort_name, k.name)), k.id"
    );
    let rows: Vec<AuthorWithCount> = sqlx::query_as(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(AUDIOBOOK)
        .fetch_all(pool)
        .await
        .context("db: list authors")?;
    Ok(rows.into_iter().map(|r| (r.author(), r.book_count)).collect())
}

#[derive(sqlx::FromRow)]
struct AuthorWithCount {
    id: Uuid,
    name: String,
    sort_name: Option<String>,
    bio: Option<String>,
    image_object_id: Option<Uuid>,
    created_at: chrono::DateTime<chrono::Utc>,
    updated_at: chrono::DateTime<chrono::Utc>,
    book_count: i64,
}

impl AuthorWithCount {
    fn author(&self) -> Author {
        Author {
            id: self.id,
            name: self.name.clone(),
            sort_name: self.sort_name.clone(),
            bio: self.bio.clone(),
            image_object_id: self.image_object_id,
            created_at: self.created_at,
            updated_at: self.updated_at,
        }
    }
}

/// A contributor of this household.
pub async fn find_author(pool: &PgPool, family_id: Uuid, id: Uuid) -> anyhow::Result<Option<Author>> {
    sqlx::query_as::<_, Author>(&format!("SELECT {AUTHOR_COLS} FROM contributors k WHERE k.id = $1 AND k.family_id = $2"))
        .bind(id)
        .bind(family_id)
        .fetch_optional(pool)
        .await
        .context("db: find author")
}

/// The household's contributor of that name, made if there is none. Marked
/// edited, so the orphan sweep keeps it while no book credits it yet.
pub async fn create_author(
    pool: &PgPool,
    family_id: Uuid,
    name: &str,
    sort_name: Option<&str>,
    bio: Option<&str>,
) -> anyhow::Result<Author> {
    let id: Uuid = sqlx::query_scalar("SELECT music_contributor($1, $2, NULL)")
        .bind(family_id)
        .bind(name.trim())
        .fetch_one(pool)
        .await
        .context("db: create author")?;
    sqlx::query(
        "UPDATE contributors SET sort_name = COALESCE($2, sort_name), biography = COALESCE($3, biography),
                edited_at = now(), updated_at = now()
          WHERE id = $1",
    )
    .bind(id)
    .bind(sort_name)
    .bind(bio)
    .execute(pool)
    .await
    .context("db: describe author")?;
    find_author(pool, family_id, id).await?.context("db: author missing after create")
}

/// A correction on the entity: the name shown everywhere it is credited. The
/// old spelling stays a key, so the next scan of a file still spelled that way
/// finds this contributor rather than making a new one.
pub async fn update_author(
    pool: &PgPool,
    family_id: Uuid,
    id: Uuid,
    name: &str,
    sort_name: Option<&str>,
    bio: Option<&str>,
) -> anyhow::Result<Option<Author>> {
    let mut tx = pool.begin().await.context("db: begin update author")?;
    let updated = sqlx::query(
        "UPDATE contributors SET name = $3, sort_name = $4, biography = $5, edited_at = now(), updated_at = now()
          WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(family_id)
    .bind(name.trim())
    .bind(sort_name)
    .bind(bio)
    .execute(&mut *tx)
    .await
    .context("db: update author")?
    .rows_affected();
    if updated == 0 {
        return Ok(None);
    }
    sqlx::query(
        "INSERT INTO contributor_keys (family_id, key, contributor_id) VALUES ($1, 'name:' || lower(trim($2)), $3)
         ON CONFLICT DO NOTHING",
    )
    .bind(family_id)
    .bind(name)
    .bind(id)
    .execute(&mut *tx)
    .await
    .context("db: key renamed author")?;
    tx.commit().await.context("db: commit update author")?;
    find_author(pool, family_id, id).await
}

/// Removes the contributor and every credit it has — music and books alike.
/// A tag that still names it recreates it at the next scan.
pub async fn delete_author(pool: &PgPool, family_id: Uuid, id: Uuid) -> anyhow::Result<bool> {
    Ok(sqlx::query("DELETE FROM contributors WHERE id = $1 AND family_id = $2")
        .bind(id)
        .bind(family_id)
        .execute(pool)
        .await
        .context("db: delete author")?
        .rows_affected()
        > 0)
}

/// How many books the viewer can see credit this contributor.
pub async fn author_book_count(pool: &PgPool, viewer: Viewer, author_id: Uuid) -> anyhow::Result<i64> {
    let sql = format!(
        "SELECT COUNT(DISTINCT t.id) FROM audiobook_books t
           JOIN credits c ON c.item_kind = 'book' AND c.item_id = t.id
          WHERE {VISIBLE} AND c.contributor_id = $5"
    );
    sqlx::query_scalar(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(AUDIOBOOK)
        .bind(author_id)
        .fetch_one(pool)
        .await
        .context("db: author book count")
}

/// A book's credits, by role and position.
pub async fn list_book_authors(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<(BookAuthor, String)>> {
    let rows: Vec<(Uuid, Uuid, String, String)> = sqlx::query_as(
        "SELECT c.item_id, c.contributor_id, c.role, k.name
           FROM credits c JOIN contributors k ON k.id = c.contributor_id
          WHERE c.item_kind = 'book' AND c.item_id = $1
          ORDER BY c.role, c.position",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list book authors")?;
    Ok(rows
        .into_iter()
        .map(|(book_id, author_id, role, name)| (BookAuthor { book_id, author_id, role }, name))
        .collect())
}

/// The books crediting this contributor (visibility is the caller's to check).
pub async fn list_author_books(pool: &PgPool, author_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT DISTINCT c.item_id FROM credits c WHERE c.item_kind = 'book' AND c.contributor_id = $1",
    )
    .bind(author_id)
    .fetch_all(pool)
    .await
    .context("db: list author books")
}

/// A credit made by hand; kept when the book's text fields change.
pub async fn link_book_author(pool: &PgPool, book_id: Uuid, author_id: Uuid, role: &str) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO credits (contributor_id, item_kind, item_id, role, position, source)
         SELECT $2, 'book', $1, $3,
                COALESCE((SELECT max(position) + 1 FROM credits WHERE item_kind = 'book' AND item_id = $1 AND role = $3), 0),
                'user'
          WHERE NOT EXISTS (SELECT 1 FROM credits WHERE item_kind = 'book' AND item_id = $1 AND role = $3 AND contributor_id = $2)",
    )
    .bind(book_id)
    .bind(author_id)
    .bind(role)
    .execute(pool)
    .await
    .context("db: link book author")?;
    sqlx::query("SELECT music_credit_joins('book', $1)")
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: book credit joins")?;
    Ok(())
}

pub async fn unlink_book_author(pool: &PgPool, book_id: Uuid, author_id: Uuid, role: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM credits WHERE item_kind = 'book' AND item_id = $1 AND contributor_id = $2 AND role = $3")
        .bind(book_id)
        .bind(author_id)
        .bind(role)
        .execute(pool)
        .await
        .context("db: unlink book author")?;
    Ok(())
}

// ── Tags ────────────────────────────────────────────────────────────────────

/// Tags on books the viewer can see; another household's tags stay theirs.
pub async fn list_tags(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<Tag>> {
    let sql = format!(
        "SELECT DISTINCT g.id, g.name FROM audiobook_tags g
           JOIN audiobook_book_tags bt ON bt.tag_id = g.id
           JOIN audiobook_books t ON t.id = bt.book_id
          WHERE {VISIBLE}
          ORDER BY g.name ASC"
    );
    sqlx::query_as::<_, Tag>(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(AUDIOBOOK)
        .fetch_all(pool)
        .await
        .context("db: list tags")
}

pub async fn get_or_create_tag(pool: &PgPool, name: &str) -> anyhow::Result<Tag> {
    let existing = sqlx::query_as::<_, Tag>(
        "SELECT id, name FROM audiobook_tags WHERE lower(name) = lower($1)",
    )
    .bind(name)
    .fetch_optional(pool)
    .await
    .context("db: find tag")?;

    if let Some(t) = existing {
        return Ok(t);
    }

    sqlx::query_as::<_, Tag>(
        "INSERT INTO audiobook_tags (name) VALUES ($1) RETURNING id, name",
    )
    .bind(name)
    .fetch_one(pool)
    .await
    .context("db: create tag")
}

pub async fn list_book_tags(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<Tag>> {
    sqlx::query_as::<_, Tag>(
        "SELECT t.id, t.name FROM audiobook_tags t
         JOIN audiobook_book_tags bt ON bt.tag_id = t.id
         WHERE bt.book_id = $1
         ORDER BY t.name",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: list book tags")
}

pub async fn set_book_tags(pool: &PgPool, book_id: Uuid, tag_ids: &[Uuid]) -> anyhow::Result<()> {
    // Remove existing
    sqlx::query("DELETE FROM audiobook_book_tags WHERE book_id = $1")
        .bind(book_id)
        .execute(pool)
        .await
        .context("db: clear book tags")?;

    // Insert new
    for tag_id in tag_ids {
        sqlx::query("INSERT INTO audiobook_book_tags (book_id, tag_id) VALUES ($1, $2) ON CONFLICT DO NOTHING")
            .bind(book_id)
            .bind(tag_id)
            .execute(pool)
            .await
            .context("db: add book tag")?;
    }
    Ok(())
}
