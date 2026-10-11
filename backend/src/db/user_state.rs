// SPDX-License-Identifier: AGPL-3.0-or-later
//! Each listener's own state, one model over every medium (DESIGN §7.6;
//! docs/api-v2-plan.md, V4): favourites, ratings, progress (with the last
//! position on each device), bookmarks.
//!
//! The storage is still the tables v1 writes, one or two per medium; this
//! module is the single shape over them, so v1 and v2 see the same state. The
//! tables become one when v1 is gone.
use crate::db::access::Viewer;
use crate::db::catalog::{books, credited_visibly, music, podcasts};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

/// What user state can be about.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Track,
    Album,
    Contributor,
    Book,
    Episode,
}

impl Kind {
    pub fn parse(s: &str) -> Option<Kind> {
        Some(match s {
            "track" => Kind::Track,
            "album" => Kind::Album,
            "contributor" => Kind::Contributor,
            "book" => Kind::Book,
            "episode" => Kind::Episode,
            _ => return None,
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Track => "track",
            Kind::Album => "album",
            Kind::Contributor => "contributor",
            Kind::Book => "book",
            Kind::Episode => "episode",
        }
    }

    /// The kinds each piece of state exists for.
    pub fn can_favourite(self) -> bool {
        matches!(self, Kind::Track | Kind::Album | Kind::Contributor | Kind::Book)
    }

    pub fn can_rate(self) -> bool {
        matches!(self, Kind::Track | Kind::Album | Kind::Contributor)
    }

    pub fn has_progress(self) -> bool {
        matches!(self, Kind::Track | Kind::Book | Kind::Episode)
    }
}

macro_rules! viewer {
    ($q:expr, $viewer:expr) => {
        $q.bind($viewer.user_id).bind($viewer.family_id).bind($viewer.is_family_admin)
    };
}

/// A `WHERE` fragment that the item `x` (a column) is visible, by kind.
fn visible(kind: Kind, x: &str) -> String {
    match kind {
        Kind::Track => format!("EXISTS (SELECT 1 FROM music_tracks t WHERE t.id = {x} AND {})", music()),
        Kind::Album => format!("EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = {x} AND {})", music()),
        Kind::Contributor => format!(
            "EXISTS (SELECT 1 FROM contributors k WHERE k.id = {x} AND k.family_id = $2 AND {})",
            credited_visibly("NULL")
        ),
        Kind::Book => format!("EXISTS (SELECT 1 FROM audiobook_books t WHERE t.id = {x} AND {})", books()),
        Kind::Episode => format!(
            "EXISTS (SELECT 1 FROM podcast_episodes e JOIN podcast_feeds t ON t.id = e.feed_id WHERE e.id = {x} AND {})",
            podcasts()
        ),
    }
}

/// Whether the viewer can see the item.
pub async fn is_visible(pool: &PgPool, viewer: Viewer, kind: Kind, id: Uuid) -> anyhow::Result<bool> {
    let sql = format!("SELECT {}", visible(kind, "$4"));
    viewer!(sqlx::query_scalar::<_, bool>(&sql), viewer)
        .bind(id)
        .fetch_one(pool)
        .await
        .context("db: item visible")
}

// ── Favourites ───────────────────────────────────────────────────────────────

/// (table, item column, extra condition) of a favourite kind.
fn favourite_table(kind: Kind) -> (&'static str, &'static str) {
    match kind {
        Kind::Track => ("music_track_stars", "track_id"),
        Kind::Album => ("music_stars", "album_id"),
        Kind::Contributor => ("music_stars", "contributor_id"),
        Kind::Book => ("audiobook_favorites", "book_id"),
        Kind::Episode => unreachable!("episodes are not favourited"),
    }
}

pub async fn set_favourite(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid, on: bool) -> anyhow::Result<()> {
    let (table, column) = favourite_table(kind);
    let sql = if on {
        format!("INSERT INTO {table} (user_id, {column}) VALUES ($1, $2) ON CONFLICT DO NOTHING")
    } else {
        format!("DELETE FROM {table} WHERE user_id = $1 AND {column} = $2")
    };
    sqlx::query(&sql).bind(user_id).bind(id).execute(pool).await.context("db: set favourite")?;
    Ok(())
}

/// The viewer's favourites of a kind that they can still see, newest first.
pub async fn list_favourites(
    pool: &PgPool,
    viewer: Viewer,
    kind: Kind,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<(Uuid, DateTime<Utc>)>> {
    let (table, column) = favourite_table(kind);
    let (at, id) = after.unzip();
    let sql = format!(
        "SELECT s.{column}, s.created_at FROM {table} s
          WHERE s.user_id = $1 AND s.{column} IS NOT NULL AND {}
            AND ($4::timestamptz IS NULL OR (s.created_at, s.{column}) < ($4, $5))
          ORDER BY s.created_at DESC, s.{column} DESC LIMIT $6",
        visible(kind, &format!("s.{column}"))
    );
    viewer!(sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(&sql), viewer)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list favourites")
}

/// When the user made it a favourite, if they did.
pub async fn favourite_at(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid) -> anyhow::Result<Option<DateTime<Utc>>> {
    let (table, column) = favourite_table(kind);
    sqlx::query_scalar(&format!("SELECT created_at FROM {table} WHERE user_id = $1 AND {column} = $2"))
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: favourite at")
}

// ── Ratings ──────────────────────────────────────────────────────────────────

/// 1 to 5, or `None` to take the rating away.
pub async fn set_rating(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid, rating: Option<i16>) -> anyhow::Result<()> {
    let (table, column, target) = match kind {
        Kind::Track => ("music_track_ratings", "track_id", "(user_id, track_id)"),
        Kind::Album => ("music_ratings", "album_id", "(user_id, album_id) WHERE album_id IS NOT NULL"),
        Kind::Contributor => ("music_ratings", "contributor_id", "(user_id, contributor_id) WHERE contributor_id IS NOT NULL"),
        _ => unreachable!("not a rated kind"),
    };
    let sql = match rating {
        Some(_) => format!(
            "INSERT INTO {table} (user_id, {column}, rating, updated_at) VALUES ($1, $2, $3, now())
             ON CONFLICT {target} DO UPDATE SET rating = EXCLUDED.rating, updated_at = now()"
        ),
        None => format!("DELETE FROM {table} WHERE user_id = $1 AND {column} = $2 AND $3::smallint IS NULL"),
    };
    sqlx::query(&sql).bind(user_id).bind(id).bind(rating).execute(pool).await.context("db: set rating")?;
    Ok(())
}

/// The viewer's ratings of a kind, most recently changed first: (item, rating, when).
pub async fn list_ratings(
    pool: &PgPool,
    viewer: Viewer,
    kind: Kind,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<(Uuid, i16, DateTime<Utc>)>> {
    let (table, column) = match kind {
        Kind::Track => ("music_track_ratings", "track_id"),
        Kind::Album => ("music_ratings", "album_id"),
        _ => ("music_ratings", "contributor_id"),
    };
    let (at, id) = after.unzip();
    let sql = format!(
        "SELECT s.{column}, s.rating, s.updated_at FROM {table} s
          WHERE s.user_id = $1 AND s.{column} IS NOT NULL AND {}
            AND ($4::timestamptz IS NULL OR (s.updated_at, s.{column}) < ($4, $5))
          ORDER BY s.updated_at DESC, s.{column} DESC LIMIT $6",
        visible(kind, &format!("s.{column}"))
    );
    viewer!(sqlx::query_as::<_, (Uuid, i16, DateTime<Utc>)>(&sql), viewer)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list ratings")
}

/// The user's rating of it, if any, and when it was set.
pub async fn rating_of(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid) -> anyhow::Result<Option<(i16, DateTime<Utc>)>> {
    let (table, column) = match kind {
        Kind::Track => ("music_track_ratings", "track_id"),
        Kind::Album => ("music_ratings", "album_id"),
        _ => ("music_ratings", "contributor_id"),
    };
    sqlx::query_as(&format!("SELECT rating, updated_at FROM {table} WHERE user_id = $1 AND {column} = $2"))
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: rating of")
}

// ── Progress ─────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct Progress {
    pub item_id: Uuid,
    /// From the start of the item; for a book, across its files.
    pub position_ms: i64,
    pub finished: bool,
    pub updated_at: DateTime<Utc>,
    /// A book's: the file the position is in, and where in it.
    pub file_id: Option<Uuid>,
    pub file_position_ms: Option<i64>,
}

/// A book's files on one timeline: (file, where it starts, its length), in order.
pub async fn book_timeline(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<(Uuid, i64, Option<i64>)>> {
    let files: Vec<(Uuid, Option<i32>)> =
        sqlx::query_as("SELECT id, duration_secs FROM audiobook_files WHERE book_id = $1 ORDER BY position, id")
            .bind(book_id)
            .fetch_all(pool)
            .await
            .context("db: book timeline")?;
    let mut start = 0_i64;
    Ok(files
        .into_iter()
        .map(|(id, secs)| {
            let len = secs.map(|s| i64::from(s) * 1000);
            let entry = (id, start, len);
            start += len.unwrap_or(0);
            entry
        })
        .collect())
}

/// Where on the timeline a whole-book position falls: (file, position in it).
pub fn locate(timeline: &[(Uuid, i64, Option<i64>)], position_ms: i64) -> Option<(Uuid, i64)> {
    let mut found = timeline.first().map(|(id, _, _)| (*id, position_ms))?;
    for (id, start, _) in timeline {
        if *start <= position_ms {
            found = (*id, position_ms - start);
        }
    }
    Some(found)
}

fn start_of(timeline: &[(Uuid, i64, Option<i64>)], file_id: Uuid) -> Option<i64> {
    timeline.iter().find(|(id, _, _)| *id == file_id).map(|(_, start, _)| *start)
}

type Row = (Uuid, f64, bool, DateTime<Utc>, Option<Uuid>);

async fn progress_from(pool: &PgPool, kind: Kind, row: Row) -> anyhow::Result<Progress> {
    let (item_id, secs, finished, updated_at, file_id) = row;
    let file_ms = (secs * 1000.0).round() as i64;
    let position_ms = match (kind, file_id) {
        (Kind::Book, Some(file)) => start_of(&book_timeline(pool, item_id).await?, file).unwrap_or(0) + file_ms,
        _ => file_ms,
    };
    Ok(Progress {
        item_id,
        position_ms,
        finished,
        updated_at,
        file_id,
        file_position_ms: file_id.map(|_| file_ms),
    })
}

fn progress_select(kind: Kind) -> (&'static str, &'static str, &'static str) {
    match kind {
        Kind::Track => ("music_progress", "track_id", "NULL::uuid"),
        Kind::Book => ("audiobook_progress", "book_id", "file_id"),
        Kind::Episode => ("podcast_progress", "episode_id", "NULL::uuid"),
        _ => unreachable!("no progress for this kind"),
    }
}

pub async fn get_progress(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid) -> anyhow::Result<Option<Progress>> {
    let (table, column, file) = progress_select(kind);
    let row: Option<Row> = sqlx::query_as(&format!(
        "SELECT {column}, position_secs, completed, updated_at, {file} FROM {table} WHERE user_id = $1 AND {column} = $2"
    ))
    .bind(user_id)
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("db: get progress")?;
    match row {
        Some(row) => Ok(Some(progress_from(pool, kind, row).await?)),
        None => Ok(None),
    }
}

/// The later change wins (DESIGN §7.6): written only when `at` is not older
/// than what is stored. Returns the state now and whether this one was applied.
#[allow(clippy::too_many_arguments)]
pub async fn put_progress(
    pool: &PgPool,
    user_id: Uuid,
    kind: Kind,
    id: Uuid,
    position_ms: i64,
    finished: bool,
    at: DateTime<Utc>,
    file: Option<(Uuid, i64)>,
) -> anyhow::Result<(Progress, bool)> {
    let (table, column, _) = progress_select(kind);
    let applied = match kind {
        Kind::Book => {
            let (file_id, file_ms) = file.context("a book's progress needs its file")?;
            sqlx::query(
                "INSERT INTO audiobook_progress (user_id, book_id, file_id, position_secs, completed, updated_at)
                 VALUES ($1, $2, $3, $4, $5, $6)
                 ON CONFLICT (user_id, book_id) DO UPDATE
                    SET file_id = EXCLUDED.file_id, position_secs = EXCLUDED.position_secs,
                        completed = EXCLUDED.completed, updated_at = EXCLUDED.updated_at
                  WHERE audiobook_progress.updated_at <= EXCLUDED.updated_at",
            )
            .bind(user_id)
            .bind(id)
            .bind(file_id)
            .bind(file_ms as f64 / 1000.0)
            .bind(finished)
            .bind(at)
            .execute(pool)
            .await
            .context("db: put book progress")?
            .rows_affected()
        }
        _ => sqlx::query(&format!(
            "INSERT INTO {table} (user_id, {column}, position_secs, completed, updated_at) VALUES ($1, $2, $3, $4, $5)
             ON CONFLICT (user_id, {column}) DO UPDATE
                SET position_secs = EXCLUDED.position_secs, completed = EXCLUDED.completed, updated_at = EXCLUDED.updated_at
              WHERE {table}.updated_at <= EXCLUDED.updated_at"
        ))
        .bind(user_id)
        .bind(id)
        .bind(position_ms as f64 / 1000.0)
        .bind(finished)
        .bind(at)
        .execute(pool)
        .await
        .context("db: put progress")?
        .rows_affected(),
    } > 0;
    let now = get_progress(pool, user_id, kind, id).await?.context("progress missing after write")?;
    Ok((now, applied))
}

/// Forget it: the item starts from the beginning everywhere.
pub async fn delete_progress(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid) -> anyhow::Result<()> {
    let (table, column, _) = progress_select(kind);
    sqlx::query(&format!("DELETE FROM {table} WHERE user_id = $1 AND {column} = $2"))
        .bind(user_id)
        .bind(id)
        .execute(pool)
        .await
        .context("db: delete progress")?;
    sqlx::query("DELETE FROM progress_devices WHERE user_id = $1 AND item_kind = $2 AND item_id = $3")
        .bind(user_id)
        .bind(kind.as_str())
        .bind(id)
        .execute(pool)
        .await
        .context("db: delete device progress")?;
    Ok(())
}

/// The viewer's progress on a kind, most recently changed first.
pub async fn list_progress(
    pool: &PgPool,
    viewer: Viewer,
    kind: Kind,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<Progress>> {
    let (table, column, file) = progress_select(kind);
    let (at, id) = after.unzip();
    let sql = format!(
        "SELECT s.{column}, s.position_secs, s.completed, s.updated_at, {} FROM {table} s
          WHERE s.user_id = $1 AND {}
            AND ($4::timestamptz IS NULL OR (s.updated_at, s.{column}) < ($4, $5))
          ORDER BY s.updated_at DESC, s.{column} DESC LIMIT $6",
        if file == "file_id" { "s.file_id" } else { file },
        visible(kind, &format!("s.{column}"))
    );
    let rows: Vec<Row> = viewer!(sqlx::query_as(&sql), viewer)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list progress")?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        out.push(progress_from(pool, kind, row).await?);
    }
    Ok(out)
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DevicePosition {
    pub device_id: Uuid,
    pub device_name: Option<String>,
    pub device_kind: String,
    pub position_ms: i64,
    pub finished: bool,
    pub at: DateTime<Utc>,
}

/// Where this device last was, kept even when an older write lost to a newer one.
#[allow(clippy::too_many_arguments)]
pub async fn put_device_position(
    pool: &PgPool,
    user_id: Uuid,
    kind: Kind,
    id: Uuid,
    device_id: Uuid,
    device_name: Option<&str>,
    device_kind: &str,
    position_ms: i64,
    finished: bool,
    at: DateTime<Utc>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO progress_devices (user_id, item_kind, item_id, device_id, device_name, device_kind, position_ms, finished, at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         ON CONFLICT (user_id, item_kind, item_id, device_id) DO UPDATE
            SET device_name = EXCLUDED.device_name, device_kind = EXCLUDED.device_kind,
                position_ms = EXCLUDED.position_ms, finished = EXCLUDED.finished, at = EXCLUDED.at
          WHERE progress_devices.at <= EXCLUDED.at",
    )
    .bind(user_id)
    .bind(kind.as_str())
    .bind(id)
    .bind(device_id)
    .bind(device_name)
    .bind(device_kind)
    .bind(position_ms)
    .bind(finished)
    .bind(at)
    .execute(pool)
    .await
    .context("db: put device position")?;
    Ok(())
}

pub async fn device_positions(pool: &PgPool, user_id: Uuid, kind: Kind, id: Uuid) -> anyhow::Result<Vec<DevicePosition>> {
    sqlx::query_as(
        "SELECT device_id, device_name, device_kind, position_ms, finished, at FROM progress_devices
          WHERE user_id = $1 AND item_kind = $2 AND item_id = $3 ORDER BY at DESC",
    )
    .bind(user_id)
    .bind(kind.as_str())
    .bind(id)
    .fetch_all(pool)
    .await
    .context("db: device positions")
}

// ── Bookmarks ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BookmarkRow {
    pub id: Uuid,
    pub track_id: Option<Uuid>,
    pub book_id: Option<Uuid>,
    pub episode_id: Option<Uuid>,
    pub file_id: Option<Uuid>,
    pub position_secs: f64,
    pub label: Option<String>,
    pub has_audio_note: bool,
    pub created_at: DateTime<Utc>,
}

const BOOKMARK_COLS: &str =
    "b.id, b.track_id, b.book_id, b.episode_id, b.file_id, b.position_secs, b.label, b.audio_object_id IS NOT NULL AS has_audio_note, b.created_at";

fn bookmark_column(kind: Kind) -> &'static str {
    match kind {
        Kind::Track => "track_id",
        Kind::Book => "book_id",
        Kind::Episode => "episode_id",
        _ => unreachable!("not a bookmarked kind"),
    }
}

/// The viewer's bookmarks on one item, or on every item of a kind, newest first.
pub async fn list_bookmarks(
    pool: &PgPool,
    viewer: Viewer,
    kind: Kind,
    item: Option<Uuid>,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<BookmarkRow>> {
    let column = bookmark_column(kind);
    let (at, id) = after.unzip();
    let sql = format!(
        "SELECT {BOOKMARK_COLS} FROM bookmarks b
          WHERE b.user_id = $1 AND b.{column} IS NOT NULL AND ($4::uuid IS NULL OR b.{column} = $4) AND {}
            AND ($5::timestamptz IS NULL OR (b.created_at, b.id) < ($5, $6))
          ORDER BY b.created_at DESC, b.id DESC LIMIT $7",
        visible(kind, &format!("b.{column}"))
    );
    viewer!(sqlx::query_as::<_, BookmarkRow>(&sql), viewer)
        .bind(item)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list bookmarks")
}

pub async fn create_bookmark(
    pool: &PgPool,
    user_id: Uuid,
    kind: Kind,
    item: Uuid,
    file_id: Option<Uuid>,
    position_secs: f64,
    label: Option<&str>,
) -> anyhow::Result<BookmarkRow> {
    let column = bookmark_column(kind);
    sqlx::query_as::<_, BookmarkRow>(&format!(
        "INSERT INTO bookmarks AS b (user_id, {column}, file_id, position_secs, label) VALUES ($1, $2, $3, $4, $5)
         RETURNING {BOOKMARK_COLS}"
    ))
    .bind(user_id)
    .bind(item)
    .bind(file_id)
    .bind(position_secs)
    .bind(label)
    .fetch_one(pool)
    .await
    .context("db: create bookmark")
}

pub async fn find_bookmark(pool: &PgPool, user_id: Uuid, id: Uuid) -> anyhow::Result<Option<BookmarkRow>> {
    sqlx::query_as::<_, BookmarkRow>(&format!("SELECT {BOOKMARK_COLS} FROM bookmarks b WHERE b.id = $1 AND b.user_id = $2"))
        .bind(id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .context("db: find bookmark")
}

/// Label and place; `None` leaves either as it is.
pub async fn update_bookmark(
    pool: &PgPool,
    user_id: Uuid,
    id: Uuid,
    label: Option<Option<&str>>,
    position: Option<(Option<Uuid>, f64)>,
) -> anyhow::Result<Option<BookmarkRow>> {
    sqlx::query_as::<_, BookmarkRow>(&format!(
        "UPDATE bookmarks b SET
            label = CASE WHEN $3 THEN $4 ELSE b.label END,
            file_id = CASE WHEN $5 THEN $6 ELSE b.file_id END,
            position_secs = COALESCE($7, b.position_secs)
          WHERE b.id = $1 AND b.user_id = $2
         RETURNING {BOOKMARK_COLS}"
    ))
    .bind(id)
    .bind(user_id)
    .bind(label.is_some())
    .bind(label.flatten())
    .bind(position.is_some())
    .bind(position.and_then(|(f, _)| f))
    .bind(position.map(|(_, p)| p))
    .fetch_optional(pool)
    .await
    .context("db: update bookmark")
}

pub async fn delete_bookmark(pool: &PgPool, user_id: Uuid, id: Uuid) -> anyhow::Result<bool> {
    Ok(sqlx::query("DELETE FROM bookmarks WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete bookmark")?
        .rows_affected()
        == 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_book_position_falls_in_its_file() {
        let (a, b, c) = (Uuid::new_v4(), Uuid::new_v4(), Uuid::new_v4());
        let timeline = vec![(a, 0, Some(60_000)), (b, 60_000, Some(30_000)), (c, 90_000, None)];
        assert_eq!(locate(&timeline, 0), Some((a, 0)));
        assert_eq!(locate(&timeline, 59_999), Some((a, 59_999)));
        assert_eq!(locate(&timeline, 60_000), Some((b, 0)));
        assert_eq!(locate(&timeline, 100_000), Some((c, 10_000)));
        assert_eq!(locate(&[], 5), None);
    }
}
