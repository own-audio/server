// SPDX-License-Identifier: AGPL-3.0-or-later
/// The 30-day trash — docs/file-sync-plan.md §5.1, migration 0077.
///
/// Books, tracks and playlists live in `…_all` tables behind views that hide
/// trashed rows, so this module is the only place that reads `…_all`. A stored
/// podcast episode is trashed as a copy: its object moves to
/// `trashed_audio_object_id` and the episode row stays in the feed.
use anyhow::Context;
use chrono::{DateTime, Utc};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

/// How long an item waits in the trash before `trash_purge` deletes it.
pub const RETENTION_DAYS: i64 = 30;

/// How long a deletion tombstone is kept for delta-syncing clients. Well past
/// the trash period, so a device offline for a while still learns what went.
pub const TOMBSTONE_RETENTION_DAYS: i64 = 180;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Kind {
    Audiobook,
    MusicTrack,
    Playlist,
    PodcastEpisode,
    CompanionFile,
}

impl Kind {
    pub const ALL: [Kind; 5] =
        [Kind::Audiobook, Kind::MusicTrack, Kind::Playlist, Kind::PodcastEpisode, Kind::CompanionFile];

    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "audiobook" => Some(Kind::Audiobook),
            "music_track" => Some(Kind::MusicTrack),
            "playlist" => Some(Kind::Playlist),
            "podcast_episode" => Some(Kind::PodcastEpisode),
            "companion_file" => Some(Kind::CompanionFile),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Audiobook => "audiobook",
            Kind::MusicTrack => "music_track",
            Kind::Playlist => "playlist",
            Kind::PodcastEpisode => "podcast_episode",
            Kind::CompanionFile => "companion_file",
        }
    }

    /// `deleted_items.media_kind` for the tombstone. A stored episode has
    /// none: the episode itself is not gone, only its stored copy.
    pub fn tombstone_kind(self) -> Option<&'static str> {
        match self {
            Kind::Audiobook => Some("audiobook"),
            Kind::MusicTrack => Some("music"),
            Kind::Playlist => Some("playlist"),
            Kind::PodcastEpisode | Kind::CompanionFile => None,
        }
    }

    /// The `content_grants` kind that decides whether someone can see it,
    /// for the kinds that have one.
    pub fn access_kind(self) -> Option<&'static str> {
        match self {
            Kind::Audiobook => Some(super::access::AUDIOBOOK),
            Kind::MusicTrack => Some(super::access::MUSIC),
            // Companion files carry no grants; being shared with the family
            // is what makes them visible.
            Kind::Playlist | Kind::CompanionFile => None,
            Kind::PodcastEpisode => Some(super::access::PODCAST),
        }
    }
}

/// A live item a delete is about to act on.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Target {
    pub owner_id: Uuid,
    pub family_id: Option<Uuid>,
    pub title: String,
    /// The id `access::can_listen` checks: the item itself, or the feed for
    /// an episode.
    pub access_id: Uuid,
}

/// A trashed item.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Trashed {
    pub kind: String,
    pub id: Uuid,
    pub owner_id: Uuid,
    pub owner_name: Option<String>,
    pub family_id: Option<Uuid>,
    pub title: String,
    pub trashed_at: DateTime<Utc>,
    pub trashed_by: Option<Uuid>,
    pub trashed_by_name: Option<String>,
    pub trash_batch: Option<Uuid>,
    pub size_bytes: i64,
}

impl Trashed {
    pub fn kind(&self) -> Kind {
        Kind::parse(&self.kind).expect("trash SQL only produces known kinds")
    }

    pub fn purge_at(&self) -> DateTime<Utc> {
        self.trashed_at + chrono::Duration::days(RETENTION_DAYS)
    }
}

/// Size of an item's stored objects, per kind, as a SQL expression over the
/// row aliased `x`.
const BOOK_SIZE: &str = "(COALESCE((SELECT SUM(m.size_bytes) FROM audiobook_files f
                                     JOIN media_objects m ON m.id = f.audio_object_id
                                    WHERE f.book_id = x.id), 0)
                        + COALESCE((SELECT m.size_bytes FROM media_objects m WHERE m.id = x.cover_object_id), 0))";
const TRACK_SIZE: &str = "(COALESCE((SELECT m.size_bytes FROM media_objects m WHERE m.id = x.audio_object_id), 0)
                         + COALESCE((SELECT m.size_bytes FROM media_objects m WHERE m.id = x.cover_object_id), 0))";
const PLAYLIST_SIZE: &str = "COALESCE((SELECT m.size_bytes FROM media_objects m WHERE m.id = x.cover_object_id), 0)";
const COMPANION_SIZE: &str = "COALESCE((SELECT m.size_bytes FROM media_objects m WHERE m.id = x.object_id), 0)";
/// A companion file's title is its file name.
const COMPANION_TITLE: &str = "COALESCE((SELECT regexp_replace(p.path, '^.*/', '') FROM sync_paths p
                                        WHERE p.kind = 'companion_file' AND p.item_id = x.id), 'File')";
const EPISODE_SIZE: &str = "COALESCE((SELECT m.size_bytes FROM media_objects m WHERE m.id = x.trashed_audio_object_id), 0)";

/// Every trashed item as one union with the columns of [`Trashed`], for the
/// caller to filter.
fn trashed_union() -> String {
    let names = "(SELECT u.display_name FROM users u WHERE u.id = {owner}) AS owner_name,
                 x.trashed_at, x.trashed_by,
                 (SELECT u.display_name FROM users u WHERE u.id = x.trashed_by) AS trashed_by_name,
                 x.trash_batch";
    let owner = |col: &str| names.replace("{owner}", col);
    format!(
        "SELECT 'audiobook' AS kind, x.id, x.user_id AS owner_id, {bo}, x.family_id, x.title,
                {BOOK_SIZE}::BIGINT AS size_bytes
           FROM audiobook_books_all x WHERE x.trashed_at IS NOT NULL
         UNION ALL
         SELECT 'music_track', x.id, x.user_id, {to}, x.family_id, x.title, {TRACK_SIZE}::BIGINT
           FROM music_tracks_all x WHERE x.trashed_at IS NOT NULL
         UNION ALL
         SELECT 'playlist', x.id, x.user_id, {po}, x.family_id, x.name, {PLAYLIST_SIZE}::BIGINT
           FROM music_playlists_all x WHERE x.trashed_at IS NOT NULL
         UNION ALL
         SELECT 'podcast_episode', x.id, f.user_id, {eo}, f.family_id, x.title, {EPISODE_SIZE}::BIGINT
           FROM podcast_episodes x JOIN podcast_feeds f ON f.id = x.feed_id
          WHERE x.trashed_at IS NOT NULL
         UNION ALL
         SELECT 'companion_file', x.id, x.user_id, {co}, x.family_id, {COMPANION_TITLE}, {COMPANION_SIZE}::BIGINT
           FROM companion_files_all x WHERE x.trashed_at IS NOT NULL",
        bo = owner("x.user_id"),
        to = owner("x.user_id"),
        po = owner("x.user_id"),
        eo = owner("f.user_id"),
        co = owner("x.user_id"),
    )
}

const TRASHED_COLS: &str = "kind, id, owner_id, owner_name, family_id, title, trashed_at, trashed_by,
                            trashed_by_name, trash_batch, size_bytes";

/// A live (not trashed) item, for a delete to decide on. Reads the views, so a
/// trashed item is `None` exactly like a missing one.
pub async fn find_live(pool: &PgPool, kind: Kind, id: Uuid) -> anyhow::Result<Option<Target>> {
    let sql = match kind {
        Kind::Audiobook => "SELECT user_id AS owner_id, family_id, title, id AS access_id
                              FROM audiobook_books WHERE id = $1",
        Kind::MusicTrack => "SELECT user_id AS owner_id, family_id, title, id AS access_id
                               FROM music_tracks WHERE id = $1",
        Kind::Playlist => "SELECT user_id AS owner_id, family_id, name AS title, id AS access_id
                             FROM music_playlists WHERE id = $1",
        Kind::PodcastEpisode => "SELECT f.user_id AS owner_id, f.family_id, e.title, f.id AS access_id
                                   FROM podcast_episodes e JOIN podcast_feeds f ON f.id = e.feed_id
                                  WHERE e.id = $1 AND e.audio_object_id IS NOT NULL",
        Kind::CompanionFile => "SELECT c.user_id AS owner_id, c.family_id,
                                       COALESCE(regexp_replace(p.path, '^.*/', ''), 'File') AS title, c.id AS access_id
                                  FROM companion_files c
                                  LEFT JOIN sync_paths p ON p.kind = 'companion_file' AND p.item_id = c.id
                                 WHERE c.id = $1",
    };
    sqlx::query_as::<_, Target>(sql)
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find item to trash")
}

/// Move an item to the trash. False when it was not live (already trashed,
/// gone, or — for an episode — not stored).
pub async fn move_to_trash(
    pool: &PgPool,
    kind: Kind,
    id: Uuid,
    actor: Uuid,
    batch: Uuid,
) -> anyhow::Result<bool> {
    // updated_at moves so /library/changes stops listing it on the next delta.
    let sql = match kind {
        Kind::Audiobook => "UPDATE audiobook_books_all
                               SET trashed_at = now(), trashed_by = $2, trash_batch = $3, updated_at = now()
                             WHERE id = $1 AND trashed_at IS NULL",
        Kind::MusicTrack => "UPDATE music_tracks_all
                                SET trashed_at = now(), trashed_by = $2, trash_batch = $3, updated_at = now()
                              WHERE id = $1 AND trashed_at IS NULL",
        Kind::Playlist => "UPDATE music_playlists_all
                              SET trashed_at = now(), trashed_by = $2, trash_batch = $3, updated_at = now()
                            WHERE id = $1 AND trashed_at IS NULL",
        // A previously trashed copy that was never purged is replaced: the
        // episode was stored again in the meantime and trashed a second time.
        // The older object is left for the storage sweep.
        Kind::PodcastEpisode => "UPDATE podcast_episodes
                                    SET trashed_audio_object_id = audio_object_id, audio_object_id = NULL,
                                        trashed_at = now(), trashed_by = $2, trash_batch = $3
                                  WHERE id = $1 AND audio_object_id IS NOT NULL",
        Kind::CompanionFile => "UPDATE companion_files_all
                                   SET trashed_at = now(), trashed_by = $2, trash_batch = $3, updated_at = now()
                                 WHERE id = $1 AND trashed_at IS NULL",
    };
    let done = sqlx::query(sql)
        .bind(id)
        .bind(actor)
        .bind(batch)
        .execute(pool)
        .await
        .context("db: move to trash")?;
    Ok(done.rows_affected() > 0)
}

/// One trashed item.
pub async fn find_trashed(pool: &PgPool, kind: Kind, id: Uuid) -> anyhow::Result<Option<Trashed>> {
    let sql = format!(
        "SELECT {TRASHED_COLS} FROM ({}) t WHERE t.kind = $1 AND t.id = $2",
        trashed_union()
    );
    sqlx::query_as::<_, Trashed>(&sql)
        .bind(kind.as_str())
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find trashed item")
}

/// Which trashed items a list shows.
#[derive(Debug, Clone, Copy)]
pub enum Scope {
    /// Everything this user owns.
    Owner(Uuid),
    /// Everything that was shared with this family — the family admin's view.
    Family(Uuid),
}

pub async fn list(pool: &PgPool, scope: Scope) -> anyhow::Result<Vec<Trashed>> {
    let (filter, id) = match scope {
        Scope::Owner(u) => ("t.owner_id = $1", u),
        Scope::Family(f) => ("t.family_id = $1", f),
    };
    let sql = format!(
        "SELECT {TRASHED_COLS} FROM ({}) t WHERE {filter} ORDER BY t.trashed_at DESC",
        trashed_union()
    );
    sqlx::query_as::<_, Trashed>(&sql)
        .bind(id)
        .fetch_all(pool)
        .await
        .context("db: list trash")
}

/// Everything deleted together in one batch.
pub async fn list_batch(pool: &PgPool, batch: Uuid) -> anyhow::Result<Vec<Trashed>> {
    let sql = format!(
        "SELECT {TRASHED_COLS} FROM ({}) t WHERE t.trash_batch = $1 ORDER BY t.trashed_at",
        trashed_union()
    );
    sqlx::query_as::<_, Trashed>(&sql)
        .bind(batch)
        .fetch_all(pool)
        .await
        .context("db: list trash batch")
}

/// Items whose 30 days are up, oldest first.
pub async fn list_expired(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<Trashed>> {
    let sql = format!(
        "SELECT {TRASHED_COLS} FROM ({}) t
          WHERE t.trashed_at < now() - make_interval(days => $1::int)
          ORDER BY t.trashed_at LIMIT $2",
        trashed_union()
    );
    sqlx::query_as::<_, Trashed>(&sql)
        .bind(RETENTION_DAYS as i32)
        .bind(limit)
        .fetch_all(pool)
        .await
        .context("db: list expired trash")
}

/// Take an item out of the trash. Returns objects that are now surplus — a
/// trashed episode copy whose episode was stored again meanwhile — for the
/// caller to delete if nothing else references them.
///
/// A shared item whose owner has since moved to another family comes back
/// private rather than shared with a family the owner is no longer in.
pub async fn restore(pool: &PgPool, kind: Kind, id: Uuid) -> anyhow::Result<Option<Vec<Uuid>>> {
    let mut tx: Transaction<'_, Postgres> = pool.begin().await.context("db: begin restore")?;
    let mut surplus = Vec::new();
    let restored = match kind {
        Kind::Audiobook | Kind::MusicTrack | Kind::Playlist | Kind::CompanionFile => {
            let table = match kind {
                Kind::Audiobook => "audiobook_books_all",
                Kind::MusicTrack => "music_tracks_all",
                Kind::CompanionFile => "companion_files_all",
                _ => "music_playlists_all",
            };
            let sql = format!(
                "UPDATE {table}
                    SET trashed_at = NULL, trashed_by = NULL, trash_batch = NULL, updated_at = now(),
                        family_id = CASE
                            WHEN {table}.family_id IS NOT NULL AND {table}.family_id IS DISTINCT FROM
                                 (SELECT fm.family_id FROM family_members fm WHERE fm.user_id = {table}.user_id)
                            THEN NULL ELSE {table}.family_id END
                  WHERE id = $1 AND trashed_at IS NOT NULL"
            );
            sqlx::query(&sql).bind(id).execute(&mut *tx).await.context("db: restore")?.rows_affected() > 0
        }
        Kind::PodcastEpisode => {
            let row: Option<(Option<Uuid>, Option<Uuid>)> = sqlx::query_as(
                "SELECT audio_object_id, trashed_audio_object_id FROM podcast_episodes
                  WHERE id = $1 AND trashed_at IS NOT NULL FOR UPDATE",
            )
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .context("db: lock trashed episode")?;
            match row {
                None => false,
                Some((current, trashed)) => {
                    if current.is_some() {
                        surplus.extend(trashed);
                    }
                    sqlx::query(
                        "UPDATE podcast_episodes
                            SET audio_object_id = COALESCE(audio_object_id, trashed_audio_object_id),
                                trashed_audio_object_id = NULL,
                                trashed_at = NULL, trashed_by = NULL, trash_batch = NULL
                          WHERE id = $1",
                    )
                    .bind(id)
                    .execute(&mut *tx)
                    .await
                    .context("db: restore episode")?;
                    true
                }
            }
        }
    };
    tx.commit().await.context("db: commit restore")?;
    Ok(restored.then_some(surplus))
}

/// Delete a trashed item for good. Returns the objects it referenced, for the
/// caller to delete where nothing else still references them.
pub async fn purge(pool: &PgPool, kind: Kind, id: Uuid) -> anyhow::Result<Option<Vec<Uuid>>> {
    let mut tx = pool.begin().await.context("db: begin purge")?;
    let objects: Vec<Option<Uuid>> = match kind {
        Kind::Audiobook => sqlx::query_scalar(
            "SELECT cover_object_id FROM audiobook_books_all WHERE id = $1 AND trashed_at IS NOT NULL
             UNION ALL
             SELECT f.audio_object_id FROM audiobook_files f
               JOIN audiobook_books_all b ON b.id = f.book_id
              WHERE b.id = $1 AND b.trashed_at IS NOT NULL",
        ),
        Kind::MusicTrack => sqlx::query_scalar(
            "SELECT unnest(ARRAY[audio_object_id, cover_object_id]) FROM music_tracks_all
              WHERE id = $1 AND trashed_at IS NOT NULL",
        ),
        Kind::Playlist => sqlx::query_scalar(
            "SELECT cover_object_id FROM music_playlists_all WHERE id = $1 AND trashed_at IS NOT NULL",
        ),
        Kind::PodcastEpisode => sqlx::query_scalar(
            "SELECT trashed_audio_object_id FROM podcast_episodes WHERE id = $1 AND trashed_at IS NOT NULL",
        ),
        Kind::CompanionFile => sqlx::query_scalar(
            "SELECT object_id FROM companion_files_all WHERE id = $1 AND trashed_at IS NOT NULL",
        ),
    }
    .bind(id)
    .fetch_all(&mut *tx)
    .await
    .context("db: collect objects to purge")?;

    let sql = match kind {
        Kind::Audiobook => "DELETE FROM audiobook_books_all WHERE id = $1 AND trashed_at IS NOT NULL",
        Kind::MusicTrack => "DELETE FROM music_tracks_all WHERE id = $1 AND trashed_at IS NOT NULL",
        Kind::Playlist => "DELETE FROM music_playlists_all WHERE id = $1 AND trashed_at IS NOT NULL",
        Kind::CompanionFile => "DELETE FROM companion_files_all WHERE id = $1 AND trashed_at IS NOT NULL",
        Kind::PodcastEpisode => "UPDATE podcast_episodes
                                    SET trashed_audio_object_id = NULL, trashed_at = NULL,
                                        trashed_by = NULL, trash_batch = NULL
                                  WHERE id = $1 AND trashed_at IS NOT NULL",
    };
    let gone = sqlx::query(sql).bind(id).execute(&mut *tx).await.context("db: purge")?.rows_affected() > 0;
    tx.commit().await.context("db: commit purge")?;
    Ok(gone.then(|| objects.into_iter().flatten().collect()))
}

/// Record one restore — the charge and the monitoring both read these.
#[allow(clippy::too_many_arguments)]
pub async fn record_restore(
    pool: &PgPool,
    family_id: Uuid,
    user_id: Uuid,
    kind: Kind,
    item_id: Uuid,
    size_bytes: i64,
    days_in_trash: i64,
    charged_micro: i64,
) -> anyhow::Result<()> {
    sqlx::query(
        "INSERT INTO trash_restores
             (family_id, user_id, media_kind, item_id, size_bytes, days_in_trash, charged_micro)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(family_id)
    .bind(user_id)
    .bind(kind.as_str())
    .bind(item_id)
    .bind(size_bytes)
    .bind(days_in_trash as i32)
    .bind(charged_micro)
    .execute(pool)
    .await
    .context("db: record restore")?;
    Ok(())
}

/// Whole days an item spent in the trash. Undo within the day costs nothing.
pub fn days_in_trash(trashed_at: DateTime<Utc>, now: DateTime<Utc>) -> i64 {
    (now - trashed_at).num_days().max(0)
}

/// One family's trash, for the instance admin.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct FamilyTrashStats {
    pub family_id: Uuid,
    pub family_name: String,
    pub trashed_items: i64,
    pub trashed_bytes: i64,
    pub restores_30d: i64,
    pub restored_bytes_30d: i64,
    /// The most times any one item was restored in 30 days.
    pub max_restores_one_item_30d: i64,
}

pub async fn family_stats(pool: &PgPool) -> anyhow::Result<Vec<FamilyTrashStats>> {
    let sql = format!(
        "WITH t AS (
             SELECT fm.family_id, x.size_bytes
               FROM ({}) x JOIN family_members fm ON fm.user_id = x.owner_id
         ),
         r AS (
             SELECT family_id, COUNT(*) AS n, SUM(size_bytes) AS bytes
               FROM trash_restores WHERE created_at > now() - interval '30 days'
              GROUP BY family_id
         ),
         per_item AS (
             SELECT family_id, MAX(n) AS n FROM (
                 SELECT family_id, item_id, COUNT(*) AS n FROM trash_restores
                  WHERE created_at > now() - interval '30 days'
                  GROUP BY family_id, item_id) s
              GROUP BY family_id
         )
         SELECT f.id AS family_id, f.name AS family_name,
                (SELECT COUNT(*) FROM t WHERE t.family_id = f.id) AS trashed_items,
                COALESCE((SELECT SUM(t.size_bytes) FROM t WHERE t.family_id = f.id), 0)::BIGINT AS trashed_bytes,
                COALESCE(r.n, 0) AS restores_30d,
                COALESCE(r.bytes, 0)::BIGINT AS restored_bytes_30d,
                COALESCE(p.n, 0) AS max_restores_one_item_30d
           FROM families f
           LEFT JOIN r ON r.family_id = f.id
           LEFT JOIN per_item p ON p.family_id = f.id
          ORDER BY trashed_bytes DESC, f.name",
        trashed_union()
    );
    sqlx::query_as::<_, FamilyTrashStats>(&sql)
        .fetch_all(pool)
        .await
        .context("db: family trash stats")
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn kinds_round_trip() {
        for k in Kind::ALL {
            assert_eq!(Kind::parse(k.as_str()), Some(k));
        }
        assert_eq!(Kind::parse("music"), None, "the API kind is music_track, not the tombstone kind");
    }

    #[test]
    fn undo_the_same_day_is_free() {
        let t = Utc.with_ymd_and_hms(2026, 9, 25, 10, 0, 0).unwrap();
        let now = t + chrono::Duration::hours(23);
        assert_eq!(days_in_trash(t, now), 0);
    }


    #[test]
    fn purge_date_is_thirty_days_on() {
        let t = Utc.with_ymd_and_hms(2026, 9, 25, 8, 0, 0).unwrap();
        let item = Trashed {
            kind: "music_track".into(),
            id: Uuid::nil(),
            owner_id: Uuid::nil(),
            owner_name: None,
            family_id: None,
            title: String::new(),
            trashed_at: t,
            trashed_by: None,
            trashed_by_name: None,
            trash_batch: None,
            size_bytes: 0,
        };
        assert_eq!(item.purge_at(), Utc.with_ymd_and_hms(2026, 10, 25, 8, 0, 0).unwrap());
    }
}
