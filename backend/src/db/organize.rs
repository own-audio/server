// SPDX-License-Identifier: AGPL-3.0-or-later
//! The organizer's view of a member's own songs (docs/metadata-organizer-plan.md,
//! O1): every editable field, where the file is, whether it is identified, with
//! filters, facets and a save of many songs as one batch of edits.

use crate::db::edits::{self, Edit};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::{FromRow, PgPool};
use std::sync::LazyLock;
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct OrganizeRow {
    pub id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub disc_total: Option<i32>,
    pub is_compilation: bool,
    pub duration_secs: Option<i32>,
    pub album_id: Option<Uuid>,
    pub year: Option<String>,
    pub musicbrainz_recording_id: Option<String>,
    pub musicbrainz_release_id: Option<String>,
    /// `identify`, `mbid_tag`, or none.
    pub identity: Option<String>,
    pub has_cover: bool,
    /// The library folder the file is in; none for an upload.
    pub folder_id: Option<Uuid>,
    /// The file's path in its folder, or the uploaded object's name.
    pub path: String,
    pub content_type: String,
    pub size_bytes: Option<i64>,
    pub edited: bool,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub q: Option<String>,
    pub folder: Option<Uuid>,
    /// A folder path prefix inside `folder`, "Artist/Album/".
    pub path: Option<String>,
    pub artist: Option<String>,
    pub album_id: Option<Uuid>,
    pub unidentified: bool,
    pub no_cover: bool,
    pub no_year: bool,
    pub no_genre: bool,
    pub edited: bool,
    pub duplicates: bool,
}

const SELECT: &str = "SELECT t.id, t.title, t.artist, t.album, t.album_artist, t.genre, t.track_number,
        t.disc_number, t.disc_total, t.is_compilation, t.duration_secs, t.album_id,
        left(al.release_date, 4) AS year, t.musicbrainz_recording_id, t.musicbrainz_release_id,
        i.source AS identity, t.cover_object_id IS NOT NULL AS has_cover,
        CASE WHEN mo.object_key LIKE 'folder/%' THEN split_part(mo.object_key, '/', 2)::uuid END AS folder_id,
        CASE WHEN mo.object_key LIKE 'folder/%'
             THEN substr(mo.object_key, length(split_part(mo.object_key, '/', 2)) + 9)
             ELSE regexp_replace(mo.object_key, '^.*/', '') END AS path,
        mo.content_type, mo.size_bytes,
        EXISTS (SELECT 1 FROM metadata_edits e WHERE e.item_kind = 'track' AND e.item_id = t.id) AS edited,
        t.updated_at
   FROM music_tracks t
   JOIN media_objects mo ON mo.id = t.audio_object_id
   LEFT JOIN albums al ON al.id = t.album_id
   LEFT JOIN music_track_identity i ON i.track_id = t.id";

/// The duplicates filter's test, for the count beside it: the same file, the same MusicBrainz
/// recording, or the same title and artist (spelling and punctuation aside) within a few
/// seconds of the same length and not known to be two different recordings — a studio take
/// and a live one share a title, and both identified they are not a pair. `WHERE`'s twelfth
/// parameter is the same test.
const DUPLICATE: &str = "EXISTS (SELECT 1 FROM music_tracks d JOIN media_objects dm ON dm.id = d.audio_object_id
         WHERE d.user_id = t.user_id AND d.id <> t.id
           AND (dm.sha256 = mo.sha256
                OR d.musicbrainz_recording_id = t.musicbrainz_recording_id
                OR (lower(regexp_replace(d.title, '[^[:alnum:]]+', '', 'g'))
                      = lower(regexp_replace(t.title, '[^[:alnum:]]+', '', 'g'))
                    AND d.primary_artist_key = t.primary_artist_key
                    AND (d.musicbrainz_recording_id IS NULL OR t.musicbrainz_recording_id IS NULL
                         OR d.musicbrainz_recording_id = t.musicbrainz_recording_id)
                    AND (d.duration_secs IS NULL OR t.duration_secs IS NULL
                         OR abs(d.duration_secs - t.duration_secs) <= 3))))";

static WHERE: LazyLock<String> = LazyLock::new(|| {
    format!(
        "t.user_id = $1
    AND ($2::text IS NULL OR t.title ILIKE '%' || $2 || '%' OR t.artist ILIKE '%' || $2 || '%'
         OR t.album ILIKE '%' || $2 || '%' OR mo.object_key ILIKE '%' || $2 || '%')
    AND ($3::uuid IS NULL OR mo.object_key LIKE 'folder/' || $3 || '/%')
    AND ($4::text IS NULL OR mo.object_key LIKE 'folder/' || $3 || '/' || $4 || '%')
    AND ($5::text IS NULL OR t.album_artist_key = $5 OR t.primary_artist_key = $5)
    AND ($6::uuid IS NULL OR t.album_id = $6)
    AND (NOT $7 OR t.musicbrainz_recording_id IS NULL)
    AND (NOT $8 OR t.cover_object_id IS NULL)
    AND (NOT $9 OR al.release_date IS NULL)
    AND (NOT $10 OR t.genre IS NULL OR trim(t.genre) = '')
    AND (NOT $11 OR EXISTS (SELECT 1 FROM metadata_edits e WHERE e.item_kind = 'track' AND e.item_id = t.id))
    AND (NOT $12 OR {DUPLICATE})"
    )
});

/// The columns a list may be sorted by, and the SQL for each.
pub fn sort_sql(sort: &str) -> Option<&'static str> {
    Some(match sort {
        "title" => "lower(t.title)",
        "artist" => "lower(t.artist)",
        "album" => "lower(t.album)",
        "album_artist" => "t.album_artist_key",
        "genre" => "lower(t.genre)",
        "track" => "t.disc_number, t.track_number",
        "year" => "al.release_date",
        "path" => "mo.object_key",
        "duration" => "t.duration_secs",
        "updated" => "t.updated_at",
        _ => return None,
    })
}

pub const DEFAULT_SORT: &str = "t.album_artist_key, lower(t.album), t.disc_number, t.track_number, lower(t.title)";

/// The order for the duplicates filter: a pair side by side, whatever albums the two are on.
/// No space after the commas inside the call: `list` splits the columns on ", ".
pub const DUPLICATES_SORT: &str =
    "t.primary_artist_key, lower(regexp_replace(t.title,'[^[:alnum:]]+','','g')), t.duration_secs, mo.object_key";

pub async fn list(
    pool: &PgPool,
    user_id: Uuid,
    f: &Filter,
    order: &str,
    desc: bool,
    offset: i64,
    limit: i64,
) -> anyhow::Result<(Vec<OrganizeRow>, i64)> {
    let dir = if desc { "DESC" } else { "ASC" };
    let order = order.split(", ").map(|c| format!("{c} {dir} NULLS LAST")).collect::<Vec<_>>().join(", ");
    let rows = bind(sqlx::query_as::<_, OrganizeRow>(&format!(
        "{SELECT} WHERE {} ORDER BY {order}, t.id LIMIT $13 OFFSET $14",
        *WHERE
    )), user_id, f)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await
    .context("db: organize list")?;
    let total: i64 = bind_scalar(
        sqlx::query_scalar(&format!(
            "SELECT count(*) FROM music_tracks t JOIN media_objects mo ON mo.id = t.audio_object_id
               LEFT JOIN albums al ON al.id = t.album_id WHERE {}",
            *WHERE
        )),
        user_id,
        f,
    )
    .fetch_one(pool)
    .await
    .context("db: organize count")?;
    Ok((rows, total))
}

macro_rules! bind_filter {
    ($q:expr, $user:expr, $f:expr) => {
        $q.bind($user)
            .bind($f.q.as_deref().map(str::trim).filter(|s| !s.is_empty()))
            .bind($f.folder)
            .bind($f.path.as_deref().filter(|s| !s.is_empty()).map(|p| p.replace('%', "\\%").replace('_', "\\_")))
            .bind($f.artist.as_deref())
            .bind($f.album_id)
            .bind($f.unidentified)
            .bind($f.no_cover)
            .bind($f.no_year)
            .bind($f.no_genre)
            .bind($f.edited)
            .bind($f.duplicates)
    };
}

fn bind<'q>(
    q: sqlx::query::QueryAs<'q, sqlx::Postgres, OrganizeRow, sqlx::postgres::PgArguments>,
    user: Uuid,
    f: &'q Filter,
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, OrganizeRow, sqlx::postgres::PgArguments> {
    bind_filter!(q, user, f)
}

fn bind_scalar<'q>(
    q: sqlx::query::QueryScalar<'q, sqlx::Postgres, i64, sqlx::postgres::PgArguments>,
    user: Uuid,
    f: &'q Filter,
) -> sqlx::query::QueryScalar<'q, sqlx::Postgres, i64, sqlx::postgres::PgArguments> {
    bind_filter!(q, user, f)
}

#[derive(Debug, Clone, FromRow)]
pub struct FolderFacet {
    pub id: Option<Uuid>,
    /// The folder on the server's disk; none for uploads.
    pub path: Option<String>,
    pub tracks: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct ArtistFacet {
    pub key: String,
    pub tracks: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct AlbumFacet {
    pub id: Uuid,
    pub title: String,
    pub artist: String,
    pub tracks: i64,
}

#[derive(Debug, Clone, FromRow)]
pub struct Counts {
    pub all: i64,
    pub unidentified: i64,
    pub no_cover: i64,
    pub no_year: i64,
    pub no_genre: i64,
    pub edited: i64,
    pub duplicates: i64,
}

pub async fn facets(pool: &PgPool, user_id: Uuid) -> anyhow::Result<(Vec<FolderFacet>, Vec<ArtistFacet>, Vec<AlbumFacet>, Counts)> {
    let folders = sqlx::query_as::<_, FolderFacet>(
        "SELECT f.id, f.path, count(*) AS tracks
           FROM music_tracks t JOIN media_objects mo ON mo.id = t.audio_object_id
           LEFT JOIN library_folders f ON mo.object_key LIKE 'folder/%' AND f.id = split_part(mo.object_key, '/', 2)::uuid
          WHERE t.user_id = $1
          GROUP BY f.id, f.path ORDER BY f.path NULLS LAST",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: organize folders")?;
    let artists = sqlx::query_as::<_, ArtistFacet>(
        "SELECT t.album_artist_key AS key, count(*) AS tracks FROM music_tracks t
          WHERE t.user_id = $1 GROUP BY t.album_artist_key ORDER BY lower(t.album_artist_key)",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: organize artists")?;
    let albums = sqlx::query_as::<_, AlbumFacet>(
        "SELECT al.id, al.title, al.artist_display AS artist, count(*) AS tracks
           FROM music_tracks t JOIN albums al ON al.id = t.album_id
          WHERE t.user_id = $1 GROUP BY al.id ORDER BY lower(al.artist_display), lower(al.title)",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: organize albums")?;
    let counts = sqlx::query_as::<_, Counts>(&format!(
        "SELECT count(*) AS all,
                count(*) FILTER (WHERE t.musicbrainz_recording_id IS NULL) AS unidentified,
                count(*) FILTER (WHERE t.cover_object_id IS NULL) AS no_cover,
                count(*) FILTER (WHERE al.release_date IS NULL) AS no_year,
                count(*) FILTER (WHERE t.genre IS NULL OR trim(t.genre) = '') AS no_genre,
                count(*) FILTER (WHERE EXISTS (SELECT 1 FROM metadata_edits e
                                                WHERE e.item_kind = 'track' AND e.item_id = t.id)) AS edited,
                count(*) FILTER (WHERE {DUPLICATE}) AS duplicates
           FROM music_tracks t JOIN media_objects mo ON mo.id = t.audio_object_id
                LEFT JOIN albums al ON al.id = t.album_id
          WHERE t.user_id = $1"
    ))
    .bind(user_id)
    .fetch_one(pool)
    .await
    .context("db: organize counts")?;
    Ok((folders, artists, albums, counts))
}

/// One song's new values: a field absent stays, `Some(None)` clears it.
#[derive(Debug, Default, Clone)]
pub struct TrackChange {
    pub id: Uuid,
    pub title: Option<String>,
    pub artist: Option<Option<String>>,
    pub album: Option<Option<String>>,
    pub album_artist: Option<Option<String>>,
    pub genre: Option<Option<String>>,
    pub track_number: Option<Option<i32>>,
    pub disc_number: Option<Option<i32>>,
    pub is_compilation: Option<bool>,
}

/// Save many songs' changes as one batch. Only the caller's songs change; the
/// count says how many did.
pub async fn save(pool: &PgPool, user_id: Uuid, changes: &[TrackChange]) -> anyhow::Result<(Uuid, u64)> {
    let edit = Edit::new(user_id, if changes.len() > 1 { "bulk" } else { "manual" });
    let mut tx = pool.begin().await.context("db: begin organize save")?;
    edits::mark(&mut tx, &edit).await?;
    let mut n = 0;
    for c in changes {
        let clean = |v: &Option<Option<String>>| v.as_ref().map(|o| o.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(str::to_string));
        let (artist, album, album_artist, genre) = (clean(&c.artist), clean(&c.album), clean(&c.album_artist), clean(&c.genre));
        n += sqlx::query(
            "UPDATE music_tracks_all SET
                title          = COALESCE($3, title),
                artist         = CASE WHEN $4 THEN $5 ELSE artist END,
                album          = CASE WHEN $6 THEN $7 ELSE album END,
                album_artist   = CASE WHEN $8 THEN $9 ELSE album_artist END,
                genre          = CASE WHEN $10 THEN $11 ELSE genre END,
                track_number   = CASE WHEN $12 THEN $13 ELSE track_number END,
                disc_number    = CASE WHEN $14 THEN $15 ELSE disc_number END,
                is_compilation = COALESCE($16, is_compilation),
                updated_at = now()
              WHERE id = $1 AND user_id = $2 AND trashed_at IS NULL",
        )
        .bind(c.id)
        .bind(user_id)
        .bind(c.title.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(artist.is_some())
        .bind(artist.flatten())
        .bind(album.is_some())
        .bind(album.flatten())
        .bind(album_artist.is_some())
        .bind(album_artist.flatten())
        .bind(genre.is_some())
        .bind(genre.flatten())
        .bind(c.track_number.is_some())
        .bind(c.track_number.flatten())
        .bind(c.disc_number.is_some())
        .bind(c.disc_number.flatten())
        .bind(c.is_compilation)
        .execute(&mut *tx)
        .await
        .context("db: organize save")?
        .rows_affected();
    }
    tx.commit().await.context("db: commit organize save")?;
    Ok((edit.batch_id, n))
}
