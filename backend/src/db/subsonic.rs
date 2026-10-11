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
// Artists and albums are the entities of migration 0100; their ids are the
// Subsonic ids (docs/data-model-plan.md, E2).
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

/// An artist as Subsonic lists it: a contributor credited on a visible track
/// or album (`db::music::list_artists`), with the albums credited to it.
#[derive(sqlx::FromRow)]
pub struct ArtistRow {
    pub id: Uuid,
    pub artist: String,
    pub album_count: i64,
}

pub async fn list_distinct_artists(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<ArtistRow>> {
    Ok(crate::db::music::list_artists(pool, viewer)
        .await?
        .into_iter()
        .map(|(id, artist, album_count, _)| ArtistRow { id, artist, album_count })
        .collect())
}

/// An album entity with its visible tracks' totals.
#[derive(sqlx::FromRow)]
pub struct AlbumRow {
    pub id: Uuid,
    pub artist_id: Option<Uuid>,
    pub artist: String,
    pub album: String,
    pub song_count: i64,
    pub duration_secs: Option<i64>,
    pub cover_object_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}

/// The album columns over `albums a` joined to its visible tracks `t`; the
/// caller adds conditions after `{VISIBLE}` and groups by `a.id`.
const ALBUM_ROW_COLS: &str = "a.id,
       (SELECT c.contributor_id FROM credits c
         WHERE c.item_kind = 'album' AND c.item_id = a.id AND c.role = 'artist'
         ORDER BY c.position LIMIT 1) AS artist_id,
       a.artist_display AS artist, a.title AS album,
       COUNT(*) AS song_count,
       SUM(t.duration_secs)::bigint AS duration_secs,
       (array_agg(t.cover_object_id ORDER BY (t.cover_object_id IS NULL), t.created_at))[1] AS cover_object_id,
       MIN(t.created_at) AS created_at";

pub async fn list_distinct_albums(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<AlbumRow>> {
    let sql = format!(
        "SELECT {ALBUM_ROW_COLS}
         FROM music_tracks t JOIN albums a ON a.id = t.album_id
         WHERE {VISIBLE}
         GROUP BY a.id
         ORDER BY MIN(t.created_at) DESC, a.id"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: list distinct albums")
}

/// One album by id, if the viewer can see at least one of its tracks.
pub async fn find_album(pool: &PgPool, viewer: Viewer, album_id: Uuid) -> anyhow::Result<Option<AlbumRow>> {
    let sql = format!(
        "SELECT {ALBUM_ROW_COLS}
         FROM music_tracks t JOIN albums a ON a.id = t.album_id
         WHERE {VISIBLE} AND a.id = $5
         GROUP BY a.id"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(album_id)
        .fetch_optional(pool)
        .await
        .context("db: find album")
}

/// The albums credited to a contributor, newest first.
pub async fn list_artist_albums(pool: &PgPool, viewer: Viewer, contributor_id: Uuid) -> anyhow::Result<Vec<AlbumRow>> {
    let sql = format!(
        "SELECT {ALBUM_ROW_COLS}
         FROM music_tracks t JOIN albums a ON a.id = t.album_id
         WHERE {VISIBLE}
           AND EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'album' AND c.item_id = a.id AND c.contributor_id = $5)
         GROUP BY a.id
         ORDER BY MIN(t.created_at) DESC, a.id"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumRow>(&sql), viewer)
        .bind(contributor_id)
        .fetch_all(pool)
        .await
        .context("db: list artist albums")
}

/// A contributor's name, if it is credited on a track or album the viewer can see.
pub async fn find_artist(pool: &PgPool, viewer: Viewer, contributor_id: Uuid) -> anyhow::Result<Option<String>> {
    let sql = format!(
        "SELECT k.name FROM contributors k
         WHERE k.id = $5 AND EXISTS (
             SELECT 1 FROM music_tracks t
              WHERE {VISIBLE}
                AND (EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'track' AND c.item_id = t.id AND c.contributor_id = k.id)
                     OR EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'album' AND c.item_id = t.album_id AND c.contributor_id = k.id)))"
    );
    bind_viewer!(sqlx::query_scalar::<_, String>(&sql), viewer)
        .bind(contributor_id)
        .fetch_optional(pool)
        .await
        .context("db: find artist")
}

/// Each track's first credited artist, for a song's `artistId`.
pub async fn track_artist_ids(pool: &PgPool, track_ids: &[Uuid]) -> anyhow::Result<Vec<(Uuid, Uuid)>> {
    if track_ids.is_empty() {
        return Ok(Vec::new());
    }
    sqlx::query_as::<_, (Uuid, Uuid)>(
        "SELECT DISTINCT ON (c.item_id) c.item_id, c.contributor_id
           FROM credits c
          WHERE c.item_kind = 'track' AND c.item_id = ANY($1) AND c.role = 'artist'
          ORDER BY c.item_id, c.position",
    )
    .bind(track_ids)
    .fetch_all(pool)
    .await
    .context("db: track artist ids")
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
    pub album_id: Uuid,
    pub play_count: i64,
    pub last_played: Option<DateTime<Utc>>,
}

pub async fn album_play_stats(
    pool: &PgPool,
    viewer: Viewer,
) -> anyhow::Result<Vec<AlbumPlayStats>> {
    let sql = format!(
        "SELECT t.album_id,
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
         WHERE {VISIBLE} AND t.album_id IS NOT NULL
         GROUP BY t.album_id"
    );
    bind_viewer!(sqlx::query_as::<_, AlbumPlayStats>(&sql), viewer)
        .fetch_all(pool)
        .await
        .context("db: album play stats")
}

/// Every album holding at least one visible track of `genre`, for
/// `getAlbumList&type=byGenre`. Case-insensitive, by the genre entities
/// (a track tagged "Rock; Pop" is in both).
pub async fn album_ids_for_genre(
    pool: &PgPool,
    viewer: Viewer,
    genre: &str,
) -> anyhow::Result<Vec<Uuid>> {
    let sql = format!(
        "SELECT DISTINCT t.album_id
         FROM music_tracks t
         JOIN track_genres tg ON tg.track_id = t.id
         JOIN genres g ON g.id = tg.genre_id
         WHERE {VISIBLE} AND g.key = lower(trim($5)) AND t.album_id IS NOT NULL"
    );
    bind_viewer!(sqlx::query_scalar::<_, Uuid>(&sql), viewer)
        .bind(genre)
        .fetch_all(pool)
        .await
        .context("db: albums for genre")
}

pub async fn list_album_tracks(
    pool: &PgPool,
    viewer: Viewer,
    album_id: Uuid,
) -> anyhow::Result<Vec<MusicTrack>> {
    let sql = format!(
        "SELECT {TRACK_COLS}
         FROM music_tracks t
         WHERE {VISIBLE} AND t.album_id = $5
         ORDER BY t.disc_number NULLS FIRST, t.track_number NULLS LAST, t.title"
    );
    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer)
        .bind(album_id)
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

/// A cover for an album id (any of its visible tracks' covers) or an artist
/// id (one of the artist's albums'), for `getCoverArt`.
pub async fn representative_cover(
    pool: &PgPool,
    viewer: Viewer,
    album_id: Option<Uuid>,
    contributor_id: Option<Uuid>,
) -> anyhow::Result<Option<Uuid>> {
    // cover_object_id IS NOT NULL in the WHERE clause guarantees a non-null
    // value whenever a row is returned, so decoding straight into `Uuid` is safe.
    let sql = format!(
        "SELECT t.cover_object_id FROM music_tracks t
         WHERE {VISIBLE} AND t.cover_object_id IS NOT NULL
           AND ($5::uuid IS NULL OR t.album_id = $5)
           AND ($6::uuid IS NULL OR EXISTS (
                SELECT 1 FROM credits c WHERE c.item_kind = 'album' AND c.item_id = t.album_id AND c.contributor_id = $6))
         LIMIT 1"
    );
    bind_viewer!(sqlx::query_scalar::<_, Uuid>(&sql), viewer)
        .bind(album_id)
        .bind(contributor_id)
        .fetch_optional(pool)
        .await
        .context("db: representative cover")
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
                COUNT(DISTINCT t.album_id) AS album_count
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
