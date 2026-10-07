// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::db::access::{MUSIC, VISIBLE, Viewer};
use crate::music::models::{MusicPlaylist, MusicPlaylistTrack, MusicProgress, MusicTrack};
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres};
use uuid::Uuid;

pub(crate) const TRACK_COLS: &str =
    "id, user_id, family_id, title, artist, album, genre, track_number,
     duration_secs, cover_object_id, audio_object_id,
     musicbrainz_recording_id, musicbrainz_release_id, musicbrainz_artist_id,
     album_artist, is_compilation, musicbrainz_release_group_id,
     lyrics, created_at, updated_at, disc_number, disc_total, source";

/// Same columns as `TRACK_COLS`, `t.`-qualified plus the `media_objects` join columns —
/// `TRACK_COLS` itself can't be reused here: it's also spliced into an `INSERT ... RETURNING`
/// (no table alias is valid there), and its bare `id` would be ambiguous once joined against
/// `media_objects`, which has its own `id` column (mirror-plan B-2).
pub(crate) const TRACK_COLS_WITH_CHECKSUM: &str =
    "t.id, t.user_id, t.family_id, t.title, t.artist, t.album, t.genre, t.track_number,
     t.duration_secs, t.cover_object_id, t.audio_object_id,
     t.musicbrainz_recording_id, t.musicbrainz_release_id, t.musicbrainz_artist_id,
     t.album_artist, t.is_compilation, t.musicbrainz_release_group_id,
     t.lyrics, t.created_at, t.updated_at, t.disc_number, t.disc_total, t.source, mo.size_bytes, mo.sha256";

/// Record which disc a track is on, as read from its file; marks it read either way.
pub async fn set_disc<'e, E: sqlx::PgExecutor<'e>>(
    executor: E,
    id: Uuid,
    disc_number: Option<i32>,
    disc_total: Option<i32>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_tracks_all
            SET disc_number = COALESCE($2, disc_number), disc_total = COALESCE($3, disc_total),
                disc_checked = true,
                -- Delta clients pick a real change up; a read that found nothing is none.
                updated_at = CASE WHEN COALESCE($2, disc_number) IS DISTINCT FROM disc_number
                                    OR COALESCE($3, disc_total) IS DISTINCT FROM disc_total
                                  THEN now() ELSE updated_at END
          WHERE id = $1",
    )
    .bind(id)
    .bind(disc_number.filter(|n| *n > 0))
    .bind(disc_total.filter(|n| *n > 0))
    .execute(executor)
    .await
    .context("db: set disc")?;
    Ok(())
}

/// The owner's tracks whose file was never read for a disc number.
pub async fn tracks_without_disc(pool: &PgPool, user_id: Uuid, limit: i64) -> anyhow::Result<(Vec<(Uuid, String)>, i64)> {
    let rows: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT t.id, mo.object_key FROM music_tracks t JOIN media_objects mo ON mo.id = t.audio_object_id
          WHERE t.user_id = $1 AND NOT t.disc_checked ORDER BY t.created_at LIMIT $2",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: tracks without disc")?;
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM music_tracks WHERE user_id = $1 AND NOT disc_checked")
        .bind(user_id)
        .fetch_one(pool)
        .await
        .context("db: count tracks without disc")?;
    Ok((rows, left))
}

/// Bind the four leading parameters every [`VISIBLE`] query expects.
macro_rules! bind_viewer {
    ($q:expr, $viewer:expr, $kind:expr) => {
        $q.bind($viewer.user_id)
            .bind($viewer.family_id)
            .bind($viewer.is_family_admin)
            .bind($kind)
    };
}

// ── Tracks ─────────────────────────────────────────────────────────────────
//
// Read paths (`list_tracks`, `find_track`) return anything the viewer may
// play: their own content plus family-shared content they are allowed.
// Write paths take `_owned` variants — only the owner may edit, delete, or
// change the visibility of an item.

/// One track in a duplicate group — DEDUPLICATION_PLAN.md P3's evidence. Not `MusicTrack` with
/// extras bolted on: sqlx has no `#[sqlx(flatten)]`, so a flat struct mapped straight off the
/// query's own column list is simpler than fighting that. Carries the fields a reviewer needs
/// to tell two copies apart and decide which to keep — the shared hash they grouped on, the
/// object's size/type, how many playlists reference this exact copy, and whether it's starred.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct DuplicateTrackRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub genre: Option<String>,
    pub track_number: Option<i32>,
    pub duration_secs: Option<i32>,
    pub cover_object_id: Option<Uuid>,
    pub musicbrainz_recording_id: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub sha256: String,
    pub size_bytes: Option<i64>,
    pub content_type: String,
    pub playlist_count: i64,
    pub is_starred: bool,
}

/// Every one of the caller's own tracks that shares a `sha256` with at least one other of
/// their own tracks — DEDUPLICATION_PLAN.md P3's Tier 1 (byte-identical) evidence. Rows arrive
/// ordered by hash then upload date, so the caller can build groups with one linear pass.
///
/// Deliberately scoped to `user_id = $1` throughout, no family-visibility join: two family
/// members each owning a copy of the same song is their own storage choice, not a duplicate to
/// flag (DEDUPLICATION_PLAN.md's own "cross-member deduplication" exclusion) — this must never
/// widen to `VISIBLE`/`Viewer` the way the read paths above do.
pub async fn duplicate_tracks(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<DuplicateTrackRow>> {
    sqlx::query_as::<_, DuplicateTrackRow>(
        "WITH dup_hashes AS (
             SELECT mo.sha256
             FROM music_tracks t
             JOIN media_objects mo ON mo.id = t.audio_object_id
             WHERE t.user_id = $1 AND mo.sha256 IS NOT NULL
             GROUP BY mo.sha256
             HAVING COUNT(*) > 1
         )
         SELECT
             t.id, t.user_id, t.family_id, t.title, t.artist, t.album, t.genre, t.track_number,
             t.duration_secs, t.cover_object_id, t.musicbrainz_recording_id,
             t.created_at, t.updated_at,
             mo.sha256, mo.size_bytes, mo.content_type,
             (SELECT COUNT(*) FROM music_playlist_tracks pt WHERE pt.track_id = t.id) AS playlist_count,
             EXISTS(SELECT 1 FROM music_track_stars s WHERE s.track_id = t.id AND s.user_id = t.user_id) AS is_starred
         FROM music_tracks t
         JOIN media_objects mo ON mo.id = t.audio_object_id
         JOIN dup_hashes dh ON dh.sha256 = mo.sha256
         WHERE t.user_id = $1
         ORDER BY mo.sha256, t.created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: duplicate tracks")
}

/// Unlike `list_tracks_by_genre`/playlist queries below, this joins `media_objects` for
/// `size_bytes`/`sha256` (mirror-plan B-2) — it feeds the public `TrackResponse` a client's
/// download/verification code reads, where the other `TRACK_COLS` call sites don't need it.
pub async fn list_tracks(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<MusicTrack>> {
    let sql = format!(
        "SELECT {TRACK_COLS_WITH_CHECKSUM}
         FROM music_tracks t
         JOIN media_objects mo ON mo.id = t.audio_object_id
         WHERE {VISIBLE} ORDER BY t.title"
    );

    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer, MUSIC)
        .fetch_all(pool)
        .await
        .context("db: list music tracks")
}

/// `list_tracks`, row by row: sends each track as PostgreSQL returns it, so a
/// caller can stream the list out without holding the catalog in memory
/// (docs/CAPACITY.md). Stops at the first error, or when the receiver is gone.
pub async fn send_tracks(pool: PgPool, viewer: Viewer, out: tokio::sync::mpsc::Sender<anyhow::Result<MusicTrack>>) {
    let sql = format!(
        "SELECT {TRACK_COLS_WITH_CHECKSUM}
         FROM music_tracks t
         JOIN media_objects mo ON mo.id = t.audio_object_id
         WHERE {VISIBLE} ORDER BY t.title"
    );
    let rows = bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer, MUSIC).fetch(&pool);
    crate::http::json_stream::send_all(rows, out, "db: list music tracks").await;
}

pub async fn find_track(
    pool: &PgPool,
    id: Uuid,
    viewer: Viewer,
) -> anyhow::Result<Option<MusicTrack>> {
    let sql = format!(
        "SELECT {TRACK_COLS_WITH_CHECKSUM}
         FROM music_tracks t
         JOIN media_objects mo ON mo.id = t.audio_object_id
         WHERE t.id = $5 AND {VISIBLE}"
    );

    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer, MUSIC)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find music track")
}

/// Owner-scoped lookup for mutations.
pub async fn find_track_owned(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<MusicTrack>> {
    sqlx::query_as::<_, MusicTrack>(&format!(
        "SELECT {TRACK_COLS} FROM music_tracks WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find owned music track")
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_track<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    executor: E,
    user_id: Uuid,
    family_id: Option<Uuid>,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    album_artist: Option<&str>,
    is_compilation: bool,
    genre: Option<&str>,
    track_number: Option<i32>,
    duration_secs: Option<i32>,
    audio_object_id: Uuid,
) -> anyhow::Result<MusicTrack> {
    sqlx::query_as::<_, MusicTrack>(&format!(
        "INSERT INTO music_tracks (user_id, family_id, title, artist, album, genre, track_number, duration_secs,
                                   audio_object_id, album_artist, is_compilation)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
         RETURNING {TRACK_COLS}"
    ))
    .bind(user_id)
    .bind(family_id)
    .bind(title)
    .bind(artist)
    .bind(album)
    .bind(genre)
    .bind(track_number)
    .bind(duration_secs)
    .bind(audio_object_id)
    .bind(album_artist)
    .bind(is_compilation)
    .fetch_one(executor)
    .await
    .context("db: insert music track")
}

/// `album_artist`: `None` keeps whatever is stored — so an old client that doesn't know the
/// field can't wipe a tag or MusicBrainz value — `Some(None)` clears it back to derived, and
/// `Some(Some(v))` sets it.
#[allow(clippy::too_many_arguments)]
pub async fn update_track(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    album_artist: Option<Option<&str>>,
    genre: Option<&str>,
    track_number: Option<i32>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_tracks
         SET title = $3, artist = $4, album = $5, genre = $6, track_number = $7,
             album_artist = CASE WHEN $8 THEN $9 ELSE album_artist END,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(artist)
    .bind(album)
    .bind(genre)
    .bind(track_number)
    .bind(album_artist.is_some())
    .bind(album_artist.flatten())
    .execute(pool)
    .await
    .context("db: update music track")?;
    Ok(())
}

/// Applies a chosen MusicBrainz recording to a track — updates the same
/// title/artist/album/genre/track_number fields a manual edit would, plus
/// records the MBIDs so a re-identify can tell this track was already
/// matched. Separate from [`update_track`] because a manual edit afterwards
/// should not be required to carry the MBIDs forward or clear them.
#[allow(clippy::too_many_arguments)]
pub async fn apply_musicbrainz_metadata(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    title: &str,
    artist: Option<&str>,
    album: Option<&str>,
    genre: Option<&str>,
    track_number: Option<i32>,
    mb_recording_id: &str,
    mb_release_id: Option<&str>,
    mb_artist_id: Option<&str>,
    album_artist: Option<&str>,
    mb_release_group_id: Option<&str>,
) -> anyhow::Result<()> {
    // The release's artist credit becomes the album artist (docs/album-artist-plan.md rule 1).
    // When the service didn't send one the stored value stays — rule 3 still derives it.
    sqlx::query(
        "UPDATE music_tracks
         SET title = $3, artist = $4, album = $5, genre = $6, track_number = $7,
             musicbrainz_recording_id = $8, musicbrainz_release_id = $9,
             musicbrainz_artist_id = $10,
             album_artist = COALESCE($11, album_artist),
             musicbrainz_release_group_id = COALESCE($12, musicbrainz_release_group_id),
             release_checked_at = CASE WHEN $12 IS NULL THEN release_checked_at ELSE CURRENT_TIMESTAMP END,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(title)
    .bind(artist)
    .bind(album)
    .bind(genre)
    .bind(track_number)
    .bind(mb_recording_id)
    .bind(mb_release_id)
    .bind(mb_artist_id)
    .bind(album_artist)
    .bind(mb_release_group_id)
    .execute(pool)
    .await
    .context("db: apply musicbrainz metadata to music track")?;
    Ok(())
}

pub async fn update_track_cover<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    executor: E,
    id: Uuid,
    user_id: Uuid,
    cover_object_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_tracks SET cover_object_id = $3, updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(cover_object_id)
    .execute(executor)
    .await
    .context("db: update music track cover")?;
    Ok(())
}

/// Caches a lazily-parsed lyrics result — `lyrics` is `""` (not skipped) when
/// the file had no embedded lyrics tag, so a repeat request doesn't re-fetch
/// and re-parse the audio file just to learn the same negative result again.
/// No owner scoping: this is a passive derived value from the file's own
/// bytes, not a user edit, and the caller (`get_track_lyrics`) already did
/// the viewer-visibility check before parsing.
/// Lyrics the user typed or pasted. Marked as theirs so nothing derived from
/// the file can replace them later.
pub async fn set_user_track_lyrics(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    lyrics: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_tracks
         SET lyrics = $3, lyrics_source = CASE WHEN $3 = '' THEN NULL ELSE 'user' END,
             updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(lyrics)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_track_lyrics(pool: &PgPool, id: Uuid, lyrics: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE music_tracks SET lyrics = $2 WHERE id = $1")
        .bind(id)
        .bind(lyrics)
        .execute(pool)
        .await
        .context("db: cache music track lyrics")?;
    Ok(())
}

// ── Grouped browsing ───────────────────────────────────────────────────────
//
// `music_tracks` denormalizes artist/album as text, so these aggregate on the
// fly. Blank values collapse into "Unknown …" buckets rather than vanishing.

/// Which artist a track's album is filed under (docs/album-artist-plan.md §2.1): the explicit
/// `album_artist`, else the track artist without its guests. The SQL twin of
/// `MusicTrack::effective_album_artist` / `music::models::primary_artist` — same pattern, and
/// the two must agree or an album screen lists different tracks than the album list counted.
/// Blank on both sides collapses into "Unknown Artist", like the other "Unknown …" buckets.
///
/// Stored as a generated column since migration 0089, which holds the
/// expression; computing it per row per query was most of an album list's time.
pub(crate) const ALBUM_ARTIST_SQL: &str = "t.album_artist_key";

/// The track's own artist without its guests — rule 3 alone, ignoring any album artist.
/// A generated column since migration 0089.
const PRIMARY_ARTIST_SQL: &str = "t.primary_artist_key";

/// Everyone with an album of their own, plus everyone who only appears on someone else's —
/// a singer whose one track sits on a "Various Artists" compilation. Grouping by album artist
/// alone dropped those from the artist list entirely. `album_count` counts only albums filed
/// under the artist; `track_count` counts their own tracks and those on their albums.
pub async fn list_artists(
    pool: &PgPool,
    viewer: Viewer,
) -> anyhow::Result<Vec<(String, i64, i64)>> {
    let sql = format!(
        "SELECT name AS artist,
                COUNT(DISTINCT album) FILTER (WHERE files_album) AS album_count,
                COUNT(DISTINCT track_id) AS track_count
         FROM (
             SELECT t.id AS track_id, {ALBUM_ARTIST_SQL} AS name,
                    t.album_key AS album, TRUE AS files_album
             FROM music_tracks t
             WHERE {VISIBLE}
             UNION ALL
             SELECT t.id, {PRIMARY_ARTIST_SQL}, NULL, FALSE
             FROM music_tracks t
             WHERE {VISIBLE} AND {PRIMARY_ARTIST_SQL} <> {ALBUM_ARTIST_SQL}
         ) named
         GROUP BY name
         ORDER BY name"
    );

    bind_viewer!(sqlx::query_as::<_, (String, i64, i64)>(&sql), viewer, MUSIC)
        .fetch_all(pool)
        .await
        .context("db: list artists")
}

pub async fn list_albums(
    pool: &PgPool,
    viewer: Viewer,
    artist: Option<&str>,
) -> anyhow::Result<Vec<(String, String, i64, Option<i64>, Option<Uuid>)>> {
    let sql = format!(
        "SELECT {ALBUM_ARTIST_SQL} AS artist,
                t.album_key AS album,
                COUNT(*) AS track_count,
                SUM(t.duration_secs)::bigint AS duration_secs,
                -- NULL when no track in the album has art at all: handing back a track id
                -- regardless made every coverless album fetch a cover and get a 404.
                (array_agg(t.id ORDER BY t.created_at)
                     FILTER (WHERE t.cover_object_id IS NOT NULL))[1] AS cover_track
         FROM music_tracks t
         WHERE {VISIBLE}
           AND ($5::text IS NULL OR {ALBUM_ARTIST_SQL} = $5)
         GROUP BY 1, 2
         ORDER BY 1, 2"
    );

    bind_viewer!(
        sqlx::query_as::<_, (String, String, i64, Option<i64>, Option<Uuid>)>(&sql),
        viewer,
        MUSIC
    )
    .bind(artist)
    .fetch_all(pool)
    .await
    .context("db: list albums")
}

pub async fn list_genres(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<(String, i64)>> {
    let sql = format!(
        "SELECT trim(t.genre) AS genre, COUNT(*) AS track_count
         FROM music_tracks t
         WHERE {VISIBLE} AND NULLIF(trim(t.genre), '') IS NOT NULL
         GROUP BY 1
         ORDER BY 1"
    );

    bind_viewer!(sqlx::query_as::<_, (String, i64)>(&sql), viewer, MUSIC)
        .fetch_all(pool)
        .await
        .context("db: list genres")
}

// ── Playlists ──────────────────────────────────────────────────────────────

const PLAYLIST_COLS: &str =
    "id, user_id, family_id, name, description, cover_object_id, created_at, updated_at, generated_at, kept_at";

// Playlists are shareable but not individually grantable: they evaluate
// against the viewer's `music` policy (see db::access::table_for_kind).

pub async fn list_playlists(pool: &PgPool, viewer: Viewer) -> anyhow::Result<Vec<MusicPlaylist>> {
    let sql =
        format!("SELECT {PLAYLIST_COLS} FROM music_playlists t WHERE {VISIBLE} ORDER BY name");

    bind_viewer!(sqlx::query_as::<_, MusicPlaylist>(&sql), viewer, MUSIC)
        .fetch_all(pool)
        .await
        .context("db: list playlists")
}

pub async fn find_playlist(
    pool: &PgPool,
    id: Uuid,
    viewer: Viewer,
) -> anyhow::Result<Option<MusicPlaylist>> {
    let sql =
        format!("SELECT {PLAYLIST_COLS} FROM music_playlists t WHERE t.id = $5 AND {VISIBLE}");

    bind_viewer!(sqlx::query_as::<_, MusicPlaylist>(&sql), viewer, MUSIC)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find playlist")
}

/// Owner-scoped lookup for mutations.
pub async fn find_playlist_owned(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
) -> anyhow::Result<Option<MusicPlaylist>> {
    sqlx::query_as::<_, MusicPlaylist>(&format!(
        "SELECT {PLAYLIST_COLS} FROM music_playlists WHERE id = $1 AND user_id = $2"
    ))
    .bind(id)
    .bind(user_id)
    .fetch_optional(pool)
    .await
    .context("db: find owned playlist")
}

pub async fn insert_playlist(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Option<Uuid>,
    name: &str,
    description: Option<&str>,
    generated: bool,
) -> anyhow::Result<MusicPlaylist> {
    sqlx::query_as::<_, MusicPlaylist>(&format!(
        "INSERT INTO music_playlists (user_id, family_id, name, description, generated_at)
         VALUES ($1, $2, $3, $4, CASE WHEN $5 THEN now() END)
         RETURNING {PLAYLIST_COLS}"
    ))
    .bind(user_id)
    .bind(family_id)
    .bind(name)
    .bind(description)
    .bind(generated)
    .fetch_one(pool)
    .await
    .context("db: insert playlist")
}

pub async fn update_playlist(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_playlists
         SET name = $3, description = $4, updated_at = CURRENT_TIMESTAMP
         WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .execute(pool)
    .await
    .context("db: update playlist")?;
    Ok(())
}

// ── Playlist tracks ────────────────────────────────────────────────────────

pub async fn list_playlist_tracks(
    pool: &PgPool,
    playlist_id: Uuid,
) -> anyhow::Result<Vec<MusicPlaylistTrack>> {
    sqlx::query_as::<_, MusicPlaylistTrack>(
        "SELECT id, playlist_id, track_id, position, added_at
         FROM music_playlist_tracks WHERE playlist_id = $1 ORDER BY position",
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await
    .context("db: list playlist tracks")
}

pub async fn add_track_to_playlist(
    pool: &PgPool,
    playlist_id: Uuid,
    track_id: Uuid,
) -> anyhow::Result<MusicPlaylistTrack> {
    // Auto-assign next position
    sqlx::query_as::<_, MusicPlaylistTrack>(
        "INSERT INTO music_playlist_tracks (playlist_id, track_id, position)
         VALUES ($1, $2, COALESCE(
             (SELECT MAX(position) + 1 FROM music_playlist_tracks WHERE playlist_id = $1), 1
         ))
         RETURNING id, playlist_id, track_id, position, added_at",
    )
    .bind(playlist_id)
    .bind(track_id)
    .fetch_one(pool)
    .await
    .context("db: add track to playlist")
}

/// Append many tracks in the given order, in one statement. Ids the viewer may
/// not play (or that do not exist) are skipped rather than failing the whole
/// list — the same line `add_to_playlist` draws for a single track.
pub async fn add_tracks_to_playlist(
    pool: &PgPool,
    playlist_id: Uuid,
    track_ids: &[Uuid],
    viewer: Viewer,
) -> anyhow::Result<u64> {
    let sql = format!(
        "INSERT INTO music_playlist_tracks (playlist_id, track_id, position)
         SELECT $5, x.id,
                COALESCE((SELECT MAX(position) FROM music_playlist_tracks WHERE playlist_id = $5), 0)
                  + row_number() OVER (ORDER BY x.ord)
           FROM unnest($6::uuid[]) WITH ORDINALITY AS x(id, ord)
           JOIN music_tracks t ON t.id = x.id
          WHERE {VISIBLE}
          ORDER BY x.ord"
    );
    let result = bind_viewer!(sqlx::query(&sql), viewer, MUSIC)
        .bind(playlist_id)
        .bind(track_ids)
        .execute(pool)
        .await
        .context("db: add tracks to playlist")?;
    Ok(result.rows_affected())
}

/// The songs of a playlist in order, with whose they are and whether they are shared —
/// what sharing the playlist needs to know about them.
pub async fn playlist_track_owners(
    pool: &PgPool,
    playlist_id: Uuid,
) -> anyhow::Result<Vec<(Uuid, Uuid, Option<Uuid>)>> {
    sqlx::query_as::<_, (Uuid, Uuid, Option<Uuid>)>(
        "SELECT t.id, t.user_id, t.family_id
           FROM music_playlist_tracks pt
           JOIN music_tracks t ON t.id = pt.track_id
          WHERE pt.playlist_id = $1
          ORDER BY pt.position",
    )
    .bind(playlist_id)
    .fetch_all(pool)
    .await
    .context("db: playlist track owners")
}

/// Mark a generated playlist as one the owner wants to keep.
pub async fn keep_playlist(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE music_playlists SET kept_at = COALESCE(kept_at, now())
          WHERE id = $1 AND user_id = $2 AND generated_at IS NOT NULL",
    )
    .bind(id)
    .bind(user_id)
    .execute(pool)
    .await
    .context("db: keep playlist")?;
    Ok(result.rows_affected() > 0)
}

pub async fn remove_track_from_playlist(
    pool: &PgPool,
    playlist_id: Uuid,
    entry_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM music_playlist_tracks WHERE id = $1 AND playlist_id = $2")
        .bind(entry_id)
        .bind(playlist_id)
        .execute(pool)
        .await
        .context("db: remove track from playlist")?;
    Ok(())
}

pub async fn reorder_playlist_tracks(
    pool: &PgPool,
    playlist_id: Uuid,
    track_entry_ids: &[Uuid],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await.context("db: begin tx for playlist reorder")?;

    // Temporarily set positions to negative to avoid unique constraint violations
    sqlx::query("UPDATE music_playlist_tracks SET position = -position WHERE playlist_id = $1")
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .context("db: clear playlist positions")?;

    for (index, entry_id) in track_entry_ids.iter().enumerate() {
        sqlx::query(
            "UPDATE music_playlist_tracks SET position = $1 WHERE id = $2 AND playlist_id = $3",
        )
        .bind((index + 1) as i32)
        .bind(entry_id)
        .bind(playlist_id)
        .execute(&mut *tx)
        .await
        .context("db: reorder playlist track")?;
    }

    tx.commit().await.context("db: commit playlist reorder")?;
    Ok(())
}

// ── Music progress ─────────────────────────────────────────────────────────

pub async fn get_track_progress(
    pool: &PgPool,
    user_id: Uuid,
    track_id: Uuid,
) -> anyhow::Result<Option<MusicProgress>> {
    sqlx::query_as::<_, MusicProgress>(
        "SELECT user_id, track_id, position_secs, completed, updated_at
         FROM music_progress WHERE user_id = $1 AND track_id = $2",
    )
    .bind(user_id)
    .bind(track_id)
    .fetch_optional(pool)
    .await
    .context("db: get music progress")
}

pub async fn upsert_track_progress(
    pool: &PgPool,
    user_id: Uuid,
    track_id: Uuid,
    position_secs: f64,
    completed: bool,
) -> anyhow::Result<MusicProgress> {
    sqlx::query_as::<_, MusicProgress>(
        "INSERT INTO music_progress (user_id, track_id, position_secs, completed)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, track_id) DO UPDATE
             SET position_secs = EXCLUDED.position_secs,
                 completed     = EXCLUDED.completed,
                 updated_at    = CURRENT_TIMESTAMP
         RETURNING user_id, track_id, position_secs, completed, updated_at",
    )
    .bind(user_id)
    .bind(track_id)
    .bind(position_secs)
    .bind(completed)
    .fetch_one(pool)
    .await
    .context("db: upsert music progress")
}

// ── Artist images ─────────────────────────────────────────────────────────

/// Three-state, which is the point: `None` means never looked up, `Some(None)`
/// means looked up and no source had it, `Some(Some(..))` means we have one.
/// Collapsing the first two would make every unknown artist re-hit the network
/// on every render.
///
/// A remembered miss expires after [`MISS_TTL_DAYS`]; a picture we hold does
/// not. Commons gains photographs of long-tail artists all the time, and rows
/// written before misses and outages were told apart are wrong to keep.
/// How long "this artist has no picture" is believed for.
const MISS_TTL_DAYS: i32 = 30;

pub async fn find_artist_image(
    pool: &PgPool,
    user_id: Uuid,
    artist: &str,
) -> anyhow::Result<Option<Option<(String, String)>>> {
    let row = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT mo.object_key, mo.content_type
         FROM music_artist_images ai
         LEFT JOIN media_objects mo ON mo.id = ai.image_object_id
         WHERE ai.user_id = $1 AND lower(ai.artist_name) = lower($2)
           AND (ai.image_object_id IS NOT NULL
                OR ai.is_user_set
                OR ai.fetched_at > now() - make_interval(days => $3))",
    )
    .bind(user_id)
    .bind(artist)
    .bind(MISS_TTL_DAYS)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(|(key, content_type)| match (key, content_type) {
        (Some(key), Some(content_type)) => Some((key, content_type)),
        _ => None,
    }))
}

pub async fn upsert_artist_image(
    pool: &PgPool,
    user_id: Uuid,
    artist: &str,
    image_object_id: Option<Uuid>,
) -> anyhow::Result<()> {
    upsert_artist_image_tx(pool, user_id, artist, image_object_id).await
}

/// The automatic path. **Never overwrites a user-set image** — the `WHERE NOT
/// is_user_set` on the update is what makes a deliberate choice permanent
/// rather than something the next catalogue lookup silently replaces.
pub async fn upsert_artist_image_tx<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    executor: E,
    user_id: Uuid,
    artist: &str,
    image_object_id: Option<Uuid>,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO music_artist_images (user_id, artist_name, image_object_id, fetched_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (user_id, artist_name)
         DO UPDATE SET image_object_id = EXCLUDED.image_object_id, fetched_at = now()
         WHERE NOT music_artist_images.is_user_set",
    )
    .bind(user_id)
    .bind(artist)
    .bind(image_object_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Any MusicBrainz artist id recorded on this user's tracks by this artist.
///
/// Artists have no row of their own, so identity has to be borrowed from the
/// tracks that name them — which is also why this can legitimately return
/// nothing for an artist whose tracks were never identified.
pub async fn find_artist_mbid(
    pool: &PgPool,
    user_id: Uuid,
    artist: &str,
) -> anyhow::Result<Option<String>> {
    let row: Option<(String,)> = sqlx::query_as(
        "SELECT musicbrainz_artist_id FROM music_tracks
         WHERE user_id = $1 AND lower(artist) = lower($2) AND musicbrainz_artist_id IS NOT NULL
         LIMIT 1",
    )
    .bind(user_id)
    .bind(artist)
    .fetch_optional(pool)
    .await?;
    Ok(row.map(|(mbid,)| mbid))
}

/// The automatic path, with the attribution its licence requires. Same
/// `WHERE NOT is_user_set` guard as `upsert_artist_image_tx` — a chosen picture
/// still wins.
pub async fn upsert_fetched_artist_image<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    executor: E,
    user_id: Uuid,
    artist: &str,
    image_object_id: Option<Uuid>,
    attribution: &crate::metadata::wikimedia::Attribution,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO music_artist_images
            (user_id, artist_name, image_object_id, fetched_at,
             image_source, image_author, image_license, image_license_url, image_source_url)
         VALUES ($1, $2, $3, now(), 'Wikimedia Commons', $4, $5, $6, $7)
         ON CONFLICT (user_id, artist_name)
         DO UPDATE SET image_object_id = EXCLUDED.image_object_id, fetched_at = now(),
                       image_source = EXCLUDED.image_source, image_author = EXCLUDED.image_author,
                       image_license = EXCLUDED.image_license,
                       image_license_url = EXCLUDED.image_license_url,
                       image_source_url = EXCLUDED.image_source_url
         WHERE NOT music_artist_images.is_user_set",
    )
    .bind(user_id)
    .bind(artist)
    .bind(image_object_id)
    .bind(attribution.author.as_deref())
    .bind(attribution.license.as_deref())
    .bind(attribution.license_url.as_deref())
    .bind(&attribution.source_url)
    .execute(executor)
    .await?;
    Ok(())
}

/// What a client must display alongside the image. `None` for a user's own
/// upload, which has nothing to attribute.
pub async fn find_artist_image_attribution(
    pool: &PgPool,
    user_id: Uuid,
    artist: &str,
) -> anyhow::Result<Option<(Option<String>, Option<String>, Option<String>, Option<String>, Option<String>, bool)>> {
    Ok(sqlx::query_as(
        "SELECT image_source, image_author, image_license, image_license_url, image_source_url, is_user_set
         FROM music_artist_images
         WHERE user_id = $1 AND lower(artist_name) = lower($2) AND image_object_id IS NOT NULL",
    )
    .bind(user_id)
    .bind(artist)
    .fetch_optional(pool)
    .await?)
}

/// The user's own picture. Unconditional, unlike the automatic path above:
/// choosing one is exactly the act that should win.
pub async fn set_user_artist_image<'e, E: sqlx::Executor<'e, Database = Postgres>>(
    executor: E,
    user_id: Uuid,
    artist: &str,
    image_object_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO music_artist_images (user_id, artist_name, image_object_id, fetched_at, is_user_set)
         VALUES ($1, $2, $3, now(), TRUE)
         ON CONFLICT (user_id, artist_name)
         DO UPDATE SET image_object_id = EXCLUDED.image_object_id, fetched_at = now(), is_user_set = TRUE,
                       image_source = NULL, image_author = NULL, image_license = NULL,
                       image_license_url = NULL, image_source_url = NULL",
    )
    .bind(user_id)
    .bind(artist)
    .bind(image_object_id)
    .execute(executor)
    .await?;
    Ok(())
}

/// Drops the row entirely rather than clearing the object — a NULL row means
/// "asked, nothing found", which would wrongly suppress the automatic lookup
/// this is meant to hand control back to.
pub async fn clear_artist_image(pool: &PgPool, user_id: Uuid, artist: &str) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM music_artist_images WHERE user_id = $1 AND lower(artist_name) = lower($2)")
        .bind(user_id)
        .bind(artist)
        .execute(pool)
        .await?;
    Ok(())
}

// ── Stars, ratings, bookmarks (OpenSubsonic annotation) ────────────────────
//
// Introduced by the `/rest` surface, where favouriting and rating are
// first-class buttons in every client. Songs are keyed by id; artists and
// albums are keyed by name, because `music_tracks` stores them as
// denormalized strings with no rows to reference — the same choice
// `music_artist_images` already makes.

pub async fn star_track(pool: &PgPool, user_id: Uuid, track_id: Uuid) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO music_track_stars (user_id, track_id) VALUES ($1, $2)
         ON CONFLICT (user_id, track_id) DO NOTHING",
    )
    .bind(user_id)
    .bind(track_id)
    .execute(pool)
    .await
    .context("db: star track")?;
    Ok(())
}

// ── Smart playlist storage ─────────────────────────────────────────────────

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SmartPlaylist {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub name: String,
    pub description: Option<String>,
    pub rule: serde_json::Value,
    pub mode: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// Every smart playlist the caller can see: their own, plus any shared with
/// their family. Same visibility rule as content (0019).
pub async fn list_smart_playlists(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Uuid,
) -> anyhow::Result<Vec<SmartPlaylist>> {
    sqlx::query_as::<_, SmartPlaylist>(
        "SELECT id, user_id, family_id, name, description, rule, mode, created_at, updated_at
         FROM music_smart_playlists
         WHERE user_id = $1 OR family_id = $2
         ORDER BY updated_at DESC",
    )
    .bind(user_id)
    .bind(family_id)
    .fetch_all(pool)
    .await
    .context("db: list smart playlists")
}

pub async fn find_smart_playlist(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    family_id: Uuid,
) -> anyhow::Result<Option<SmartPlaylist>> {
    sqlx::query_as::<_, SmartPlaylist>(
        "SELECT id, user_id, family_id, name, description, rule, mode, created_at, updated_at
         FROM music_smart_playlists
         WHERE id = $1 AND (user_id = $2 OR family_id = $3)",
    )
    .bind(id)
    .bind(user_id)
    .bind(family_id)
    .fetch_optional(pool)
    .await
    .context("db: find smart playlist")
}

pub async fn create_smart_playlist(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Option<Uuid>,
    name: &str,
    description: Option<&str>,
    rule: &serde_json::Value,
) -> anyhow::Result<SmartPlaylist> {
    sqlx::query_as::<_, SmartPlaylist>(
        "INSERT INTO music_smart_playlists (user_id, family_id, name, description, rule)
         VALUES ($1, $2, $3, $4, $5)
         RETURNING id, user_id, family_id, name, description, rule, mode, created_at, updated_at",
    )
    .bind(user_id)
    .bind(family_id)
    .bind(name)
    .bind(description)
    .bind(rule)
    .fetch_one(pool)
    .await
    .context("db: create smart playlist")
}

/// Owner-scoped: sharing does not grant editing. A member who was sent a rule
/// can resolve it and copy it, not change it under the owner.
pub async fn update_smart_playlist(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    name: &str,
    description: Option<&str>,
    rule: &serde_json::Value,
) -> anyhow::Result<Option<SmartPlaylist>> {
    sqlx::query_as::<_, SmartPlaylist>(
        "UPDATE music_smart_playlists
            SET name = $3, description = $4, rule = $5, updated_at = now()
          WHERE id = $1 AND user_id = $2
      RETURNING id, user_id, family_id, name, description, rule, mode, created_at, updated_at",
    )
    .bind(id)
    .bind(user_id)
    .bind(name)
    .bind(description)
    .bind(rule)
    .fetch_optional(pool)
    .await
    .context("db: update smart playlist")
}

pub async fn delete_smart_playlist(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<bool> {
    let r = sqlx::query("DELETE FROM music_smart_playlists WHERE id = $1 AND user_id = $2")
        .bind(id)
        .bind(user_id)
        .execute(pool)
        .await
        .context("db: delete smart playlist")?;
    Ok(r.rows_affected() > 0)
}

/// Record that a smart playlist was materialised into an ordinary one.
///
/// Kept for provenance — "this came from that rule" — which is what lets a
/// client offer "refresh from the rule" later without guessing where a playlist
/// came from.
pub async fn mark_smart_playlist_frozen(
    pool: &PgPool,
    id: Uuid,
    user_id: Uuid,
    into: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_smart_playlists
            SET mode = 'frozen', frozen_into = $3, updated_at = now()
          WHERE id = $1 AND user_id = $2",
    )
    .bind(id)
    .bind(user_id)
    .bind(into)
    .execute(pool)
    .await
    .context("db: mark smart playlist frozen")?;
    Ok(())
}

// ── Smart playlist candidates ──────────────────────────────────────────────

/// Candidate tracks for a rule, already filtered and scored.
///
/// Filtering happens in SQL because the library is large; sequencing happens in
/// `music::sequence` because it is a product decision with an opinion, and one
/// copy of that in Rust beats two that can disagree.
///
/// Three things are subtracted before anything is scored, and all three are
/// per-user even though the library is shared:
///   • tracks this user banned (0069) — never in automatic selection,
///   • tracks with no audio object,
///   • anything the caller cannot see (`VISIBLE`, 0019).
pub async fn smart_playlist_candidates(
    pool: &PgPool,
    viewer: crate::db::access::Viewer,
    rule: &crate::music::rules::Rule,
    cap: i64,
) -> anyhow::Result<Vec<crate::music::sequence::Candidate>> {
    use crate::music::rules::Sort;

    let f = &rule.filters;
    let mut sql = format!(
        "SELECT t.id, COALESCE(t.artist, '') AS artist, COALESCE(t.duration_secs, 0) AS duration_secs,
                e.energy,
                COALESCE(a.weight, 1.0) AS weight,
                a.last_played_at
         FROM music_tracks t
         JOIN media_objects mo ON mo.id = t.audio_object_id
         LEFT JOIN music_track_energy   e ON e.track_id = t.id
         LEFT JOIN music_track_affinity a ON a.track_id = t.id AND a.user_id = $1
         LEFT JOIN music_track_stars    st ON st.track_id = t.id AND st.user_id = $1
         LEFT JOIN music_track_ratings  r  ON r.track_id  = t.id AND r.user_id  = $1
         WHERE {visible}
           AND NOT EXISTS (
               SELECT 1 FROM music_track_feedback fb
               WHERE fb.user_id = $1 AND fb.track_id = t.id AND fb.kind = 'banned'
           )",
        visible = crate::db::access::VISIBLE
    );

    // $1..$4 are the viewer; rule parameters start after them.
    let mut n = 4;
    let push = |sql: &mut String, clause: &str, count: usize, n: &mut i32| {
        let mut c = clause.to_string();
        for _ in 0..count {
            *n += 1;
            c = c.replacen("$?", &format!("${n}"), 1);
        }
        sql.push_str(" AND ");
        sql.push_str(&c);
    };

    if f.bpm.is_some() {
        push(&mut sql, "t.bpm BETWEEN $? AND $?", 2, &mut n);
    }
    if f.energy.is_some() {
        push(&mut sql, "e.energy BETWEEN $? AND $?", 2, &mut n);
    }
    if f.starred == Some(true) {
        sql.push_str(" AND st.user_id IS NOT NULL");
    }
    if f.min_rating.is_some() {
        push(&mut sql, "r.rating >= $?", 1, &mut n);
    }
    if f.not_played_days.is_some() {
        // "Never played" counts as "not played in N days" — that is what makes
        // "new in the library" expressible at all.
        push(
            &mut sql,
            "(a.last_played_at IS NULL OR a.last_played_at < now() - make_interval(days => $?))",
            1,
            &mut n,
        );
    }
    if f.added_days.is_some() {
        push(&mut sql, "t.created_at >= now() - make_interval(days => $?)", 1, &mut n);
    }
    if f.played_by_others_not_me == Some(true) {
        sql.push_str(
            " AND a.last_played_at IS NULL
               AND EXISTS (
                   SELECT 1 FROM listening_sessions ls
                   WHERE ls.media_kind = 'music' AND ls.item_id = t.id AND ls.user_id <> $1
               )",
        );
    }
    if let Some(g) = &f.genres {
        if !g.include.is_empty() {
            push(&mut sql, "lower(COALESCE(t.genre, '')) = ANY($?)", 1, &mut n);
        }
        if !g.exclude.is_empty() {
            push(&mut sql, "NOT (lower(COALESCE(t.genre, '')) = ANY($?))", 1, &mut n);
        }
    }
    if let Some(a) = &f.artists {
        if !a.include.is_empty() {
            push(&mut sql, "lower(COALESCE(t.artist, '')) = ANY($?)", 1, &mut n);
        }
        if !a.exclude.is_empty() {
            push(&mut sql, "NOT (lower(COALESCE(t.artist, '')) = ANY($?))", 1, &mut n);
        }
    }

    // Ordering that depends on columns only SQL has. The rest is the
    // sequencer's, and weighted-random deliberately does no ordering here.
    sql.push_str(match rule.sort {
        Sort::LeastRecentlyPlayed => " ORDER BY a.last_played_at ASC NULLS FIRST",
        Sort::Weight => " ORDER BY COALESCE(a.weight, 1.0) DESC",
        _ => " ORDER BY random()",
    });
    n += 1;
    sql.push_str(&format!(" LIMIT ${n}"));

    let mut q = bind_viewer!(
        sqlx::query_as::<_, CandidateRow>(&sql),
        viewer,
        crate::db::access::MUSIC
    );

    if let Some([lo, hi]) = f.bpm {
        q = q.bind(lo).bind(hi);
    }
    if let Some([lo, hi]) = f.energy {
        q = q.bind(lo).bind(hi);
    }
    if let Some(r) = f.min_rating {
        q = q.bind(r);
    }
    // i32, NOT f64. `make_interval(days => ...)` takes an integer; binding a
    // double makes Postgres fail to resolve the function at all, at query time,
    // as a 500.
    //
    // This slipped through twice. Postgres 17 — which the local stack runs —
    // accepts the double; production and canary run 16.14, which does not. So
    // the local verification passed against the same code that broke canary.
    // If a query works locally and 500s on the server, check the major version
    // before anything else.
    if let Some(d) = f.not_played_days {
        q = q.bind(d);
    }
    if let Some(d) = f.added_days {
        q = q.bind(d);
    }
    if let Some(g) = &f.genres {
        if !g.include.is_empty() {
            q = q.bind(lowered(&g.include));
        }
        if !g.exclude.is_empty() {
            q = q.bind(lowered(&g.exclude));
        }
    }
    if let Some(a) = &f.artists {
        if !a.include.is_empty() {
            q = q.bind(lowered(&a.include));
        }
        if !a.exclude.is_empty() {
            q = q.bind(lowered(&a.exclude));
        }
    }

    let rows = q
        .bind(cap)
        .fetch_all(pool)
        .await
        .context("db: smart playlist candidates")?;

    let now = Utc::now();
    Ok(rows
        .into_iter()
        .map(|r| crate::music::sequence::Candidate {
            track_id: r.id,
            artist: r.artist,
            duration_secs: r.duration_secs,
            energy: r.energy,
            // Rest is applied here, not baked into the stored weight: it
            // changes by the hour, and a stored copy would be wrong the moment
            // the rollup finished (see music::affinity).
            score: r.weight
                * crate::music::affinity::recency_multiplier(r.last_played_at, now) as f32,
        })
        .collect())
}

fn lowered(v: &[String]) -> Vec<String> {
    v.iter().map(|s| s.to_lowercase()).collect()
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    id: Uuid,
    artist: String,
    duration_secs: i32,
    energy: Option<f32>,
    weight: f32,
    last_played_at: Option<DateTime<Utc>>,
}

// ── Acoustic analysis ──────────────────────────────────────────────────────

/// The storage key of a track's audio object, if the track still exists.
pub async fn track_audio_key(pool: &PgPool, track_id: Uuid) -> anyhow::Result<Option<String>> {
    sqlx::query_scalar::<_, String>(
        "SELECT m.object_key
         FROM music_tracks t
         JOIN media_objects m ON m.id = t.audio_object_id
         WHERE t.id = $1",
    )
    .bind(track_id)
    .fetch_optional(pool)
    .await
    .context("db: track audio key")
}

/// Store what the extractor measured.
///
/// `raw` keeps the full output because re-measuring means fetching the object
/// out of R2 again; Postgres storage is cheap by comparison. Writing
/// `analysis_version` is what takes the track off the pending queue, so it is
/// set even when every individual measurement came back `None` — a file that
/// cannot be measured must not be retried forever.
pub async fn set_track_analysis(
    pool: &PgPool,
    track_id: Uuid,
    a: &crate::music::analysis::TrackAnalysis,
    version: i16,
) -> anyhow::Result<()> {
    let raw = serde_json::to_value(a).unwrap_or(serde_json::Value::Null);
    sqlx::query(
        "UPDATE music_tracks
            SET bpm = $2, music_key = $3, key_scale = $4, loudness_lufs = $5,
                dynamic_range = $6, spectral_centroid = $7, onset_rate = $8,
                analysis_raw = $9, analysis_version = $10, analyzed_at = now()
          WHERE id = $1",
    )
    .bind(track_id)
    .bind(a.bpm)
    .bind(a.music_key.as_deref())
    .bind(a.key_scale.as_deref())
    .bind(a.loudness_lufs)
    .bind(a.dynamic_range)
    .bind(a.spectral_centroid)
    .bind(a.onset_rate)
    .bind(raw)
    .bind(version)
    .execute(pool)
    .await
    .context("db: store track analysis")?;
    Ok(())
}

/// Tracks still needing measurement, **most-played first**.
///
/// The ordering is the difference between the feature being useful at five per
/// cent coverage and at a hundred: the tracks people actually play are the ones
/// any early playlist draws on. Cheap to do, and it means a half-finished
/// backfill already produces good results.
pub async fn tracks_pending_analysis(
    pool: &PgPool,
    version: i16,
    limit: i64,
) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT t.id
         FROM music_tracks t
         LEFT JOIN (
             SELECT item_id, COUNT(*) AS plays
             FROM listening_sessions
             WHERE media_kind = 'music'
             GROUP BY item_id
         ) p ON p.item_id = t.id
         WHERE t.audio_object_id IS NOT NULL
           AND (t.analysis_version IS NULL OR t.analysis_version < $1)
         ORDER BY COALESCE(p.plays, 0) DESC, t.created_at DESC
         LIMIT $2",
    )
    .bind(version)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: tracks pending analysis")
}

/// Whether a family asked for its library to be measured, and by whom.
pub async fn audio_analysis_optin(
    pool: &PgPool,
    family_id: Uuid,
) -> anyhow::Result<(bool, Option<DateTime<Utc>>)> {
    let row: Option<(bool, Option<DateTime<Utc>>)> = sqlx::query_as(
        "SELECT audio_analysis_enabled, audio_analysis_enabled_at
         FROM families WHERE id = $1",
    )
    .bind(family_id)
    .fetch_optional(pool)
    .await
    .context("db: read audio analysis opt-in")?;
    Ok(row.unwrap_or((false, None)))
}

/// Turn measurement on or off for a family.
///
/// Turning it off keeps every measurement already taken. Deleting them would
/// mean re-fetching the whole library if the user changed their mind, which is
/// the one genuinely expensive thing here.
pub async fn set_audio_analysis_optin(
    pool: &PgPool,
    family_id: Uuid,
    enabled: bool,
    by: Uuid,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE families
            SET audio_analysis_enabled    = $2,
                audio_analysis_enabled_at = CASE WHEN $2 THEN now() ELSE NULL END,
                audio_analysis_enabled_by = CASE WHEN $2 THEN $3::uuid ELSE NULL END,
                updated_at                = now()
          WHERE id = $1",
    )
    .bind(family_id)
    .bind(enabled)
    .bind(by)
    .execute(pool)
    .await
    .context("db: set audio analysis opt-in")?;
    Ok(())
}

/// How far a family's library has got: measured, and total worth measuring.
pub async fn audio_analysis_progress(
    pool: &PgPool,
    family_id: Uuid,
    version: i16,
) -> anyhow::Result<(i64, i64)> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT COUNT(*) FILTER (WHERE analysis_version >= $2)::bigint,
                COUNT(*)::bigint
         FROM music_tracks
         WHERE audio_object_id IS NOT NULL AND family_id = $1",
    )
    .bind(family_id)
    .bind(version)
    .fetch_one(pool)
    .await
    .context("db: audio analysis progress")?;
    Ok(row)
}

/// Enqueue measurement for a family's unmeasured tracks, **most-played first**.
///
/// The ordering is what makes the feature useful at five per cent coverage
/// instead of a hundred: the tracks people actually play are the ones any early
/// playlist draws on.
///
/// The priority is carried in `scheduled_at`, not in insertion order.
/// `db::jobs::claim_next` orders by `scheduled_at`, and a plain
/// `INSERT ... ORDER BY` gives every row the same default timestamp — which
/// makes the claim order arbitrary and throws the ranking away silently. Each
/// row is backdated by its rank in microseconds instead: all of them are
/// already in the past, so nothing is delayed, and the order survives.
///
/// Re-enabling is safe: a track already measured fails the
/// `analysis_version` filter, and one already queued fails the NOT EXISTS.
pub async fn enqueue_family_analysis(
    pool: &PgPool,
    family_id: Uuid,
    version: i16,
    limit: i64,
) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "INSERT INTO jobs (job_type, payload, status, scheduled_at)
         SELECT 'analyze_audio',
                jsonb_build_object('track_id', t.id::text),
                'pending',
                -- Window ordered ASCENDING by priority on purpose: the most
                -- played track gets the HIGHEST row number, so it is backdated
                -- FURTHEST and claim_next (ORDER BY scheduled_at) reaches it
                -- first. Ranking DESC here backdates the least played the most
                -- and runs the queue in exactly the wrong order.
                now() - make_interval(secs =>
                    (ROW_NUMBER() OVER (ORDER BY COALESCE(p.plays, 0) ASC, t.created_at ASC))
                    * 0.000001)
         FROM music_tracks t
         LEFT JOIN (
             SELECT item_id, COUNT(*) AS plays
             FROM listening_sessions
             WHERE media_kind = 'music'
             GROUP BY item_id
         ) p ON p.item_id = t.id
         WHERE t.family_id = $1
           AND t.audio_object_id IS NOT NULL
           AND (t.analysis_version IS NULL OR t.analysis_version < $2)
           AND NOT EXISTS (
               SELECT 1 FROM jobs j
               WHERE j.job_type = 'analyze_audio'
                 AND j.status IN ('pending', 'running')
                 AND j.payload->>'track_id' = t.id::text
           )
         ORDER BY COALESCE(p.plays, 0) DESC, t.created_at DESC
         LIMIT $3",
    )
    .bind(family_id)
    .bind(version)
    .bind(limit)
    .execute(pool)
    .await
    .context("db: enqueue family analysis")?;
    Ok(result.rows_affected())
}

/// Families that asked for measurement. The sweep's guard.
///
/// `media_checksum`'s sweep runs unconditionally for every object; this one
/// must not, or the opt-in is decoration and the library gets measured anyway.
/// That is the easiest thing to get wrong by copying that job too faithfully.
pub async fn families_wanting_analysis(pool: &PgPool) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar::<_, Uuid>(
        "SELECT id FROM families WHERE audio_analysis_enabled",
    )
    .fetch_all(pool)
    .await
    .context("db: families wanting analysis")
}

/// Rebuild the derived energy scale.
///
/// Cheap — it reads the stored measurements and ranks them, touching no audio.
/// CONCURRENTLY so playlist queries keep answering while it runs; that needs
/// the unique index migration 0072 creates.
///
/// Called after a batch of analysis and from the nightly rollup, because the
/// scale is a percentile and therefore shifts as the library grows.
pub async fn refresh_energy(pool: &PgPool) -> anyhow::Result<()> {
    // CONCURRENTLY cannot populate a view that has never been filled, and a
    // never-refreshed view is exactly the state right after the migration.
    let populated: bool = sqlx::query_scalar(
        "SELECT ispopulated FROM pg_matviews WHERE matviewname = 'music_track_energy'",
    )
    .fetch_optional(pool)
    .await
    .context("db: check energy view")?
    .unwrap_or(false);

    let sql = if populated {
        "REFRESH MATERIALIZED VIEW CONCURRENTLY music_track_energy"
    } else {
        "REFRESH MATERIALIZED VIEW music_track_energy"
    };

    sqlx::query(sql)
        .execute(pool)
        .await
        .context("db: refresh energy view")?;
    Ok(())
}

// ── Preference affinity rollup ─────────────────────────────────────────────

#[derive(Debug, sqlx::FromRow)]
struct AffinityRow {
    user_id: Uuid,
    track_id: Uuid,
    play_count: i32,
    early_skips: i32,
    late_skips: i32,
    last_played_at: Option<DateTime<Utc>>,
    last_skipped_at: Option<DateTime<Utc>>,
    starred: bool,
    rating: Option<i16>,
    disliked_at: Option<DateTime<Utc>>,
}

/// Rebuild `music_track_affinity` from the session log and the explicit marks.
///
/// Aggregation happens in SQL because the session log is large; the weight
/// itself is computed in `music::affinity` because it is a product decision
/// that will be retuned, and having one copy of it in Rust beats two copies
/// that can disagree. The skip boundary is bound from the same Rust constant
/// rather than written into the query twice.
///
/// `days` bounds how far back sessions are read, matching `rebuild_daily_rollup`.
pub async fn rebuild_affinity(pool: &PgPool, days: i32) -> anyhow::Result<u64> {
    let rows = sqlx::query_as::<_, AffinityRow>(
        "WITH s AS (
             SELECT ls.user_id,
                    ls.item_id AS track_id,
                    ls.ended_reason,
                    ls.started_at,
                    CASE
                        WHEN t.duration_secs IS NULL OR t.duration_secs <= 0 THEN NULL
                        ELSE ls.seconds_listened::float8 / t.duration_secs
                    END AS fraction
             FROM listening_sessions ls
             JOIN music_tracks t ON t.id = ls.item_id
             WHERE ls.media_kind = 'music'
               AND ls.ended_reason IS NOT NULL
               AND ls.started_at >= (now() - make_interval(days => $1))
         ),
         agg AS (
             SELECT user_id,
                    track_id,
                    COUNT(*) FILTER (WHERE ended_reason = 'completed')::int AS play_count,
                    COUNT(*) FILTER (
                        WHERE ended_reason = 'skipped' AND fraction IS NOT NULL AND fraction < $2
                    )::int AS early_skips,
                    COUNT(*) FILTER (
                        WHERE ended_reason = 'skipped' AND fraction IS NOT NULL AND fraction >= $2
                    )::int AS late_skips,
                    MAX(started_at) FILTER (WHERE ended_reason = 'completed') AS last_played_at,
                    MAX(started_at) FILTER (WHERE ended_reason = 'skipped')   AS last_skipped_at
             FROM s
             GROUP BY user_id, track_id
         )
         SELECT agg.user_id,
                agg.track_id,
                agg.play_count,
                agg.early_skips,
                agg.late_skips,
                agg.last_played_at,
                agg.last_skipped_at,
                (st.user_id IS NOT NULL) AS starred,
                r.rating,
                CASE WHEN f.kind = 'dislike' THEN f.updated_at END AS disliked_at
         FROM agg
         LEFT JOIN music_track_stars   st ON st.user_id = agg.user_id AND st.track_id = agg.track_id
         LEFT JOIN music_track_ratings r  ON r.user_id  = agg.user_id AND r.track_id  = agg.track_id
         LEFT JOIN music_track_feedback f ON f.user_id  = agg.user_id AND f.track_id  = agg.track_id",
    )
    .bind(days)
    .bind(crate::music::affinity::LATE_SKIP_FRACTION)
    .fetch_all(pool)
    .await
    .context("db: read affinity inputs")?;

    if rows.is_empty() {
        return Ok(0);
    }

    let now = Utc::now();
    let mut users = Vec::with_capacity(rows.len());
    let mut tracks = Vec::with_capacity(rows.len());
    let mut plays = Vec::with_capacity(rows.len());
    let mut earlies = Vec::with_capacity(rows.len());
    let mut lates = Vec::with_capacity(rows.len());
    let mut played = Vec::with_capacity(rows.len());
    let mut skipped = Vec::with_capacity(rows.len());
    let mut weights = Vec::with_capacity(rows.len());

    for r in &rows {
        let w = crate::music::affinity::weight(
            &crate::music::affinity::AffinityInputs {
                play_count: r.play_count,
                early_skips: r.early_skips,
                late_skips: r.late_skips,
                last_played_at: r.last_played_at,
                last_skipped_at: r.last_skipped_at,
                starred: r.starred,
                rating: r.rating,
                disliked_at: r.disliked_at,
            },
            now,
        );
        users.push(r.user_id);
        tracks.push(r.track_id);
        plays.push(r.play_count);
        earlies.push(r.early_skips);
        lates.push(r.late_skips);
        played.push(r.last_played_at);
        skipped.push(r.last_skipped_at);
        weights.push(w as f32);
    }

    let result = sqlx::query(
        "INSERT INTO music_track_affinity
             (user_id, track_id, play_count, early_skips, late_skips,
              last_played_at, last_skipped_at, weight)
         SELECT * FROM UNNEST(
             $1::uuid[], $2::uuid[], $3::int[], $4::int[], $5::int[],
             $6::timestamptz[], $7::timestamptz[], $8::real[]
         ) AS t(user_id, track_id, play_count, early_skips, late_skips,
                last_played_at, last_skipped_at, weight)
         ON CONFLICT (user_id, track_id) DO UPDATE
             SET play_count      = EXCLUDED.play_count,
                 early_skips     = EXCLUDED.early_skips,
                 late_skips      = EXCLUDED.late_skips,
                 last_played_at  = EXCLUDED.last_played_at,
                 last_skipped_at = EXCLUDED.last_skipped_at,
                 weight          = EXCLUDED.weight,
                 updated_at      = now()",
    )
    .bind(&users)
    .bind(&tracks)
    .bind(&plays)
    .bind(&earlies)
    .bind(&lates)
    .bind(&played)
    .bind(&skipped)
    .bind(&weights)
    .execute(pool)
    .await
    .context("db: write affinity rollup")?;

    Ok(result.rows_affected())
}

/// A user's negative feedback on one track. See migration 0069 for why
/// `dislike` and `banned` are two meanings rather than one.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TrackFeedback {
    pub track_id: Uuid,
    pub kind: String,
    pub updated_at: DateTime<Utc>,
}

/// Upsert, so moving a track from `dislike` to `banned` (or back) replaces the
/// row rather than accumulating a second one.
pub async fn set_track_feedback(
    pool: &PgPool,
    user_id: Uuid,
    track_id: Uuid,
    kind: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO music_track_feedback (user_id, track_id, kind)
         VALUES ($1, $2, $3)
         ON CONFLICT (user_id, track_id)
         DO UPDATE SET kind = EXCLUDED.kind, updated_at = now()",
    )
    .bind(user_id)
    .bind(track_id)
    .bind(kind)
    .execute(pool)
    .await
    .context("db: set track feedback")?;
    Ok(())
}

/// Undo. Deleting the row is the whole of it — nothing about the track itself
/// was ever changed, so there is nothing to restore.
pub async fn clear_track_feedback(
    pool: &PgPool,
    user_id: Uuid,
    track_id: Uuid,
) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM music_track_feedback WHERE user_id = $1 AND track_id = $2")
        .bind(user_id)
        .bind(track_id)
        .execute(pool)
        .await
        .context("db: clear track feedback")?;
    Ok(())
}

/// Everything this user has marked. A client needs this to render an undo list:
/// someone who banned a track by a mis-tap has no other way to find it again.
pub async fn list_track_feedback(
    pool: &PgPool,
    user_id: Uuid,
    kind: Option<&str>,
) -> anyhow::Result<Vec<TrackFeedback>> {
    sqlx::query_as::<_, TrackFeedback>(
        "SELECT track_id, kind, updated_at
         FROM music_track_feedback
         WHERE user_id = $1 AND ($2::text IS NULL OR kind = $2)
         ORDER BY updated_at DESC",
    )
    .bind(user_id)
    .bind(kind)
    .fetch_all(pool)
    .await
    .context("db: list track feedback")
}

pub async fn unstar_track(pool: &PgPool, user_id: Uuid, track_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM music_track_stars WHERE user_id = $1 AND track_id = $2")
        .bind(user_id)
        .bind(track_id)
        .execute(pool)
        .await
        .context("db: unstar track")?;
    Ok(())
}

/// `album` is `""` for an artist star, matching the table's own CHECK.
pub async fn star_group(
    pool: &PgPool,
    user_id: Uuid,
    kind: &str,
    artist: &str,
    album: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO music_group_stars (user_id, kind, artist_name, album_name)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (user_id, kind, artist_name, album_name) DO NOTHING",
    )
    .bind(user_id)
    .bind(kind)
    .bind(artist)
    .bind(album)
    .execute(pool)
    .await
    .context("db: star group")?;
    Ok(())
}

pub async fn unstar_group(
    pool: &PgPool,
    user_id: Uuid,
    kind: &str,
    artist: &str,
    album: &str,
) -> anyhow::Result<()> {
    sqlx::query(
        "DELETE FROM music_group_stars
         WHERE user_id = $1 AND kind = $2 AND artist_name = $3 AND album_name = $4",
    )
    .bind(user_id)
    .bind(kind)
    .bind(artist)
    .bind(album)
    .execute(pool)
    .await
    .context("db: unstar group")?;
    Ok(())
}

/// Every starred track id for a user, for annotating song payloads in bulk.
///
/// Fetched once per response rather than per song: a 500-track album list
/// would otherwise issue 500 queries.
pub async fn starred_track_ids(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<(Uuid, DateTime<Utc>)>> {
    sqlx::query_as::<_, (Uuid, DateTime<Utc>)>(
        "SELECT track_id, created_at FROM music_track_stars WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: starred track ids")
}

/// Starred artists/albums as `(kind, artist, album, created_at)`.
pub async fn starred_groups(
    pool: &PgPool,
    user_id: Uuid,
) -> anyhow::Result<Vec<(String, String, String, DateTime<Utc>)>> {
    sqlx::query_as::<_, (String, String, String, DateTime<Utc>)>(
        "SELECT kind, artist_name, album_name, created_at
         FROM music_group_stars WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: starred groups")
}

/// Subsonic's rating 0 means "remove the rating", which is the absence of a
/// row here rather than a stored zero — hence the delete branch.
pub async fn set_track_rating(
    pool: &PgPool,
    user_id: Uuid,
    track_id: Uuid,
    rating: i16,
) -> anyhow::Result<()> {
    if rating <= 0 {
        sqlx::query("DELETE FROM music_track_ratings WHERE user_id = $1 AND track_id = $2")
            .bind(user_id)
            .bind(track_id)
            .execute(pool)
            .await
            .context("db: clear track rating")?;
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO music_track_ratings (user_id, track_id, rating) VALUES ($1, $2, $3)
         ON CONFLICT (user_id, track_id)
         DO UPDATE SET rating = EXCLUDED.rating, updated_at = now()",
    )
    .bind(user_id)
    .bind(track_id)
    .bind(rating.min(5))
    .execute(pool)
    .await
    .context("db: set track rating")?;
    Ok(())
}

pub async fn track_ratings(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<(Uuid, i16)>> {
    sqlx::query_as::<_, (Uuid, i16)>(
        "SELECT track_id, rating FROM music_track_ratings WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: track ratings")
}

// ── List queries backing getSongsByGenre / getRandomSongs ─────────────────

pub async fn list_tracks_by_genre(
    pool: &PgPool,
    viewer: Viewer,
    genre: &str,
    limit: i64,
    offset: i64,
) -> anyhow::Result<Vec<MusicTrack>> {
    let sql = format!(
        "SELECT {TRACK_COLS}
         FROM music_tracks t
         WHERE {VISIBLE} AND lower(trim(t.genre)) = lower($5)
         ORDER BY t.artist NULLS LAST, t.album NULLS LAST, t.disc_number NULLS FIRST, t.track_number NULLS LAST, t.title
         LIMIT $6 OFFSET $7"
    );
    bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer, MUSIC)
        .bind(genre)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await
        .context("db: list tracks by genre")
}

/// Randomised in the database rather than in Rust so the `LIMIT` can do the
/// work — a library of any size would otherwise have to be loaded in full to
/// pick ten songs.
pub async fn list_random_tracks(
    pool: &PgPool,
    viewer: Viewer,
    genre: Option<&str>,
    from_year: Option<i32>,
    to_year: Option<i32>,
    limit: i64,
) -> anyhow::Result<Vec<MusicTrack>> {
    // `year` is not a column on music_tracks; the parameters are accepted and
    // ignored rather than rejected, because clients send them unconditionally
    // and a hard failure would break the shuffle button outright.
    let _ = (from_year, to_year);

    let genre_clause = if genre.is_some() {
        "AND lower(trim(t.genre)) = lower($5)"
    } else {
        ""
    };
    let limit_param = if genre.is_some() { "$6" } else { "$5" };
    let sql = format!(
        "SELECT {TRACK_COLS}
         FROM music_tracks t
         WHERE {VISIBLE} {genre_clause}
         ORDER BY random()
         LIMIT {limit_param}"
    );

    let query = bind_viewer!(sqlx::query_as::<_, MusicTrack>(&sql), viewer, MUSIC);
    let query = match genre {
        Some(g) => query.bind(g),
        None => query,
    };
    query
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list random tracks")
}

// ── Bookmarks ─────────────────────────────────────────────────────────────

pub struct MusicBookmark {
    pub track_id: Uuid,
    pub position_secs: f64,
    pub label: Option<String>,
    pub created_at: DateTime<Utc>,
}

pub async fn list_music_bookmarks(pool: &PgPool, user_id: Uuid) -> anyhow::Result<Vec<MusicBookmark>> {
    let rows = sqlx::query_as::<_, (Uuid, f64, Option<String>, DateTime<Utc>)>(
        "SELECT track_id, position_secs, label, created_at
         FROM bookmarks
         WHERE user_id = $1 AND track_id IS NOT NULL
         ORDER BY created_at",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: list music bookmarks")?;

    Ok(rows
        .into_iter()
        .map(|(track_id, position_secs, label, created_at)| MusicBookmark {
            track_id,
            position_secs,
            label,
            created_at,
        })
        .collect())
}

/// One bookmark per (user, track): Subsonic's `createBookmark` is an upsert,
/// not an append — calling it twice on a track moves the mark rather than
/// leaving two behind.
pub async fn upsert_music_bookmark(
    pool: &PgPool,
    user_id: Uuid,
    track_id: Uuid,
    position_secs: f64,
    comment: Option<&str>,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM bookmarks WHERE user_id = $1 AND track_id = $2")
        .bind(user_id)
        .bind(track_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "INSERT INTO bookmarks (user_id, track_id, position_secs, label)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id)
    .bind(track_id)
    .bind(position_secs)
    .bind(comment)
    .execute(&mut *tx)
    .await?;
    tx.commit().await.context("db: upsert music bookmark")?;
    Ok(())
}

pub async fn delete_music_bookmark(pool: &PgPool, user_id: Uuid, track_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM bookmarks WHERE user_id = $1 AND track_id = $2")
        .bind(user_id)
        .bind(track_id)
        .execute(pool)
        .await
        .context("db: delete music bookmark")?;
    Ok(())
}

/// The stored artist photo's object id, for the Subsonic cover-art route.
///
/// Separate from [`find_artist_image`], which returns the storage key and is
/// shaped for serving bytes directly; the Subsonic path resolves an id first
/// and fetches through the same helper every other cover art uses.
pub async fn find_artist_image_object(
    pool: &PgPool,
    user_id: Uuid,
    artist: &str,
) -> anyhow::Result<Option<Uuid>> {
    sqlx::query_scalar::<_, Option<Uuid>>(
        "SELECT image_object_id FROM music_artist_images
         WHERE user_id = $1 AND lower(artist_name) = lower($2)",
    )
    .bind(user_id)
    .bind(artist)
    .fetch_optional(pool)
    .await
    .map(Option::flatten)
    .context("db: find artist image object")
}

/// Track count for the viewer's library, for Subsonic's `getScanStatus`.
pub async fn count_tracks(pool: &PgPool, viewer: Viewer) -> anyhow::Result<i64> {
    let sql = format!("SELECT COUNT(*) FROM music_tracks t WHERE {VISIBLE}");
    bind_viewer!(sqlx::query_scalar::<_, i64>(&sql), viewer, MUSIC)
        .fetch_one(pool)
        .await
        .context("db: count music tracks")
}

/// Rating for an album or artist. `album` is `""` for an artist, matching the
/// table's CHECK. Rating 0 removes the row, as for tracks.
pub async fn set_group_rating(
    pool: &PgPool,
    user_id: Uuid,
    kind: &str,
    artist: &str,
    album: &str,
    rating: i16,
) -> anyhow::Result<()> {
    if rating <= 0 {
        sqlx::query(
            "DELETE FROM music_group_ratings
             WHERE user_id = $1 AND kind = $2 AND artist_name = $3 AND album_name = $4",
        )
        .bind(user_id)
        .bind(kind)
        .bind(artist)
        .bind(album)
        .execute(pool)
        .await
        .context("db: clear group rating")?;
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO music_group_ratings (user_id, kind, artist_name, album_name, rating)
         VALUES ($1, $2, $3, $4, $5)
         ON CONFLICT (user_id, kind, artist_name, album_name)
         DO UPDATE SET rating = EXCLUDED.rating, updated_at = now()",
    )
    .bind(user_id)
    .bind(kind)
    .bind(artist)
    .bind(album)
    .bind(rating.min(5))
    .execute(pool)
    .await
    .context("db: set group rating")?;
    Ok(())
}

pub async fn group_ratings(
    pool: &PgPool,
    user_id: Uuid,
) -> anyhow::Result<Vec<(String, String, String, i16)>> {
    sqlx::query_as::<_, (String, String, String, i16)>(
        "SELECT kind, artist_name, album_name, rating
         FROM music_group_ratings WHERE user_id = $1",
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
    .context("db: group ratings")
}

// ── Release backfill (docs/album-artist-plan.md A2) ─────────────────────────

/// Identified tracks whose release hasn't been re-read since the metadata service started
/// returning the release's artist credit.
pub async fn enqueue_release_backfill(pool: &PgPool, limit: i64) -> anyhow::Result<u64> {
    let result = sqlx::query(
        "INSERT INTO jobs (job_type, payload, status)
         SELECT 'music_release_backfill', jsonb_build_object('track_id', t.id::text), 'pending'
         FROM music_tracks t
         WHERE t.musicbrainz_recording_id IS NOT NULL
           AND t.release_checked_at IS NULL
           AND NOT EXISTS (
               SELECT 1 FROM jobs j
               WHERE j.job_type = 'music_release_backfill'
                 AND j.status IN ('pending', 'running')
                 AND j.payload->>'track_id' = t.id::text
           )
         ORDER BY t.created_at
         LIMIT $1",
    )
    .bind(limit)
    .execute(pool)
    .await
    .context("db: enqueue music release backfill")?;
    Ok(result.rows_affected())
}

/// `(recording id, release id)` of an identified track, or `None` once it is gone or no longer
/// identified.
pub async fn release_backfill_target(
    pool: &PgPool,
    track_id: Uuid,
) -> anyhow::Result<Option<(String, Option<String>)>> {
    sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT musicbrainz_recording_id, musicbrainz_release_id
         FROM music_tracks WHERE id = $1 AND musicbrainz_recording_id IS NOT NULL",
    )
    .bind(track_id)
    .fetch_optional(pool)
    .await
    .context("db: music release backfill target")
}

/// Records what the release says. `updated_at` moves only when the album artist really changes:
/// it is what `/library/changes` hands clients, and a no-op must not re-sync a whole library.
pub async fn set_release_info(
    pool: &PgPool,
    track_id: Uuid,
    album_artist: Option<&str>,
    mb_release_group_id: Option<&str>,
) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE music_tracks
         SET updated_at = CASE WHEN album_artist IS DISTINCT FROM COALESCE($2, album_artist)
                               THEN CURRENT_TIMESTAMP ELSE updated_at END,
             album_artist = COALESCE($2, album_artist),
             musicbrainz_release_group_id = COALESCE($3, musicbrainz_release_group_id),
             release_checked_at = CURRENT_TIMESTAMP
         WHERE id = $1",
    )
    .bind(track_id)
    .bind(album_artist)
    .bind(mb_release_group_id)
    .execute(pool)
    .await
    .context("db: set music release info")?;
    Ok(())
}
