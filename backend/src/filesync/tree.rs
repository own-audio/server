// SPDX-License-Identifier: AGPL-3.0-or-later
// Row tuples mirror the SELECT lists one-to-one; naming each a type would hide that.
#![allow(clippy::type_complexity)]
/// The sync feed — docs/file-sync-plan.md §5.3 and §5.4.
///
/// `GET /sync/tree` hands a sync client every item that belongs in the
/// own.audio folder, then only what changed. A first call (no cursor) pages
/// through a full snapshot; later calls read the `sync_changes` log from
/// migration 0078 and send each changed item's current state, or its removal.
///
/// The cursor is the oldest transaction still running when the previous round
/// began (see the migration): a late commit is always picked up, at the price
/// of an item sometimes arriving twice, which a client applies idempotently.
use super::paths::{self, Kind};
use crate::db::access::{self, Viewer};
use anyhow::Context;
use base64::Engine;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::collections::{BTreeSet, HashMap};
use uuid::Uuid;

pub const DEFAULT_LIMIT: i64 = 500;
pub const MAX_LIMIT: i64 = 2000;

/// A cursor older than this may point at log rows `trash_purge` has already
/// pruned, so the client starts over with a full snapshot.
pub const LOG_RETENTION_DAYS: i64 = crate::db::trash::TOMBSTONE_RETENTION_DAYS;

const KINDS: [Kind; 4] = [Kind::Audiobook, Kind::MusicTrack, Kind::PodcastEpisode, Kind::CompanionFile];

/// A companion file is visible under the member policy of the kind whose
/// folder it sits in; it has no grants of its own.
const COMPANION_VISIBLE: &str =
    "audio2_can_access($1, $2, $3, t.media_kind, t.id, t.user_id, t.family_id) AND $4::text IS NOT NULL";

// ── Wire format ───────────────────────────────────────────────────────────

#[derive(Serialize)]
pub struct TreeResponse {
    /// The caller, named as the family knows them — the name of their own
    /// `Family/<Me>` folder (§2 item 17).
    pub me: Owner,
    pub cursor: String,
    pub has_more: bool,
    /// The cursor was too old: drop what you know and rebuild from the items
    /// that follow (the next pages are a full snapshot).
    pub reset: bool,
    pub items: Vec<TreeItem>,
    pub removed: Vec<Removed>,
}

#[derive(Serialize)]
pub struct TreeItem {
    pub kind: &'static str,
    pub id: Uuid,
    pub updated_at: DateTime<Utc>,
    pub owner: Owner,
    pub is_owner: bool,
    pub can_delete: bool,
    pub shared_with_family: bool,
    /// For messages only; the tree is built from `path`.
    pub title: String,
    /// A book's folder, a track's or episode's file — as the owner put it.
    pub path: String,
    /// A book's files, with their path inside its folder. A track, episode
    /// or companion file has one entry with an empty `relative_path`.
    pub files: Vec<TreeFile>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub show: Option<Show>,
    /// What an album shortcut matches on (§4.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub album: Option<Album>,
}

#[derive(Serialize)]
pub struct Owner {
    pub id: Uuid,
    pub display_name: Option<String>,
}

#[derive(Serialize)]
pub struct TreeFile {
    pub id: Uuid,
    pub relative_path: String,
    pub size_bytes: Option<i64>,
    /// Filled by a background job after upload; verify by size until then.
    pub sha256: Option<String>,
}

#[derive(Serialize)]
pub struct Show {
    pub id: Uuid,
    pub title: String,
}

#[derive(Serialize)]
pub struct Album {
    pub title: Option<String>,
    pub album_artist: Option<String>,
    pub release_group: Option<String>,
}

#[derive(Serialize, Debug, PartialEq)]
pub struct Removed {
    pub kind: &'static str,
    pub id: Uuid,
    /// `trashed` (in the trash, may come back), `deleted` (gone for good, or
    /// an episode no longer stored), `hidden` (still exists, no longer
    /// visible to you — unshared, or access withdrawn).
    pub reason: &'static str,
}

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
struct Cursor {
    /// Full snapshot in progress: the last (kind, id) sent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    full: Option<(u8, Uuid)>,
    /// Oldest transaction this round must still look at.
    from: i64,
    /// Log rows already sent in this round (delta paging).
    #[serde(default)]
    seq: i64,
    /// Where the next round starts; fixed when a round begins.
    next: i64,
    /// When `from` was taken, for the retention check.
    at: i64,
}

impl Cursor {
    fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("cursor serialises");
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(json)
    }

    fn decode(raw: &str) -> Option<Self> {
        let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(raw.trim()).ok()?;
        serde_json::from_slice(&bytes).ok()
    }
}

// ── Entry point ───────────────────────────────────────────────────────────

pub enum CursorError {
    Invalid,
}

pub async fn read(
    pool: &PgPool,
    viewer: Viewer,
    cursor: Option<&str>,
    limit: i64,
) -> anyhow::Result<Result<TreeResponse, CursorError>> {
    let limit = limit.clamp(1, MAX_LIMIT);
    let now = Utc::now().timestamp();
    let cursor = match cursor.map(str::trim).filter(|c| !c.is_empty()) {
        None => None,
        Some(raw) => match Cursor::decode(raw) {
            Some(c) => Some(c),
            None => return Ok(Err(CursorError::Invalid)),
        },
    };

    let mut response = match cursor {
        None => full_page(pool, viewer, fresh_full(pool, now).await?, limit, false).await?,
        Some(c) if c.full.is_some() => full_page(pool, viewer, c, limit, false).await?,
        Some(c) if now - c.at > LOG_RETENTION_DAYS * 86_400 => {
            full_page(pool, viewer, fresh_full(pool, now).await?, limit, true).await?
        }
        Some(c) => delta_page(pool, viewer, c, limit, now).await?,
    };
    let name: Option<String> = sqlx::query_scalar(
        "SELECT COALESCE(fm.display_label, u.display_name)
           FROM users u LEFT JOIN family_members fm ON fm.user_id = u.id WHERE u.id = $1",
    )
    .bind(viewer.user_id)
    .fetch_optional(pool)
    .await
    .context("db: caller name")?
    .flatten();
    response.me = Owner { id: viewer.user_id, display_name: name };
    Ok(Ok(response))
}

/// Filled in by `read` once the page is built.
fn nobody() -> Owner {
    Owner { id: Uuid::nil(), display_name: None }
}

async fn snapshot_xmin(pool: &PgPool) -> anyhow::Result<i64> {
    sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot())::text::bigint")
        .fetch_one(pool)
        .await
        .context("db: snapshot xmin")
}

async fn fresh_full(pool: &PgPool, now: i64) -> anyhow::Result<Cursor> {
    let x = snapshot_xmin(pool).await?;
    Ok(Cursor { full: Some((0, Uuid::nil())), from: x, seq: 0, next: x, at: now })
}

// ── Full snapshot ─────────────────────────────────────────────────────────

async fn full_page(pool: &PgPool, viewer: Viewer, cursor: Cursor, limit: i64, reset: bool) -> anyhow::Result<TreeResponse> {
    let (mut kind_ix, mut after) = cursor.full.expect("full cursor");
    let mut keys: Vec<(Kind, Uuid)> = Vec::new();
    while (kind_ix as usize) < KINDS.len() && (keys.len() as i64) < limit {
        let kind = KINDS[kind_ix as usize];
        let ids = visible_ids_after(pool, viewer, kind, after, limit - keys.len() as i64).await?;
        let exhausted = (ids.len() as i64) < limit - keys.len() as i64;
        if let Some(last) = ids.last() {
            after = *last;
        }
        keys.extend(ids.into_iter().map(|id| (kind, id)));
        if exhausted {
            kind_ix += 1;
            after = Uuid::nil();
        }
    }

    let done = kind_ix as usize >= KINDS.len();
    let next = if done {
        // The snapshot is complete; changes made while it was paged come
        // from the log, starting where the snapshot started.
        Cursor { full: None, from: cursor.next, seq: 0, next: cursor.next, at: cursor.at }
    } else {
        Cursor { full: Some((kind_ix, after)), ..cursor }
    };

    paths::ensure_all(pool, &keys).await?;
    let (items, _) = load(pool, viewer, &keys).await?;
    Ok(TreeResponse { me: nobody(), cursor: next.encode(), has_more: !done, reset, items, removed: Vec::new() })
}

async fn visible_ids_after(pool: &PgPool, viewer: Viewer, kind: Kind, after: Uuid, limit: i64) -> anyhow::Result<Vec<Uuid>> {
    let visible = access::VISIBLE;
    let sql = match kind {
        Kind::Audiobook => format!(
            "SELECT t.id FROM audiobook_books t WHERE {visible} AND t.id > $5 ORDER BY t.id LIMIT $6"
        ),
        Kind::MusicTrack => format!(
            "SELECT t.id FROM music_tracks t WHERE {visible} AND t.id > $5 ORDER BY t.id LIMIT $6"
        ),
        Kind::PodcastEpisode => format!(
            "SELECT e.id FROM podcast_episodes e JOIN podcast_feeds t ON t.id = e.feed_id
              WHERE {visible} AND e.audio_object_id IS NOT NULL AND e.id > $5
              ORDER BY e.id LIMIT $6"
        ),
        Kind::CompanionFile => format!(
            "SELECT t.id FROM companion_files t WHERE {COMPANION_VISIBLE} AND t.id > $5 ORDER BY t.id LIMIT $6"
        ),
    };
    sqlx::query_scalar(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .bind(access_kind(kind))
        .bind(after)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: visible ids for sync tree")
}

fn access_kind(kind: Kind) -> &'static str {
    match kind {
        Kind::Audiobook => access::AUDIOBOOK,
        Kind::MusicTrack => access::MUSIC,
        Kind::PodcastEpisode => access::PODCAST,
        // Unused: companion files carry their kind per row.
        Kind::CompanionFile => access::MUSIC,
    }
}

// ── Changes ───────────────────────────────────────────────────────────────

async fn delta_page(pool: &PgPool, viewer: Viewer, cursor: Cursor, limit: i64, now: i64) -> anyhow::Result<TreeResponse> {
    // A new round fixes where the round after it will start.
    let (next, at) = if cursor.seq == 0 { (snapshot_xmin(pool).await?, now) } else { (cursor.next, cursor.at) };

    let rows: Vec<(i64, String, Uuid)> = sqlx::query_as(
        "SELECT seq, kind, item_id FROM sync_changes
          WHERE xid >= $1::text::xid8 AND seq > $2
            AND (owner_id = $3 OR owner_family_id = $4)
          ORDER BY seq LIMIT $5",
    )
    .bind(cursor.from)
    .bind(cursor.seq)
    .bind(viewer.user_id)
    .bind(viewer.family_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: read sync changes")?;

    let has_more = rows.len() as i64 == limit;
    let new_cursor = if has_more {
        Cursor { full: None, from: cursor.from, seq: rows.last().map(|r| r.0).unwrap_or(cursor.seq), next, at: cursor.at }
    } else {
        Cursor { full: None, from: next, seq: 0, next, at }
    };

    let mut keys: BTreeSet<(Kind, Uuid)> = BTreeSet::new();
    let mut feeds: Vec<Uuid> = Vec::new();
    for (_, kind, id) in rows {
        match Kind::parse(&kind) {
            Some(k) => {
                keys.insert((k, id));
            }
            None if kind == "podcast_feed" => feeds.push(id),
            None => {}
        }
    }
    if !feeds.is_empty() {
        // A show's sharing changed: every copy of it the family keeps.
        let episodes: Vec<Uuid> = sqlx::query_scalar(
            "SELECT id FROM podcast_episodes
              WHERE feed_id = ANY($1) AND (audio_object_id IS NOT NULL OR trashed_at IS NOT NULL)",
        )
        .bind(&feeds)
        .fetch_all(pool)
        .await
        .context("db: stored episodes of changed shows")?;
        keys.extend(episodes.into_iter().map(|id| (Kind::PodcastEpisode, id)));
    }

    let keys: Vec<(Kind, Uuid)> = keys.into_iter().collect();
    paths::ensure_all(pool, &keys).await?;
    let (items, missing) = load(pool, viewer, &keys).await?;
    let removed = removal_reasons(pool, &missing).await?;
    Ok(TreeResponse { me: nobody(), cursor: new_cursor.encode(), has_more, reset: false, items, removed })
}

/// Why each of these items is not in the caller's tree.
async fn removal_reasons(pool: &PgPool, missing: &[(Kind, Uuid)]) -> anyhow::Result<Vec<Removed>> {
    let mut out = Vec::with_capacity(missing.len());
    for &(kind, id) in missing {
        let state: Option<(bool, bool)> = match kind {
            // (exists in the trash, exists live)
            Kind::Audiobook => sqlx::query_as(
                "SELECT trashed_at IS NOT NULL, trashed_at IS NULL FROM audiobook_books_all WHERE id = $1",
            ),
            Kind::MusicTrack => sqlx::query_as(
                "SELECT trashed_at IS NOT NULL, trashed_at IS NULL FROM music_tracks_all WHERE id = $1",
            ),
            Kind::PodcastEpisode => sqlx::query_as(
                "SELECT trashed_at IS NOT NULL, audio_object_id IS NOT NULL FROM podcast_episodes WHERE id = $1",
            ),
            Kind::CompanionFile => sqlx::query_as(
                "SELECT trashed_at IS NOT NULL, trashed_at IS NULL FROM companion_files_all WHERE id = $1",
            ),
        }
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: removal reason")?;
        let reason = match state {
            Some((true, false)) => "trashed",
            Some((_, true)) => "hidden",
            _ => "deleted",
        };
        out.push(Removed { kind: kind.as_str(), id, reason });
    }
    Ok(out)
}

// ── Loading items ─────────────────────────────────────────────────────────

/// The caller's view of these items. Returns the visible, live ones and the
/// keys that are not (for `removed`), in input order.
async fn load(pool: &PgPool, viewer: Viewer, keys: &[(Kind, Uuid)]) -> anyhow::Result<(Vec<TreeItem>, Vec<(Kind, Uuid)>)> {
    let ids = |kind: Kind| -> Vec<Uuid> { keys.iter().filter(|(k, _)| *k == kind).map(|(_, id)| *id).collect() };
    let mut found: HashMap<(Kind, Uuid), TreeItem> = HashMap::new();

    let books = ids(Kind::Audiobook);
    if !books.is_empty() {
        for item in load_books(pool, viewer, &books).await? {
            found.insert((Kind::Audiobook, item.id), item);
        }
    }
    let tracks = ids(Kind::MusicTrack);
    if !tracks.is_empty() {
        for item in load_tracks(pool, viewer, &tracks).await? {
            found.insert((Kind::MusicTrack, item.id), item);
        }
    }
    let episodes = ids(Kind::PodcastEpisode);
    if !episodes.is_empty() {
        for item in load_episodes(pool, viewer, &episodes).await? {
            found.insert((Kind::PodcastEpisode, item.id), item);
        }
    }
    let companions = ids(Kind::CompanionFile);
    if !companions.is_empty() {
        for item in load_companions(pool, viewer, &companions).await? {
            found.insert((Kind::CompanionFile, item.id), item);
        }
    }

    let mut items = Vec::with_capacity(found.len());
    let mut missing = Vec::new();
    for key in keys {
        match found.remove(key) {
            Some(item) => items.push(item),
            None => missing.push(*key),
        }
    }
    Ok((items, missing))
}

fn can_delete(viewer: Viewer, owner: Uuid, family_id: Option<Uuid>) -> bool {
    owner == viewer.user_id || (viewer.is_family_admin && family_id == Some(viewer.family_id))
}

#[derive(sqlx::FromRow)]
struct BookRow {
    id: Uuid,
    user_id: Uuid,
    family_id: Option<Uuid>,
    title: String,
    updated_at: DateTime<Utc>,
    owner_name: Option<String>,
    path: String,
}

async fn load_books(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<TreeItem>> {
    let sql = format!(
        "SELECT t.id, t.user_id, t.family_id, t.title, t.updated_at, {OWNER_NAME} AS owner_name, p.path
           FROM audiobook_books t
           JOIN sync_paths p ON p.kind = 'audiobook' AND p.item_id = t.id
          WHERE t.id = ANY($5) AND {}",
        access::VISIBLE
    );
    let rows: Vec<BookRow> = bind_viewer(sqlx::query_as(&sql), viewer, access::AUDIOBOOK)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: sync tree books")?;
    if rows.is_empty() {
        return Ok(Vec::new());
    }

    let book_ids: Vec<Uuid> = rows.iter().map(|r| r.id).collect();
    let files: Vec<(Uuid, Uuid, Option<String>, Option<i64>, Option<String>)> = sqlx::query_as(
        "SELECT f.book_id, f.id, f.relative_path, m.size_bytes, m.sha256
           FROM audiobook_files f JOIN media_objects m ON m.id = f.audio_object_id
          WHERE f.book_id = ANY($1) ORDER BY f.book_id, f.position",
    )
    .bind(&book_ids)
    .fetch_all(pool)
    .await
    .context("db: sync tree book files")?;
    let mut by_book: HashMap<Uuid, Vec<TreeFile>> = HashMap::new();
    for (book, id, relative, size, sha) in files {
        // A file still waiting for its name is left out until it has one.
        if let Some(relative_path) = relative {
            by_book.entry(book).or_default().push(TreeFile { id, relative_path, size_bytes: size, sha256: sha });
        }
    }

    Ok(rows
        .into_iter()
        .map(|r| TreeItem {
            kind: Kind::Audiobook.as_str(),
            id: r.id,
            updated_at: r.updated_at,
            owner: Owner { id: r.user_id, display_name: r.owner_name },
            is_owner: r.user_id == viewer.user_id,
            can_delete: can_delete(viewer, r.user_id, r.family_id),
            shared_with_family: r.family_id.is_some(),
            title: r.title,
            path: r.path,
            files: by_book.remove(&r.id).unwrap_or_default(),
            show: None,
            album: None,
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct TrackRow {
    #[sqlx(flatten)]
    track: crate::music::models::MusicTrack,
    owner_name: Option<String>,
    path: String,
}

async fn load_tracks(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<TreeItem>> {
    let sql = format!(
        "SELECT {}, {OWNER_NAME} AS owner_name, p.path
           FROM music_tracks t
           JOIN media_objects mo ON mo.id = t.audio_object_id
           JOIN sync_paths p ON p.kind = 'music_track' AND p.item_id = t.id
          WHERE t.id = ANY($5) AND {}",
        crate::db::music::TRACK_COLS_WITH_CHECKSUM,
        access::VISIBLE
    );
    let rows: Vec<TrackRow> = bind_viewer(sqlx::query_as(&sql), viewer, access::MUSIC)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: sync tree tracks")?;
    Ok(rows
        .into_iter()
        .map(|r| {
            let t = r.track;
            let album_artist = t.effective_album_artist();
            TreeItem {
                kind: Kind::MusicTrack.as_str(),
                id: t.id,
                updated_at: t.updated_at,
                owner: Owner { id: t.user_id, display_name: r.owner_name },
                is_owner: t.user_id == viewer.user_id,
                can_delete: can_delete(viewer, t.user_id, t.family_id),
                shared_with_family: t.family_id.is_some(),
                files: vec![TreeFile { id: t.id, relative_path: String::new(), size_bytes: t.size_bytes, sha256: t.sha256 }],
                album: Some(Album { title: t.album, album_artist, release_group: t.musicbrainz_release_group_id }),
                title: t.title,
                path: r.path,
                show: None,
            }
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct EpisodeRow {
    id: Uuid,
    title: String,
    feed_id: Uuid,
    feed_title: String,
    user_id: Uuid,
    family_id: Option<Uuid>,
    stored_at: DateTime<Utc>,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    owner_name: Option<String>,
    path: String,
}

async fn load_episodes(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<TreeItem>> {
    let sql = format!(
        "SELECT e.id, e.title, t.id AS feed_id, t.title AS feed_title, t.user_id, t.family_id,
                m.created_at AS stored_at, m.size_bytes, m.sha256, {OWNER_NAME} AS owner_name, p.path
           FROM podcast_episodes e
           JOIN podcast_feeds t ON t.id = e.feed_id
           JOIN media_objects m ON m.id = e.audio_object_id
           JOIN sync_paths p ON p.kind = 'podcast_episode' AND p.item_id = e.id
          WHERE e.id = ANY($5) AND {}",
        access::VISIBLE
    );
    let rows: Vec<EpisodeRow> = bind_viewer(sqlx::query_as(&sql), viewer, access::PODCAST)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: sync tree episodes")?;
    Ok(rows
        .into_iter()
        .map(|r| TreeItem {
            kind: Kind::PodcastEpisode.as_str(),
            id: r.id,
            updated_at: r.stored_at,
            owner: Owner { id: r.user_id, display_name: r.owner_name },
            is_owner: r.user_id == viewer.user_id,
            can_delete: can_delete(viewer, r.user_id, r.family_id),
            shared_with_family: r.family_id.is_some(),
            title: r.title,
            path: r.path,
            files: vec![TreeFile { id: r.id, relative_path: String::new(), size_bytes: r.size_bytes, sha256: r.sha256 }],
            show: Some(Show { id: r.feed_id, title: r.feed_title }),
            album: None,
        })
        .collect())
}

#[derive(sqlx::FromRow)]
struct CompanionRow {
    id: Uuid,
    user_id: Uuid,
    family_id: Option<Uuid>,
    updated_at: DateTime<Utc>,
    size_bytes: Option<i64>,
    sha256: Option<String>,
    owner_name: Option<String>,
    path: String,
}

async fn load_companions(pool: &PgPool, viewer: Viewer, ids: &[Uuid]) -> anyhow::Result<Vec<TreeItem>> {
    let sql = format!(
        "SELECT t.id, t.user_id, t.family_id, t.updated_at, m.size_bytes, m.sha256,
                {OWNER_NAME} AS owner_name, p.path
           FROM companion_files t
           JOIN media_objects m ON m.id = t.object_id
           JOIN sync_paths p ON p.kind = 'companion_file' AND p.item_id = t.id
          WHERE t.id = ANY($5) AND {COMPANION_VISIBLE}"
    );
    let rows: Vec<CompanionRow> = bind_viewer(sqlx::query_as(&sql), viewer, access::MUSIC)
        .bind(ids)
        .fetch_all(pool)
        .await
        .context("db: sync tree companion files")?;
    Ok(rows
        .into_iter()
        .map(|r| TreeItem {
            kind: Kind::CompanionFile.as_str(),
            id: r.id,
            updated_at: r.updated_at,
            owner: Owner { id: r.user_id, display_name: r.owner_name },
            is_owner: r.user_id == viewer.user_id,
            can_delete: can_delete(viewer, r.user_id, r.family_id),
            shared_with_family: r.family_id.is_some(),
            title: r.path.rsplit('/').next().unwrap_or("").to_string(),
            files: vec![TreeFile { id: r.id, relative_path: String::new(), size_bytes: r.size_bytes, sha256: r.sha256 }],
            path: r.path,
            show: None,
            album: None,
        })
        .collect())
}

/// The owner's name as the family knows them: their in-family label, else
/// their display name.
const OWNER_NAME: &str = "(SELECT COALESCE(fm.display_label, u.display_name)
                             FROM users u LEFT JOIN family_members fm ON fm.user_id = u.id
                            WHERE u.id = t.user_id)";

fn bind_viewer<'q, O>(
    q: sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments>,
    viewer: Viewer,
    kind: &'q str,
) -> sqlx::query::QueryAs<'q, sqlx::Postgres, O, sqlx::postgres::PgArguments> {
    q.bind(viewer.user_id).bind(viewer.family_id).bind(viewer.is_family_admin).bind(kind)
}

// ── Reconciliation ids (§5.4) ─────────────────────────────────────────────

#[derive(Serialize, sqlx::FromRow)]
pub struct TreeId {
    pub kind: String,
    pub id: Uuid,
    pub updated_at: DateTime<Utc>,
}

/// Every item in the caller's tree, id only, so a client can drop what it
/// holds and the server no longer shows (revoked sharing, a changed member
/// policy, a missed change). Streamed into `out`: at 600,000 tracks it is
/// about 100 MB of JSON.
pub async fn send_all_ids(pool: PgPool, viewer: Viewer, out: tokio::sync::mpsc::Sender<anyhow::Result<TreeId>>) {
    let v = access::VISIBLE;
    let books = v.replace("$4", "'audiobook'");
    let tracks = v.replace("$4", "'music'");
    let feeds = v.replace("$4", "'podcast'");
    let sql = format!(
        "SELECT 'audiobook' AS kind, t.id, t.updated_at FROM audiobook_books t WHERE {books}
         UNION ALL
         SELECT 'music_track', t.id, t.updated_at FROM music_tracks t WHERE {tracks}
         UNION ALL
         SELECT 'podcast_episode', e.id, m.created_at
           FROM podcast_episodes e
           JOIN podcast_feeds t ON t.id = e.feed_id
           JOIN media_objects m ON m.id = e.audio_object_id
          WHERE {feeds}
         UNION ALL
         SELECT 'companion_file', t.id, t.updated_at FROM companion_files t
          WHERE audio2_can_access($1, $2, $3, t.media_kind, t.id, t.user_id, t.family_id)"
    );
    let rows = sqlx::query_as::<_, TreeId>(&sql)
        .bind(viewer.user_id)
        .bind(viewer.family_id)
        .bind(viewer.is_family_admin)
        .fetch(&pool);
    crate::http::json_stream::send_all(rows, out, "db: sync tree ids").await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursors_round_trip() {
        let c = Cursor { full: Some((1, Uuid::new_v4())), from: 42, seq: 7, next: 50, at: 1_700_000_000 };
        assert_eq!(Cursor::decode(&c.encode()), Some(c));
        let d = Cursor { full: None, from: 42, seq: 0, next: 42, at: 1 };
        assert!(!d.encode().contains("full"));
        assert_eq!(Cursor::decode(&d.encode()), Some(d));
        assert_eq!(Cursor::decode("not a cursor"), None);
    }

    #[test]
    fn who_may_delete() {
        let me = Uuid::new_v4();
        let fam = Uuid::new_v4();
        let member = Viewer { user_id: me, family_id: fam, is_family_admin: false };
        let admin = Viewer { is_family_admin: true, ..member };
        let other = Uuid::new_v4();
        assert!(can_delete(member, me, None));
        assert!(!can_delete(member, other, Some(fam)));
        assert!(can_delete(admin, other, Some(fam)));
        assert!(!can_delete(admin, other, None));
    }
}
