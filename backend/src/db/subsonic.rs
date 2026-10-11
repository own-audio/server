// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::access::{MUSIC, VISIBLE, Viewer};
/// Reused rather than restated: this module and `db::music` decode into the
/// same [`MusicTrack`], so a second copy of the column list silently rots the
/// moment a column is added. It did — the MusicBrainz and lyrics columns landed
/// in the struct without a local copy here learning about them, and `getAlbum`
/// and `search3` failed to decode for every client.
use crate::db::music::TRACK_COLS;
use crate::music::models::MusicTrack;
use anyhow::Context;
use chrono::{DateTime, Utc};
use rand::Rng;
use rand::distr::Alphanumeric;
use sqlx::PgPool;
use crate::auth::at_rest::Cipher;
use uuid::Uuid;

const UNKNOWN_ARTIST: &str = "Unknown Artist";
const UNKNOWN_ALBUM: &str = "Unknown Album";

/// Look up the user's Subsonic API key without creating one. The key is
/// stored encrypted (`auth::at_rest`); one written in the clear before that
/// is rewritten encrypted on the way out.
pub async fn get_key(pool: &PgPool, cipher: &Cipher, user_id: Uuid) -> anyhow::Result<Option<String>> {
    let Some(stored) = find_key(pool, user_id).await? else {
        return Ok(None);
    };
    let (key, sealed) = cipher.open(&stored)?;
    if !sealed {
        store_key(pool, user_id, &cipher.seal(&key), false).await?;
    }
    Ok(Some(key))
}

/// Fetch the user's Subsonic API key, creating one on first use.
pub async fn get_or_create_key(pool: &PgPool, cipher: &Cipher, user_id: Uuid) -> anyhow::Result<String> {
    if let Some(key) = get_key(pool, cipher, user_id).await? {
        return Ok(key);
    }
    store_key(pool, user_id, &cipher.seal(&generate_key()), false).await?;
    // Another request may have raced us to creation; re-fetch to be sure.
    get_key(pool, cipher, user_id)
        .await?
        .context("db: subsonic api key missing after insert")
}

/// Replace the user's Subsonic API key with a freshly generated one.
pub async fn regenerate_key(pool: &PgPool, cipher: &Cipher, user_id: Uuid) -> anyhow::Result<String> {
    let key = generate_key();
    store_key(pool, user_id, &cipher.seal(&key), true).await?;
    Ok(key)
}

/// Encrypts every key still stored in the clear; returns how many. Run once
/// at start-up, so a database dump stops carrying usable keys the moment the
/// server is upgraded, not when each user next signs in.
pub async fn encrypt_legacy_keys(pool: &PgPool, cipher: &Cipher) -> anyhow::Result<usize> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as("SELECT user_id, api_key FROM subsonic_api_keys WHERE api_key NOT LIKE 'enc1:%'")
        .fetch_all(pool)
        .await
        .context("db: list plaintext subsonic api keys")?;
    for (user_id, key) in &rows {
        store_key(pool, *user_id, &cipher.seal(key), false).await?;
    }
    Ok(rows.len())
}

async fn store_key(pool: &PgPool, user_id: Uuid, stored: &str, replace: bool) -> anyhow::Result<()> {
    let sql = if replace {
        "INSERT INTO subsonic_api_keys (user_id, api_key) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET api_key = EXCLUDED.api_key, created_at = now()"
    } else {
        "INSERT INTO subsonic_api_keys (user_id, api_key) VALUES ($1, $2)
         ON CONFLICT (user_id) DO UPDATE SET api_key = EXCLUDED.api_key"
    };
    sqlx::query(sql)
        .bind(user_id)
        .bind(stored)
        .execute(pool)
        .await
        .context("db: store subsonic api key")?;
    Ok(())
}

async fn find_key(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT api_key FROM subsonic_api_keys WHERE user_id = $1")
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .context("db: find subsonic api key")
}

fn generate_key() -> String {
    rand::rng()
        .sample_iter(&Alphanumeric)
        .take(32)
        .map(char::from)
        .collect()
}

// ── ID3 browsing (Subsonic) ──────────────────────────────────────────────────
//
// `music_tracks` has no normalized artist/album tables (denormalized string
// columns), so these queries aggregate on the fly. Fine at self-hosted-library
// scale; the Subsonic route layer derives stable synthetic IDs for the
// distinct (artist) / (artist, album) values these return.
//
// All of them apply the same visibility rule as the REST API, so a Subsonic
// client shows exactly the library the web UI does and never another member's
// private content. VISIBLE occupies $1..$4; extra parameters start at $5.

/// Bind the four leading parameters every [`VISIBLE`] query expects.
macro_rules! bind_viewer {
    ($q:expr, $viewer:expr) => {
        $q.bind($viewer.user_id)
            .bind($viewer.family_id)
            .bind($viewer.is_family_admin)
            .bind(MUSIC)
    };
}

#[derive(sqlx::FromRow)]
pub struct ArtistRow {
    pub artist: String,
    pub album_count: i64,
}

pub async fn list_distinct_artists(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<ArtistRow>> {
    let sql = format!(
        "SELECT COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') AS artist,
                COUNT(DISTINCT COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}')) AS album_count
         FROM music_tracks t
         WHERE {VISIBLE}
         GROUP BY 1
         ORDER BY 1"
    );
    bind_viewer!(sqlx::query_as::<_, ArtistRow>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list distinct artists")
}

#[derive(sqlx::FromRow)]
pub struct AlbumRow {
    pub artist: String,
    pub album: String,
    pub song_count: i64,
    pub duration_secs: Option<i64>,
    pub cover_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

pub async fn list_distinct_albums(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<AlbumRow>> {
    let sql = format!(
        "SELECT COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') AS artist,
                COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}') AS album,
                COUNT(*) AS song_count,
                SUM(t.duration_secs)::bigint AS duration_secs,
                (array_agg(t.cover_object_id ORDER BY (t.cover_object_id IS NULL), t.created_at))[1] AS cover_object_id,
                MIN(t.created_at) AS created_at
         FROM music_tracks t
         WHERE {VISIBLE}
         GROUP BY 1, 2
         ORDER BY MIN(t.created_at) DESC"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list distinct albums")
}

const ARTIST_EXPR: &str = "COALESCE(NULLIF(trim(t.artist), ''), 'Unknown Artist')";
const ALBUM_EXPR: &str = "COALESCE(NULLIF(trim(t.album), ''), 'Unknown Album')";

/// One album as [`list_distinct_albums`] would list it, found by its names
/// through `music_tracks_subsonic_group_idx`.
pub async fn find_album(pool: &PgPool, viewer: Viewer, artist: &str, album: &str) -> anyhow::Result<Option<AlbumRow>> {
    let sql = format!(
        "SELECT {ARTIST_EXPR} AS artist, {ALBUM_EXPR} AS album,
                COUNT(*) AS song_count,
                SUM(t.duration_secs)::bigint AS duration_secs,
                (array_agg(t.cover_object_id ORDER BY (t.cover_object_id IS NULL), t.created_at))[1] AS cover_object_id,
                MIN(t.created_at) AS created_at
         FROM music_tracks t
         WHERE {VISIBLE} AND {ARTIST_EXPR} = $5 AND {ALBUM_EXPR} = $6
         GROUP BY 1, 2"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(artist)
        .bind(album)
        .fetch_optional(pool)
        .await
        .context("db: find album")
}

/// An artist's albums, newest first, as [`list_distinct_albums`] lists them.
pub async fn list_artist_albums(pool: &PgPool, viewer: Viewer, artist: &str) -> anyhow::Result<Vec<AlbumRow>> {
    let sql = format!(
        "SELECT {ARTIST_EXPR} AS artist, {ALBUM_EXPR} AS album,
                COUNT(*) AS song_count,
                SUM(t.duration_secs)::bigint AS duration_secs,
                (array_agg(t.cover_object_id ORDER BY (t.cover_object_id IS NULL), t.created_at))[1] AS cover_object_id,
                MIN(t.created_at) AS created_at
         FROM music_tracks t
         WHERE {VISIBLE} AND {ARTIST_EXPR} = $5
         GROUP BY 1, 2
         ORDER BY MIN(t.created_at) DESC"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(artist)
        .fetch_all(pool)
        .await
        .context("db: list artist albums")
}

/// The names a Subsonic album or artist id stands for, if this viewer has
/// been handed it before (`subsonic::resolve`). `album` is `None` for an artist.
pub async fn cached_names(pool: &PgPool, user_id: Uuid, id: Uuid) -> anyhow::Result<Option<(String, Option<String>)>> {
    sqlx::query_as::<_, (String, Option<String>)>("SELECT artist, album FROM subsonic_ids WHERE user_id = $1 AND id = $2")
        .bind(user_id)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: cached subsonic id")
}

/// Remember ids and their names; already known ones are left alone.
pub async fn remember_names(
    pool: &PgPool,
    user_id: Uuid,
    ids: &[Uuid],
    artists: &[String],
    albums: &[Option<String>],
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO subsonic_ids (user_id, id, artist, album)
         SELECT $1, * FROM unnest($2::uuid[], $3::text[], $4::text[])
         ON CONFLICT DO NOTHING",
    )
    .bind(user_id)
    .bind(ids)
    .bind(artists)
    .bind(albums)
    .execute(pool)
    .await
    .context("db: remember subsonic ids")?;
    Ok(())
}

/// Fetch many tracks by id at once, applying the same visibility rule as
/// every other listing here.
///
/// Added for `getPlayQueue`, which stores bare ids and previously rendered
/// them as `<entry id="..."/>` with nothing else — a restored queue showed a
/// client a column of blank rows. Bulk rather than per-id because a saved
/// queue is routinely hundreds of tracks long.
///
/// Order is not preserved; the caller re-orders to match its own list, since
/// a queue's order is the point of it.
pub async fn tracks_by_ids(
    pool: &PgPool,
    viewer: Viewer,
    ids: &[Uuid],
) -> anyhow::Result<Vec<MusicTrack>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let sql = format!(
        "SELECT {TRACK_COLS} FROM music_tracks t WHERE {VISIBLE} AND t.id = ANY($5)"
    );
    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: tracks by ids")
}

/// `(playlist_id, song_count, duration_secs)` for many playlists at once.
///
/// Replaces a COUNT per playlist in the Subsonic playlist listing, and
/// supplies the `duration` that listing used to report as a flat zero.
pub async fn playlist_track_stats(
    pool: &PgPool,
    playlist_ids: &[Uuid],
) -> anyhow::Result<Vec<(Uuid, i64, i64)>> {
    if playlist_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, (Uuid, i64, i64)>(
        "SELECT pt.playlist_id,
                COUNT(*)::bigint,
                COALESCE(SUM(t.duration_secs), 0)::bigint
         FROM music_playlist_tracks pt
         JOIN music_tracks t ON t.id = pt.track_id
         WHERE pt.playlist_id = ANY($1)
         GROUP BY pt.playlist_id",
    )
    .bind(playlist_ids)
    .fetch_all(pool)
    .await
    .context("db: playlist track stats")
}

/// The most recent change to this viewer's visible music, for
/// `<indexes lastModified>`.
pub async fn library_last_modified(
    pool: &PgPool,
    viewer: Viewer,
) -> anyhow::Result<Option<DateTime<Utc>>> {
    let sql = format!(
        "SELECT MAX(GREATEST(t.created_at, t.updated_at)) FROM music_tracks t WHERE {VISIBLE}"
    );
    bind_viewer!(sqlx::query_scalar::<_, Option<DateTime<Utc>>>(&sql), viewer)
        .fetch_one(pool)
        .await
        .context("db: library last modified")
}

/// Per-album listening signals for one user, keyed the same way
/// [`list_distinct_albums`] keys its rows.
///
/// Exists so `getAlbumList`'s `frequent` and `recent` types mean what the
/// protocol says they mean. Both were previously ignored, which left a
/// client's "Most Played" and "Recently Played" shelves showing the newest
/// albums instead — the same wrong list under three different headings.
///
/// One query rather than a lookup per album: these shelves are drawn on every
/// home-screen refresh. `music_progress` is consulted alongside
/// `listening_sessions` because a client that only reports position (rather
/// than closing out a session) still evidences a recent play.
#[derive(sqlx::FromRow)]
pub struct AlbumPlayStats {
    pub artist: String,
    pub album: String,
    pub play_count: i64,
    pub last_played: Option<DateTime<Utc>>,
}

pub async fn album_play_stats(
    pool: &PgPool,
    viewer: Viewer,
) -> anyhow::Result<Vec<AlbumPlayStats>> {
    let sql = format!(
        "SELECT COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') AS artist,
                COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}') AS album,
                COALESCE(SUM(s.plays), 0)::bigint AS play_count,
                MAX(GREATEST(s.last_session, pr.updated_at)) AS last_played
         FROM music_tracks t
         LEFT JOIN (
             SELECT item_id, COUNT(*) AS plays, MAX(ended_at) AS last_session
             FROM listening_sessions
             WHERE media_kind = 'music' AND user_id = $1
             GROUP BY item_id
         ) s ON s.item_id = t.id
         LEFT JOIN music_progress pr ON pr.track_id = t.id AND pr.user_id = $1
         WHERE {VISIBLE}
         GROUP BY 1, 2"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumPlayStats>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: album play stats")
}

/// The `(artist, album)` keys of every album holding at least one track of
/// `genre`, for `getAlbumList&type=byGenre`.
///
/// Case-insensitive to match `db::music::list_tracks_by_genre`, so the genre
/// string a client echoes back from `getGenres` resolves identically whether
/// it asks for songs or for albums.
pub async fn album_keys_for_genre(
    pool: &PgPool,
    viewer: Viewer,
    genre: &str,
) -> anyhow::Result<Vec<(String, String)>> {
    let sql = format!(
        "SELECT DISTINCT COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') AS artist,
                COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}') AS album
         FROM music_tracks t
         WHERE {VISIBLE} AND lower(trim(t.genre)) = lower($5)"
    );
    bind_viewer!(sqlx::query_as::<_, (String, String)>(&sql), viewer)
        .bind(genre)
        .fetch_all(pool)
        .await
        .context("db: album keys for genre")
}

pub async fn list_album_tracks(
    pool: &PgPool,
    viewer: Viewer,
    artist: &str,
    album: &str,
) -> anyhow::Result<Vec<MusicTrack>> {
    let sql = format!(
        "SELECT {TRACK_COLS}
         FROM music_tracks t
         WHERE {VISIBLE}
           AND COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') = $5
           AND COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}') = $6
         ORDER BY t.disc_number NULLS FIRST, t.track_number NULLS LAST, t.title"
    );
    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer)
        .bind(artist)
        .bind(album)
        .fetch_all(pool)
        .await
        .context("db: list album tracks")
}

pub async fn search_tracks(
    pool: &PgPool,
    viewer: Viewer,
    query: &str,
    limit: i64,
) -> anyhow::Result<Vec<MusicTrack>> {
    let pattern = format!("%{query}%");
    let sql = format!(
        "SELECT {TRACK_COLS}
         FROM music_tracks t
         WHERE {VISIBLE}
           AND (t.title ILIKE $5 OR t.artist ILIKE $5 OR t.album ILIKE $5)
         ORDER BY t.title
         LIMIT $6"
    );
    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer)
        .bind(&pattern)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: search music tracks")
}

/// Content type and byte size for many audio objects at once, keyed by
/// object id — fills the Subsonic song `contentType`/`suffix`/`size`/`bitRate`
/// fields.
///
/// Deliberately bulk: these fields are needed for every song in a response,
/// and looking them up one at a time turned a 500-track album list into 500
/// round trips.
pub async fn media_info_for(
    pool: &PgPool,
    object_ids: &[Uuid],
) -> anyhow::Result<std::collections::HashMap<Uuid, (String, Option<i64>)>> {
    if object_ids.is_empty() {
        return Ok(std::collections::HashMap::new());
    }
    let rows = sqlx::query_as::<_, (Uuid, String, Option<i64>)>(
        "SELECT id, content_type, size_bytes FROM media_objects WHERE id = ANY($1)",
    )
    .bind(object_ids)
    .fetch_all(pool)
    .await
    .context("db: media info for objects")?;

    Ok(rows
        .into_iter()
        .map(|(id, content_type, size)| (id, (content_type, size)))
        .collect())
}

/// `media_objects.content_type` for a track's audio object — used to fill
/// in Subsonic song `contentType`/`suffix` fields.
pub async fn media_content_type(pool: &PgPool, object_id: Uuid) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT content_type FROM media_objects WHERE id = $1")
        .bind(object_id)
        .fetch_optional(pool)
        .await
        .context("db: media object content type")
}

/// The storage object key for a track's audio object (used to build
/// presigned stream URLs for `/rest/stream`).
pub async fn media_object_key(pool: &PgPool, object_id: Uuid) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar::<_, String>("SELECT object_key FROM media_objects WHERE id = $1")
        .bind(object_id)
        .fetch_optional(pool)
        .await
        .context("db: media object key")
}

/// (object_key, content_type) for a media object — used to proxy cover art
/// bytes for `getCoverArt`.
pub async fn media_object_key_and_type(
    pool: &PgPool,
    object_id: Uuid,
) -> anyhow::Result<Option<(String, String)>> {
    sqlx::query_as::<_, (String, String)>(
        "SELECT object_key, content_type FROM media_objects WHERE id = $1",
    )
    .bind(object_id)
    .fetch_optional(pool)
    .await
    .context("db: media object key and content type")
}

/// Any one track's `cover_object_id` for a given (artist, album) or artist
/// bucket, used to satisfy `getCoverArt` for synthetic album/artist ids.
pub async fn representative_cover(
    pool: &PgPool,
    viewer: Viewer,
    artist: &str,
    album: Option<&str>,
) -> anyhow::Result<Option<Uuid>> {
    // cover_object_id IS NOT NULL in the WHERE clause guarantees a non-null
    // value whenever a row is returned, so decoding straight into `Uuid` is safe.
    if let Some(album) = album {
        let sql = format!(
            "SELECT t.cover_object_id FROM music_tracks t
             WHERE {VISIBLE}
               AND COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') = $5
               AND COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}') = $6
               AND t.cover_object_id IS NOT NULL
             LIMIT 1"
        );
        bind_viewer!(sqlx::query_scalar::<_, Uuid>(&sql), viewer)
            .bind(artist)
            .bind(album)
            .fetch_optional(pool)
            .await
            .context("db: representative album cover")
    } else {
        let sql = format!(
            "SELECT t.cover_object_id FROM music_tracks t
             WHERE {VISIBLE}
               AND COALESCE(NULLIF(trim(t.artist), ''), '{UNKNOWN_ARTIST}') = $5
               AND t.cover_object_id IS NOT NULL
             LIMIT 1"
        );
        bind_viewer!(sqlx::query_scalar::<_, Uuid>(&sql), viewer)
            .bind(artist)
            .fetch_optional(pool)
            .await
            .context("db: representative artist cover")
    }
}

/// Genres with both counts Subsonic's `getGenres` asks for.
///
/// `db::music::list_genres` returns only a track count; the protocol wants an
/// album count alongside it, and computing that from the album list would mean
/// loading every album just to group them again.
pub async fn list_genres_with_counts(
    pool: &PgPool,
    viewer: Viewer,
) -> anyhow::Result<Vec<(String, i64, i64)>> {
    let sql = format!(
        "SELECT trim(t.genre) AS genre,
                COUNT(*) AS song_count,
                COUNT(DISTINCT COALESCE(NULLIF(trim(t.album), ''), '{UNKNOWN_ALBUM}')) AS album_count
         FROM music_tracks t
         WHERE {VISIBLE} AND NULLIF(trim(t.genre), '') IS NOT NULL
         GROUP BY 1
         ORDER BY 1"
    );
    bind_viewer!(sqlx::query_as::<_, (String, i64, i64)>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list genres with counts")
}
