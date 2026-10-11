// SPDX-License-Identifier: AGPL-3.0-or-later
//! The music catalogue as `/api/v2` reads it (docs/api-v2-plan.md, V3): the
//! entities of migration 0100, paged by keyset in the orders of migration
//! 0106, each row kept only when the viewer can see an item behind it.
//!
//! Every query binds the viewer as `$1` user, `$2` family, `$3` family admin
//! (`db::access::VISIBLE` with the media kind written in), then its own
//! parameters from `$4`.
use crate::db::access::{Viewer, visible_for};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

macro_rules! viewer {
    ($q:expr, $viewer:expr) => {
        $q.bind($viewer.user_id).bind($viewer.family_id).bind($viewer.is_family_admin)
    };
}

pub(crate) fn music() -> String {
    visible_for("'music'")
}

/// A contributor credited (in one of `roles`, or any role when `roles` is
/// NULL — bound at `$roles`) on something the viewer can see: a track, an album
/// with a visible track, a book, a podcast.
pub(crate) fn credited_visibly(roles: &str) -> String {
    let music = music();
    let book = visible_for("'audiobook'");
    let podcast = visible_for("'podcast'");
    format!(
        "EXISTS (SELECT 1 FROM credits c
                  WHERE c.contributor_id = k.id AND ({roles}::text[] IS NULL OR c.role = ANY({roles}::text[]))
                    AND ((c.item_kind = 'track' AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.id = c.item_id AND {music}))
                      OR (c.item_kind = 'album' AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = c.item_id AND {music}))
                      OR (c.item_kind = 'book' AND EXISTS (SELECT 1 FROM audiobook_books t WHERE t.id = c.item_id AND {book}))
                      OR (c.item_kind = 'podcast' AND EXISTS (SELECT 1 FROM podcast_feeds t WHERE t.id = c.item_id AND {podcast}))))"
    )
}

// ── Contributors ─────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ContributorRow {
    pub id: Uuid,
    pub name: String,
    pub sort_name: Option<String>,
    pub kind: String,
    pub musicbrainz_artist_id: Option<String>,
    pub image_object_id: Option<Uuid>,
    /// The list's order: `lower(sort name or name)`.
    pub sort_key: String,
}

const CONTRIBUTOR_COLS: &str = "k.id, k.name, k.sort_name, k.kind, k.musicbrainz_artist_id,
    k.image_object_id, lower(COALESCE(k.sort_name, k.name)) AS sort_key";

/// Contributors in name order, after `after` (sort key, id).
pub async fn list_contributors(
    pool: &PgPool,
    viewer: Viewer,
    roles: Option<&[String]>,
    query: Option<&str>,
    after: Option<(String, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<ContributorRow>> {
    let (after_key, after_id) = after.unzip();
    let sql = format!(
        "SELECT {CONTRIBUTOR_COLS} FROM contributors k
          WHERE k.family_id = $2
            AND ($6::text IS NULL OR (lower(COALESCE(k.sort_name, k.name)), k.id) > ($6, $7))
            AND ($5::text IS NULL OR k.name ILIKE '%' || $5 || '%')
            AND {}
          ORDER BY lower(COALESCE(k.sort_name, k.name)), k.id
          LIMIT $8",
        credited_visibly("$4")
    );
    viewer!(sqlx::query_as::<_, ContributorRow>(&sql), viewer)
        .bind(roles)
        .bind(query.map(escape_like))
        .bind(after_key)
        .bind(after_id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list contributors")
}

/// One contributor the viewer can see something of.
pub async fn find_contributor(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<ContributorRow>> {
    let sql = format!(
        "SELECT {CONTRIBUTOR_COLS} FROM contributors k WHERE k.id = $5 AND k.family_id = $2 AND {}",
        credited_visibly("$4")
    );
    viewer!(sqlx::query_as::<_, ContributorRow>(&sql), viewer)
        .bind(None::<Vec<String>>)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find contributor")
}

/// The roles a contributor has on what the viewer can see.
pub async fn contributor_roles(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Vec<String>> {
    let mut roles = Vec::new();
    for role in sqlx::query_scalar::<_, String>("SELECT DISTINCT role FROM credits WHERE contributor_id = $1 ORDER BY role")
        .bind(id)
        .fetch_all(pool)
        .await
        .context("db: contributor roles")?
    {
        let sql = format!("SELECT {CONTRIBUTOR_COLS} FROM contributors k WHERE k.id = $5 AND {}", credited_visibly("$4"));
        let seen = viewer!(sqlx::query_as::<_, ContributorRow>(&sql), viewer)
            .bind(vec![role.clone()])
            .bind(id)
            .fetch_optional(pool)
            .await
            .context("db: contributor role visible")?
            .is_some();
        if seen {
            roles.push(role);
        }
    }
    Ok(roles)
}

/// Contributors by id, for `include`: the household's, whatever is credited.
/// Callers pass ids from records the viewer already sees.
pub async fn contributors_by_ids(pool: &PgPool, family_id: Uuid, ids: &[Uuid]) -> anyhow::Result<Vec<ContributorRow>> {
    sqlx::query_as::<_, ContributorRow>(&format!(
        "SELECT {CONTRIBUTOR_COLS} FROM contributors k WHERE k.family_id = $1 AND k.id = ANY($2)
          ORDER BY lower(COALESCE(k.sort_name, k.name)), k.id"
    ))
    .bind(family_id)
    .bind(ids)
    .fetch_all(pool)
    .await
    .context("db: contributors by ids")
}

// ── Credits ──────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct CreditRow {
    pub item_id: Uuid,
    pub contributor_id: Uuid,
    pub role: String,
    pub position: i16,
    pub credited_as: Option<String>,
    pub join_phrase: String,
}

/// The order roles are listed in. Positions count within a role (a book's
/// first narrator is 0, like its first author), so the role comes first.
const ROLE_ORDER: &[&str] = &[
    "artist", "featured", "composer", "lyricist", "producer", "conductor", "performer", "remixer",
    "author", "co_author", "translator", "narrator", "editor", "illustrator", "host", "co_host", "guest",
];

/// The credits of several items of one kind, in order.
pub async fn credits(pool: &PgPool, kind: &str, ids: &[Uuid]) -> anyhow::Result<Vec<CreditRow>> {
    sqlx::query_as::<_, CreditRow>(
        "SELECT item_id, contributor_id, role, position, credited_as, join_phrase FROM credits
          WHERE item_kind = $1 AND item_id = ANY($2)
          ORDER BY item_id, array_position($3::text[], role) NULLS LAST, role, position",
    )
    .bind(kind)
    .bind(ids)
    .bind(ROLE_ORDER)
    .fetch_all(pool)
    .await
    .context("db: credits")
}

// ── Albums ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AlbumRow {
    pub id: Uuid,
    pub title: String,
    pub release_group_id: Option<Uuid>,
    pub release_date: Option<String>,
    pub country: Option<String>,
    pub label: Option<String>,
    pub barcode: Option<String>,
    pub is_compilation: bool,
    pub musicbrainz_release_id: Option<String>,
    pub created_at: DateTime<Utc>,
    /// `lower(title)` as the database orders it, for the cursor.
    pub sort_key: String,
}

const ALBUM_COLS: &str = "a.id, a.title, a.release_group_id, a.release_date, a.country, a.label, a.barcode,
    a.is_compilation, a.musicbrainz_release_id, a.created_at, lower(a.title) AS sort_key";

#[derive(Debug, Default)]
pub struct AlbumFilter {
    pub contributor_id: Option<Uuid>,
    pub release_group_id: Option<Uuid>,
    pub genre_id: Option<Uuid>,
    pub query: Option<String>,
}

/// Where a page of albums starts: after this title, or before this time (newest first).
pub enum AlbumAfter {
    Title(String, Uuid),
    Added(DateTime<Utc>, Uuid),
}

pub async fn list_albums(
    pool: &PgPool,
    viewer: Viewer,
    filter: &AlbumFilter,
    by_added: bool,
    after: Option<AlbumAfter>,
    limit: i64,
) -> anyhow::Result<Vec<AlbumRow>> {
    let music = music();
    let (order, keyset) = if by_added {
        ("a.created_at DESC, a.id DESC", "($9::timestamptz IS NULL OR (a.created_at, a.id) < ($9, $10))")
    } else {
        ("lower(a.title), a.id", "($8::text IS NULL OR (lower(a.title), a.id) > ($8, $10))")
    };
    let sql = format!(
        "SELECT {ALBUM_COLS} FROM albums a
          WHERE a.family_id = $2 AND {keyset}
            AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'album' AND c.item_id = a.id AND c.contributor_id = $4)
                 OR EXISTS (SELECT 1 FROM music_tracks t JOIN credits c ON c.item_kind = 'track' AND c.item_id = t.id
                             WHERE t.album_id = a.id AND c.contributor_id = $4 AND {music}))
            AND ($5::uuid IS NULL OR a.release_group_id = $5)
            AND ($7::text IS NULL OR a.title ILIKE '%' || $7 || '%')
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = a.id AND {music}
                          AND ($6::uuid IS NULL OR EXISTS (SELECT 1 FROM track_genres g WHERE g.track_id = t.id AND g.genre_id = $6)))
          ORDER BY {order}
          LIMIT $11"
    );
    let (title, added, id) = match after {
        None => (None, None, None),
        Some(AlbumAfter::Title(t, id)) => (Some(t), None, Some(id)),
        Some(AlbumAfter::Added(at, id)) => (None, Some(at), Some(id)),
    };
    viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(filter.contributor_id)
        .bind(filter.release_group_id)
        .bind(filter.genre_id)
        .bind(filter.query.as_deref().map(escape_like))
        .bind(title)
        .bind(added)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list albums")
}

pub async fn find_album(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<AlbumRow>> {
    let sql = format!(
        "SELECT {ALBUM_COLS} FROM albums a WHERE a.id = $4 AND a.family_id = $2
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = a.id AND {})",
        music()
    );
    viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find album")
}

pub async fn albums_by_ids(pool: &PgPool, family_id: Uuid, ids: &[Uuid]) -> anyhow::Result<Vec<AlbumRow>> {
    sqlx::query_as::<_, AlbumRow>(&format!(
        "SELECT {ALBUM_COLS} FROM albums a WHERE a.family_id = $1 AND a.id = ANY($2) ORDER BY lower(a.title), a.id"
    ))
    .bind(family_id)
    .bind(ids)
    .fetch_all(pool)
    .await
    .context("db: albums by ids")
}

/// Per album, what the viewer sees of it: tracks, total seconds, discs, and
/// the cover of its first track that has one.
pub async fn album_stats(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, i64, i64, i32, Option<Uuid>)>> {
    let sql = format!(
        "SELECT t.album_id, count(*), COALESCE(sum(t.duration_secs), 0)::bigint, COALESCE(max(t.disc_number), 1),
                (array_agg(t.cover_object_id ORDER BY COALESCE(t.disc_number, 1), COALESCE(t.track_number, 0), t.id)
                   FILTER (WHERE t.cover_object_id IS NOT NULL))[1]
           FROM music_tracks t WHERE t.album_id = ANY($4) AND {}
          GROUP BY t.album_id",
        music()
    );
    viewer!(sqlx::query_as::<_, (Uuid, i64, i64, i32, Option<Uuid>)>(&sql), viewer)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: album stats")
}

// ── Release groups, recordings, genres ───────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ReleaseGroupRow {
    pub id: Uuid,
    pub title: String,
    pub kind: String,
    pub first_release_date: Option<String>,
    pub musicbrainz_release_group_id: Option<String>,
}

/// A release group with an album the viewer can see, and those albums' ids.
pub async fn find_release_group(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<(ReleaseGroupRow, Vec<Uuid>)>> {
    let Some(group) = sqlx::query_as::<_, ReleaseGroupRow>(
        "SELECT id, title, kind, first_release_date, musicbrainz_release_group_id FROM release_groups
          WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(viewer.family_id)
    .fetch_optional(pool)
    .await
    .context("db: find release group")?
    else {
        return Ok(None);
    };
    let sql = format!(
        "SELECT a.id FROM albums a WHERE a.release_group_id = $4
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = a.id AND {})
          ORDER BY a.release_date NULLS LAST, a.created_at, a.id",
        music()
    );
    let albums: Vec<Uuid> = viewer!(sqlx::query_scalar(&sql), viewer)
        .bind(id)
        .fetch_all(pool)
        .await
        .context("db: release group albums")?;
    Ok((!albums.is_empty()).then_some((group, albums)))
}

pub async fn release_groups_by_ids(pool: &PgPool, family_id: Uuid, ids: &[Uuid]) -> anyhow::Result<Vec<ReleaseGroupRow>> {
    sqlx::query_as::<_, ReleaseGroupRow>(
        "SELECT id, title, kind, first_release_date, musicbrainz_release_group_id FROM release_groups
          WHERE family_id = $1 AND id = ANY($2) ORDER BY lower(title), id",
    )
    .bind(family_id)
    .bind(ids)
    .fetch_all(pool)
    .await
    .context("db: release groups by ids")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct RecordingRow {
    pub id: Uuid,
    pub title: String,
    pub musicbrainz_recording_id: String,
    pub isrc: Option<String>,
}

/// A recording with a track the viewer can see, and those tracks' ids.
pub async fn find_recording(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<(RecordingRow, Vec<Uuid>)>> {
    let Some(recording) = sqlx::query_as::<_, RecordingRow>(
        "SELECT id, title, musicbrainz_recording_id, isrc FROM recordings WHERE id = $1 AND family_id = $2",
    )
    .bind(id)
    .bind(viewer.family_id)
    .fetch_optional(pool)
    .await
    .context("db: find recording")?
    else {
        return Ok(None);
    };
    let sql = format!("SELECT t.id FROM music_tracks t WHERE t.recording_id = $4 AND {} ORDER BY t.created_at, t.id", music());
    let tracks: Vec<Uuid> = viewer!(sqlx::query_scalar(&sql), viewer)
        .bind(id)
        .fetch_all(pool)
        .await
        .context("db: recording tracks")?;
    Ok((!tracks.is_empty()).then_some((recording, tracks)))
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct GenreRow {
    pub id: Uuid,
    pub name: String,
    pub sort_key: String,
}

/// Genres of tracks the viewer can see, by name.
pub async fn list_genres(pool: &PgPool, viewer: Viewer, after: Option<(String, Uuid)>, limit: i64) -> anyhow::Result<Vec<GenreRow>> {
    let (after_key, after_id) = after.unzip();
    let sql = format!(
        "SELECT g.id, g.name, lower(g.name) AS sort_key FROM genres g
          WHERE g.family_id = $2 AND ($4::text IS NULL OR (lower(g.name), g.id) > ($4, $5))
            AND EXISTS (SELECT 1 FROM track_genres tg JOIN music_tracks t ON t.id = tg.track_id
                         WHERE tg.genre_id = g.id AND {})
          ORDER BY lower(g.name), g.id LIMIT $6",
        music()
    );
    viewer!(sqlx::query_as::<_, GenreRow>(&sql), viewer)
        .bind(after_key)
        .bind(after_id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list genres")
}

pub async fn genres_by_ids(pool: &PgPool, family_id: Uuid, ids: &[Uuid]) -> anyhow::Result<Vec<GenreRow>> {
    sqlx::query_as::<_, GenreRow>(
        "SELECT id, name, lower(name) AS sort_key FROM genres WHERE family_id = $1 AND id = ANY($2) ORDER BY lower(name), id",
    )
    .bind(family_id)
    .bind(ids)
    .fetch_all(pool)
    .await
    .context("db: genres by ids")
}

/// Each track's genres, in the order the tag gave them.
pub async fn track_genres(pool: &PgPool, ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    sqlx::query_as("SELECT track_id, genre_id FROM track_genres WHERE track_id = ANY($1) ORDER BY track_id, position")
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: track genres")
}

// ── Tracks ───────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrackRow {
    pub id: Uuid,
    pub title: String,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub album_id: Option<Uuid>,
    pub recording_id: Option<Uuid>,
    pub disc_number: Option<i32>,
    pub track_number: Option<i32>,
    pub duration_secs: Option<i32>,
    pub cover_object_id: Option<Uuid>,
    pub source: String,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// `lower(title)` as the database orders it, for the cursor.
    pub sort_key: String,
}

const TRACK_COLS: &str = "t.id, t.title, t.user_id, t.family_id, t.album_id, t.recording_id, t.disc_number,
    t.track_number, t.duration_secs, t.cover_object_id, t.source,
    (SELECT o.size_bytes FROM media_objects o WHERE o.id = t.audio_object_id) AS size_bytes,
    (SELECT o.content_type FROM media_objects o WHERE o.id = t.audio_object_id) AS content_type,
    t.created_at, t.updated_at, lower(t.title) AS sort_key";

#[derive(Debug, Default)]
pub struct TrackFilter {
    pub album_id: Option<Uuid>,
    pub contributor_id: Option<Uuid>,
    pub genre_id: Option<Uuid>,
    pub recording_id: Option<Uuid>,
    pub query: Option<String>,
}

/// Where a page of tracks starts.
pub enum TrackAfter {
    Title(String, Uuid),
    Added(DateTime<Utc>, Uuid),
    /// Disc, number and title within an album.
    InAlbum(i32, i32, String, Uuid),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TrackOrder {
    Title,
    Added,
    InAlbum,
}

pub async fn list_tracks(
    pool: &PgPool,
    viewer: Viewer,
    filter: &TrackFilter,
    order: TrackOrder,
    after: Option<TrackAfter>,
    limit: i64,
) -> anyhow::Result<Vec<TrackRow>> {
    let (order_sql, keyset) = match order {
        TrackOrder::Title => ("lower(t.title), t.id", "($9::text IS NULL OR (lower(t.title), t.id) > ($9, $14))"),
        TrackOrder::Added => ("t.created_at DESC, t.id DESC", "($10::timestamptz IS NULL OR (t.created_at, t.id) < ($10, $14))"),
        TrackOrder::InAlbum => (
            "COALESCE(t.disc_number, 1), COALESCE(t.track_number, 0), lower(t.title), t.id",
            "($11::int IS NULL OR (COALESCE(t.disc_number, 1), COALESCE(t.track_number, 0), lower(t.title), t.id) > ($11, $12, $13, $14))",
        ),
    };
    let sql = format!(
        "SELECT {TRACK_COLS} FROM music_tracks t
          WHERE {} AND {keyset}
            AND ($4::uuid IS NULL OR t.album_id = $4)
            AND ($5::uuid IS NULL OR EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'track' AND c.item_id = t.id AND c.contributor_id = $5))
            AND ($6::uuid IS NULL OR EXISTS (SELECT 1 FROM track_genres g WHERE g.track_id = t.id AND g.genre_id = $6))
            AND ($7::uuid IS NULL OR t.recording_id = $7)
            AND ($8::text IS NULL OR t.title ILIKE '%' || $8 || '%')
          ORDER BY {order_sql}
          LIMIT $15",
        music()
    );
    let (title, added, disc, number, in_album_title, id) = match after {
        None => (None, None, None, None, None, None),
        Some(TrackAfter::Title(t, id)) => (Some(t), None, None, None, None, Some(id)),
        Some(TrackAfter::Added(at, id)) => (None, Some(at), None, None, None, Some(id)),
        Some(TrackAfter::InAlbum(d, n, t, id)) => (None, None, Some(d), Some(n), Some(t), Some(id)),
    };
    viewer!(sqlx::query_as::<_, TrackRow>(&sql), viewer)
        .bind(filter.album_id)
        .bind(filter.contributor_id)
        .bind(filter.genre_id)
        .bind(filter.recording_id)
        .bind(filter.query.as_deref().map(escape_like))
        .bind(title)
        .bind(added)
        .bind(disc)
        .bind(number)
        .bind(in_album_title)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list tracks")
}

pub async fn find_track(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<TrackRow>> {
    let sql = format!("SELECT {TRACK_COLS} FROM music_tracks t WHERE t.id = $4 AND {}", music());
    viewer!(sqlx::query_as::<_, TrackRow>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find track")
}

/// `%` and `_` typed into a search box are letters, not wildcards.
fn escape_like(text: &str) -> String {
    text.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_")
}

// ── Books ────────────────────────────────────────────────────────────────────

pub(crate) fn books() -> String {
    visible_for("'audiobook'")
}

pub(crate) fn podcasts() -> String {
    visible_for("'podcast'")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BookRow {
    pub id: Uuid,
    pub title: String,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub description: Option<String>,
    pub total_duration_secs: Option<i32>,
    pub isbn: Option<String>,
    pub publisher: Option<String>,
    pub published_year: Option<i32>,
    pub cover_object_id: Option<Uuid>,
    pub source: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sort_key: String,
}

const BOOK_COLS: &str = "t.id, t.title, t.user_id, t.family_id, t.description, t.total_duration_secs, t.isbn,
    t.publisher, t.published_year, t.cover_object_id, t.source, t.created_at,
    t.updated_at, lower(t.title) AS sort_key";

/// Where a page of books or podcasts starts: after this title, or before this time.
pub enum TitleOrAdded {
    Title(String, Uuid),
    Added(DateTime<Utc>, Uuid),
}

fn title_or_added(after: Option<TitleOrAdded>) -> (Option<String>, Option<DateTime<Utc>>, Option<Uuid>) {
    match after {
        None => (None, None, None),
        Some(TitleOrAdded::Title(t, id)) => (Some(t), None, Some(id)),
        Some(TitleOrAdded::Added(at, id)) => (None, Some(at), Some(id)),
    }
}

fn title_or_added_sql(by_added: bool, title: &str, added: &str, id: &str) -> (&'static str, String) {
    if by_added {
        ("t.created_at DESC, t.id DESC", format!("({added}::timestamptz IS NULL OR (t.created_at, t.id) < ({added}, {id}))"))
    } else {
        ("lower(t.title), t.id", format!("({title}::text IS NULL OR (lower(t.title), t.id) > ({title}, {id}))"))
    }
}

pub async fn list_books(
    pool: &PgPool,
    viewer: Viewer,
    contributor_id: Option<Uuid>,
    query: Option<&str>,
    by_added: bool,
    after: Option<TitleOrAdded>,
    limit: i64,
) -> anyhow::Result<Vec<BookRow>> {
    let (order, keyset) = title_or_added_sql(by_added, "$6", "$7", "$8");
    let sql = format!(
        "SELECT {BOOK_COLS} FROM audiobook_books t
          WHERE {} AND {keyset}
            AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'book' AND c.item_id = t.id AND c.contributor_id = $4))
            AND ($5::text IS NULL OR t.title ILIKE '%' || $5 || '%')
          ORDER BY {order} LIMIT $9",
        books()
    );
    let (title, added, id) = title_or_added(after);
    viewer!(sqlx::query_as::<_, BookRow>(&sql), viewer)
        .bind(contributor_id)
        .bind(query.map(escape_like))
        .bind(title)
        .bind(added)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list books")
}

pub async fn find_book(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<BookRow>> {
    let sql = format!("SELECT {BOOK_COLS} FROM audiobook_books t WHERE t.id = $4 AND {}", books());
    viewer!(sqlx::query_as::<_, BookRow>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find book")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct BookFileRow {
    pub id: Uuid,
    pub position: i32,
    pub title: Option<String>,
    pub duration_secs: Option<i32>,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
}

pub async fn book_files(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<BookFileRow>> {
    sqlx::query_as::<_, BookFileRow>(
        "SELECT f.id, f.position, f.title, f.duration_secs, m.size_bytes, m.content_type
           FROM audiobook_files f LEFT JOIN media_objects m ON m.id = f.audio_object_id
          WHERE f.book_id = $1 ORDER BY f.position, f.id",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: book files")
}

/// The series a book belongs to: (series id, name, position).
pub async fn book_series(pool: &PgPool, book_id: Uuid) -> anyhow::Result<Vec<(Uuid, String, f64)>> {
    sqlx::query_as(
        "SELECT s.id, s.name, sb.position FROM audiobook_series_books sb JOIN audiobook_series s ON s.id = sb.series_id
          WHERE sb.book_id = $1 ORDER BY s.name, s.id",
    )
    .bind(book_id)
    .fetch_all(pool)
    .await
    .context("db: book series")
}

// ── Podcasts and episodes ────────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PodcastRow {
    pub id: Uuid,
    pub title: String,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub feed_url: String,
    pub description: Option<String>,
    pub link: Option<String>,
    pub language: Option<String>,
    pub podcast_guid: Option<String>,
    pub categories: Vec<String>,
    pub image_object_id: Option<Uuid>,
    pub image_url: Option<String>,
    pub last_refreshed_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sort_key: String,
}

const PODCAST_COLS: &str = "t.id, t.title, t.user_id, t.family_id, t.feed_url, t.description, t.link, t.language,
    t.podcast_guid, t.categories, t.image_object_id, t.image_url,
    t.last_refreshed_at, t.created_at, t.updated_at, lower(t.title) AS sort_key";

pub async fn list_podcasts(
    pool: &PgPool,
    viewer: Viewer,
    contributor_id: Option<Uuid>,
    query: Option<&str>,
    by_added: bool,
    after: Option<TitleOrAdded>,
    limit: i64,
) -> anyhow::Result<Vec<PodcastRow>> {
    let (order, keyset) = title_or_added_sql(by_added, "$6", "$7", "$8");
    let sql = format!(
        "SELECT {PODCAST_COLS} FROM podcast_feeds t
          WHERE {} AND {keyset}
            AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'podcast' AND c.item_id = t.id AND c.contributor_id = $4))
            AND ($5::text IS NULL OR t.title ILIKE '%' || $5 || '%')
          ORDER BY {order} LIMIT $9",
        podcasts()
    );
    let (title, added, id) = title_or_added(after);
    viewer!(sqlx::query_as::<_, PodcastRow>(&sql), viewer)
        .bind(contributor_id)
        .bind(query.map(escape_like))
        .bind(title)
        .bind(added)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list podcasts")
}

pub async fn find_podcast(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<PodcastRow>> {
    let sql = format!("SELECT {PODCAST_COLS} FROM podcast_feeds t WHERE t.id = $4 AND {}", podcasts());
    viewer!(sqlx::query_as::<_, PodcastRow>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find podcast")
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct EpisodeRow {
    pub id: Uuid,
    pub podcast_id: Uuid,
    pub guid: String,
    pub title: String,
    pub description: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub duration_secs: Option<i32>,
    pub episode_number: Option<i32>,
    pub season_number: Option<i32>,
    pub audio_url: Option<String>,
    pub stored: bool,
    pub size_bytes: Option<i64>,
    pub content_type: Option<String>,
    pub image_object_id: Option<Uuid>,
    pub image_url: Option<String>,
    pub transcript_url: Option<String>,
    pub transcript_type: Option<String>,
    pub created_at: DateTime<Utc>,
    /// The order: published, or added when the feed gives no date.
    pub sort_at: DateTime<Utc>,
}

const EPISODE_COLS: &str = "e.id, e.feed_id AS podcast_id, e.guid, e.title, e.description, e.published_at,
    e.duration_secs, e.episode_number, e.season_number, e.audio_url, e.audio_object_id IS NOT NULL AS stored,
    m.size_bytes, m.content_type, e.image_object_id, e.image_url,
    e.transcript_url, e.transcript_type, e.created_at, COALESCE(e.published_at, e.created_at) AS sort_at";

/// A podcast's episodes, newest first.
pub async fn list_episodes(
    pool: &PgPool,
    viewer: Viewer,
    podcast_id: Uuid,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<EpisodeRow>> {
    let (at, id) = after.unzip();
    let sql = format!(
        "SELECT {EPISODE_COLS} FROM podcast_episodes e LEFT JOIN media_objects m ON m.id = e.audio_object_id
          WHERE e.feed_id = $4 AND EXISTS (SELECT 1 FROM podcast_feeds t WHERE t.id = e.feed_id AND {})
            AND ($5::timestamptz IS NULL OR (COALESCE(e.published_at, e.created_at), e.id) < ($5, $6))
          ORDER BY COALESCE(e.published_at, e.created_at) DESC, e.id DESC LIMIT $7",
        podcasts()
    );
    viewer!(sqlx::query_as::<_, EpisodeRow>(&sql), viewer)
        .bind(podcast_id)
        .bind(at)
        .bind(id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list episodes")
}

pub async fn find_episode(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<EpisodeRow>> {
    let sql = format!(
        "SELECT {EPISODE_COLS} FROM podcast_episodes e LEFT JOIN media_objects m ON m.id = e.audio_object_id
          WHERE e.id = $4 AND EXISTS (SELECT 1 FROM podcast_feeds t WHERE t.id = e.feed_id AND {})",
        podcasts()
    );
    viewer!(sqlx::query_as::<_, EpisodeRow>(&sql), viewer)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find episode")
}

// ── By id, for the change feed and the snapshot ──────────────────────────────

pub async fn tracks_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<TrackRow>> {
    let sql = format!("SELECT {TRACK_COLS} FROM music_tracks t WHERE t.id = ANY($4) AND {} ORDER BY t.id", music());
    viewer!(sqlx::query_as::<_, TrackRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: tracks by ids")
}

pub async fn visible_albums_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<AlbumRow>> {
    let sql = format!(
        "SELECT {ALBUM_COLS} FROM albums a WHERE a.id = ANY($4) AND a.family_id = $2
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = a.id AND {}) ORDER BY a.id",
        music()
    );
    viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: visible albums by ids")
}

pub async fn visible_contributors_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<ContributorRow>> {
    let sql = format!(
        "SELECT {CONTRIBUTOR_COLS} FROM contributors k WHERE k.id = ANY($4) AND k.family_id = $2 AND {} ORDER BY k.id",
        credited_visibly("NULL")
    );
    viewer!(sqlx::query_as::<_, ContributorRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: visible contributors by ids")
}

pub async fn visible_release_groups_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<ReleaseGroupRow>> {
    let sql = format!(
        "SELECT g.id, g.title, g.kind, g.first_release_date, g.musicbrainz_release_group_id FROM release_groups g
          WHERE g.id = ANY($4) AND g.family_id = $2
            AND EXISTS (SELECT 1 FROM albums a JOIN music_tracks t ON t.album_id = a.id WHERE a.release_group_id = g.id AND {})
          ORDER BY g.id",
        music()
    );
    viewer!(sqlx::query_as::<_, ReleaseGroupRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: visible release groups by ids")
}

pub async fn visible_genres_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<GenreRow>> {
    let sql = format!(
        "SELECT g.id, g.name, lower(g.name) AS sort_key FROM genres g WHERE g.id = ANY($4) AND g.family_id = $2
            AND EXISTS (SELECT 1 FROM track_genres tg JOIN music_tracks t ON t.id = tg.track_id WHERE tg.genre_id = g.id AND {})
          ORDER BY g.id",
        music()
    );
    viewer!(sqlx::query_as::<_, GenreRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: visible genres by ids")
}

pub async fn books_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<BookRow>> {
    let sql = format!("SELECT {BOOK_COLS} FROM audiobook_books t WHERE t.id = ANY($4) AND {} ORDER BY t.id", books());
    viewer!(sqlx::query_as::<_, BookRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: books by ids")
}

pub async fn podcasts_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<PodcastRow>> {
    let sql = format!("SELECT {PODCAST_COLS} FROM podcast_feeds t WHERE t.id = ANY($4) AND {} ORDER BY t.id", podcasts());
    viewer!(sqlx::query_as::<_, PodcastRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: podcasts by ids")
}

pub async fn episodes_by_ids(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<EpisodeRow>> {
    let sql = format!(
        "SELECT {EPISODE_COLS} FROM podcast_episodes e LEFT JOIN media_objects m ON m.id = e.audio_object_id
          WHERE e.id = ANY($4) AND EXISTS (SELECT 1 FROM podcast_feeds t WHERE t.id = e.feed_id AND {}) ORDER BY e.id",
        podcasts()
    );
    viewer!(sqlx::query_as::<_, EpisodeRow>(&sql), viewer).bind(ids).fetch_all(pool).await.context("db: episodes by ids")
}

/// The next ids of a kind the viewer can see, in id order: the snapshot's walk.
pub async fn visible_ids_after(pool: &PgPool, viewer: Viewer, kind: &str, after: Uuid, limit: i64) -> anyhow::Result<Vec<Uuid>> {
    let source = match kind {
        "track" => format!("SELECT t.id FROM music_tracks t WHERE {} AND t.id > $4", music()),
        "album" => format!(
            "SELECT a.id FROM albums a WHERE a.family_id = $2 AND a.id > $4 AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = a.id AND {})",
            music()
        ),
        "contributor" => format!("SELECT k.id FROM contributors k WHERE k.family_id = $2 AND k.id > $4 AND {}", credited_visibly("NULL")),
        "release_group" => format!(
            "SELECT g.id FROM release_groups g WHERE g.family_id = $2 AND g.id > $4
                AND EXISTS (SELECT 1 FROM albums a JOIN music_tracks t ON t.album_id = a.id WHERE a.release_group_id = g.id AND {})",
            music()
        ),
        "genre" => format!(
            "SELECT g.id FROM genres g WHERE g.family_id = $2 AND g.id > $4
                AND EXISTS (SELECT 1 FROM track_genres tg JOIN music_tracks t ON t.id = tg.track_id WHERE tg.genre_id = g.id AND {})",
            music()
        ),
        "book" => format!("SELECT t.id FROM audiobook_books t WHERE {} AND t.id > $4", books()),
        "podcast" => format!("SELECT t.id FROM podcast_feeds t WHERE {} AND t.id > $4", podcasts()),
        "episode" => format!(
            "SELECT e.id FROM podcast_episodes e JOIN podcast_feeds t ON t.id = e.feed_id WHERE {} AND e.id > $4",
            podcasts()
        ),
        _ => anyhow::bail!("no snapshot of {kind}"),
    };
    let sql = format!("{source} ORDER BY 1 LIMIT $5");
    viewer!(sqlx::query_scalar::<_, Uuid>(&sql), viewer)
        .bind(after)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: snapshot ids")
}

// ── Playlists (V10) ──────────────────────────────────────────────────────────
//
// A playlist is its owner's, private or shared with the household, and is
// seen through the viewer's `music` policy like a track. Its entries are kept
// whoever can see their tracks; each reader is shown, and counted, only the
// entries whose tracks they can see.

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PlaylistRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub cover_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// What the viewer sees of it.
    pub track_count: i64,
    pub duration_secs: i64,
    /// The first visible entry's cover, for a playlist without its own.
    pub first_cover: Option<Uuid>,
    /// `lower(name)`, for the cursor.
    pub sort_key: String,
}

/// Visible playlists in name order after `after`, or the one with `id`.
pub async fn list_playlists(
    pool: &PgPool,
    viewer: Viewer,
    id: Option<Uuid>,
    after: Option<(String, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<PlaylistRow>> {
    let music = music();
    // The playlist is `t` for the visibility rule, and so is each track inside
    // the lateral count, hence the CTE between them.
    let sql = format!(
        "WITH pl AS (
            SELECT t.id, t.user_id, t.family_id, t.name, t.description, t.cover_object_id, t.created_at, t.updated_at
              FROM music_playlists t
             WHERE {music} AND ($4::uuid IS NULL OR t.id = $4)
               AND ($5::text IS NULL OR (lower(t.name), t.id) > ($5, $6))
             ORDER BY lower(t.name), t.id
             LIMIT $7)
         SELECT pl.*, s.n AS track_count, s.secs AS duration_secs, s.cover AS first_cover, lower(pl.name) AS sort_key
           FROM pl
           LEFT JOIN LATERAL (
                SELECT count(*) AS n, COALESCE(sum(t.duration_secs), 0)::bigint AS secs,
                       (array_agg(t.cover_object_id ORDER BY e.position) FILTER (WHERE t.cover_object_id IS NOT NULL))[1] AS cover
                  FROM music_playlist_tracks e JOIN music_tracks t ON t.id = e.track_id
                 WHERE e.playlist_id = pl.id AND {music}) s ON true
          ORDER BY lower(pl.name), pl.id"
    );
    let (name, after_id) = after.map_or((None, None), |(n, i)| (Some(n), Some(i)));
    viewer!(sqlx::query_as::<_, PlaylistRow>(&sql), viewer)
        .bind(id)
        .bind(name)
        .bind(after_id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list playlists")
}

pub async fn find_playlist(pool: &PgPool, viewer: Viewer, id: Uuid) -> anyhow::Result<Option<PlaylistRow>> {
    Ok(list_playlists(pool, viewer, Some(id), None, 1).await?.into_iter().next())
}

/// A playlist's entries the viewer can see, in order, after position `after`.
pub async fn playlist_entries(
    pool: &PgPool,
    viewer: Viewer,
    playlist_id: Uuid,
    after: Option<i32>,
    limit: i64,
) -> anyhow::Result<Vec<(Uuid, i32, DateTime<Utc>, TrackRow)>> {
    #[derive(sqlx::FromRow)]
    struct Row {
        entry_id: Uuid,
        position: i32,
        added_at: DateTime<Utc>,
        #[sqlx(flatten)]
        track: TrackRow,
    }
    let sql = format!(
        "SELECT e.id AS entry_id, e.position, e.added_at, {TRACK_COLS}
           FROM music_playlist_tracks e JOIN music_tracks t ON t.id = e.track_id
          WHERE e.playlist_id = $4 AND ($5::int IS NULL OR e.position > $5) AND {}
          ORDER BY e.position
          LIMIT $6",
        music()
    );
    let rows = viewer!(sqlx::query_as::<_, Row>(&sql), viewer)
        .bind(playlist_id)
        .bind(after)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: playlist entries")?;
    Ok(rows.into_iter().map(|r| (r.entry_id, r.position, r.added_at, r.track)).collect())
}

/// Name, description and sharing at once; the owner's only.
pub async fn update_playlist(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
    family_id: Option<Uuid>,
) -> anyhow::Result<bool> {
    let done = sqlx::query(
        "UPDATE music_playlists SET name = $3, description = $4, family_id = $5, updated_at = now()
          WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(family_id)
    .execute(pool)
    .await
    .context("db: update playlist")?;
    Ok(done.rows_affected() > 0)
}

pub async fn touch_playlist(pool: &PgPool, id: Uuid) -> anyhow::Result<()> {
    sqlx::query("UPDATE music_playlists SET updated_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await
        .context("db: touch playlist")?;
    Ok(())
}

pub async fn remove_playlist_entry(pool: &PgPool, playlist_id: Uuid, entry_id: Uuid) -> anyhow::Result<bool> {
    let done = sqlx::query("DELETE FROM music_playlist_tracks WHERE id = $1 AND playlist_id = $2")
        .bind(entry_id)
        .bind(playlist_id)
        .execute(pool)
        .await
        .context("db: remove playlist entry")?;
    Ok(done.rows_affected() > 0)
}

/// Puts `first` at the front in that order, the other entries after them as
/// they were, and numbers them 1…n. `false` when an id is not this playlist's.
pub async fn order_playlist(pool: &PgPool, playlist_id: Uuid, first: &[Uuid]) -> anyhow::Result<bool> {
    let mut tx = pool.begin().await.context("db: begin playlist order")?;
    // Taken in turn with every other writer of this playlist's order.
    sqlx::query("SELECT 1 FROM music_playlists_all WHERE id = $1 FOR UPDATE")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .context("db: lock playlist")?;
    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM music_playlist_tracks WHERE playlist_id = $1 AND id = ANY($2)")
        .bind(playlist_id)
        .bind(first)
        .fetch_one(&mut *tx)
        .await
        .context("db: check playlist entries")?;
    let distinct: std::collections::HashSet<&Uuid> = first.iter().collect();
    if known != first.len() as i64 || distinct.len() != first.len() {
        return Ok(false);
    }
    sqlx::query("UPDATE music_playlist_tracks SET position = -position WHERE playlist_id = $1")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .context("db: clear playlist positions")?;
    sqlx::query(
        "UPDATE music_playlist_tracks e SET position = o.n
           FROM (SELECT e2.id, row_number() OVER (ORDER BY x.ord NULLS LAST, -e2.position) AS n
                   FROM music_playlist_tracks e2
                   LEFT JOIN unnest($2::uuid[]) WITH ORDINALITY AS x(id, ord) ON x.id = e2.id
                  WHERE e2.playlist_id = $1) o
          WHERE e.id = o.id",
    )
    .bind(playlist_id)
    .bind(first)
    .execute(&mut *tx)
    .await
    .context("db: order playlist")?;
    sqlx::query("UPDATE music_playlists_all SET updated_at = now() WHERE id = $1")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .context("db: touch playlist")?;
    tx.commit().await.context("db: commit playlist order")?;
    Ok(true)
}

// ── Search, random tracks, plays (V10) ───────────────────────────────────────

/// Contributors credited as artists on something visible whose name holds `q`;
/// names that start with it first.
pub async fn search_artists(pool: &PgPool, viewer: Viewer, q: &str, limit: i64) -> anyhow::Result<Vec<ContributorRow>> {
    let sql = format!(
        "SELECT {CONTRIBUTOR_COLS} FROM contributors k
          WHERE k.family_id = $2 AND k.name ILIKE '%' || $4 || '%' AND {}
          ORDER BY (k.name ILIKE $4 || '%') DESC, lower(COALESCE(k.sort_name, k.name)), k.id
          LIMIT $5",
        credited_visibly("ARRAY['artist','featured']")
    );
    viewer!(sqlx::query_as::<_, ContributorRow>(&sql), viewer)
        .bind(escape_like(q))
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: search artists")
}

/// Albums with a visible track whose title holds `q`.
pub async fn search_albums(pool: &PgPool, viewer: Viewer, q: &str, limit: i64) -> anyhow::Result<Vec<AlbumRow>> {
    let sql = format!(
        "SELECT {ALBUM_COLS} FROM albums a
          WHERE a.family_id = $2 AND a.title ILIKE '%' || $4 || '%'
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.album_id = a.id AND {})
          ORDER BY (a.title ILIKE $4 || '%') DESC, lower(a.title), a.id
          LIMIT $5",
        music()
    );
    viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(escape_like(q))
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: search albums")
}

/// Visible tracks whose title, album title or a credited name holds `q`;
/// title matches first.
pub async fn search_tracks(pool: &PgPool, viewer: Viewer, q: &str, limit: i64) -> anyhow::Result<Vec<TrackRow>> {
    let sql = format!(
        // Matching titles, albums and names are each found once and then
        // joined, rather than every track looking up its album and credits.
        "SELECT {TRACK_COLS} FROM music_tracks t
          WHERE t.id IN (SELECT x.id FROM music_tracks x WHERE x.title ILIKE '%' || $4 || '%'
                         UNION SELECT x.id FROM albums a JOIN music_tracks x ON x.album_id = a.id
                                WHERE a.family_id = $2 AND a.title ILIKE '%' || $4 || '%'
                         UNION SELECT c.item_id FROM contributors k JOIN credits c ON c.contributor_id = k.id AND c.item_kind = 'track'
                                WHERE k.family_id = $2 AND k.name ILIKE '%' || $4 || '%')
            AND {}
          ORDER BY (t.title ILIKE $4 || '%') DESC, (t.title ILIKE '%' || $4 || '%') DESC, lower(t.title), t.id
          LIMIT $5",
        music()
    );
    viewer!(sqlx::query_as::<_, TrackRow>(&sql), viewer)
        .bind(escape_like(q))
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: search tracks")
}

/// Visible tracks in random order, from one genre when given.
pub async fn random_tracks(pool: &PgPool, viewer: Viewer, genre_id: Option<Uuid>, limit: i64) -> anyhow::Result<Vec<TrackRow>> {
    let sql = format!(
        "SELECT {TRACK_COLS} FROM music_tracks t
          WHERE {} AND ($4::uuid IS NULL OR EXISTS (SELECT 1 FROM track_genres g WHERE g.track_id = t.id AND g.genre_id = $4))
          ORDER BY random()
          LIMIT $5",
        music()
    );
    viewer!(sqlx::query_as::<_, TrackRow>(&sql), viewer)
        .bind(genre_id)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: random tracks")
}

/// The viewer's own plays of what they can still see, per track or per album:
/// (id, plays, last played), most recent or most played first.
pub async fn played(
    pool: &PgPool,
    viewer: Viewer,
    per_album: bool,
    most: bool,
    limit: i64,
) -> anyhow::Result<Vec<(Uuid, i64, DateTime<Utc>)>> {
    let key = if per_album { "t.album_id" } else { "t.id" };
    let order = if most { "count(*) DESC, max(COALESCE(s.ended_at, s.started_at)) DESC" } else { "max(COALESCE(s.ended_at, s.started_at)) DESC" };
    let sql = format!(
        "SELECT {key}, count(*), max(COALESCE(s.ended_at, s.started_at))
           FROM listening_sessions s JOIN music_tracks t ON t.id = s.item_id
          WHERE s.user_id = $1 AND s.media_kind = 'music' AND {key} IS NOT NULL AND {}
          GROUP BY {key}
          ORDER BY {order}, {key}
          LIMIT $4",
        music()
    );
    viewer!(sqlx::query_as::<_, (Uuid, i64, DateTime<Utc>)>(&sql), viewer)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: played")
}

/// The track's lyrics as stored: `None` never looked for, `""` none found.
pub async fn track_lyrics(pool: &PgPool, id: Uuid) -> anyhow::Result<(Option<String>, Option<Uuid>)> {
    sqlx::query_as::<_, (Option<String>, Option<Uuid>)>("SELECT lyrics, audio_object_id FROM music_tracks WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("db: track lyrics")
}
