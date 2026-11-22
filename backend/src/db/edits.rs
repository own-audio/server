// SPDX-License-Identifier: AGPL-3.0-or-later
//! The history of metadata changes (migration 0114,
//! docs/metadata-organizer-plan.md O0). A write that someone made marks its
//! transaction with [`mark`]; the tables' triggers then keep the fields it
//! changed, before and after. [`undo`] puts them back, only while they still
//! hold what the edit wrote.

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{FromRow, PgConnection, PgPool};
use uuid::Uuid;

/// Who made a change, how, and the batch it belongs to.
#[derive(Debug, Clone, Copy)]
pub struct Edit {
    pub user_id: Uuid,
    /// `manual`, `bulk`, `identify`, `run`, `revert` or `undo`.
    pub source: &'static str,
    pub batch_id: Uuid,
}

impl Edit {
    /// A change on its own: a batch of one save.
    pub fn new(user_id: Uuid, source: &'static str) -> Self {
        Self { user_id, source, batch_id: Uuid::new_v4() }
    }

    pub fn in_batch(user_id: Uuid, source: &'static str, batch_id: Uuid) -> Self {
        Self { user_id, source, batch_id }
    }
}

/// Mark the transaction: what it changes from now on is this edit. Local to
/// the transaction, so a pooled connection never carries it further.
pub async fn mark(conn: &mut PgConnection, edit: &Edit) -> anyhow::Result<()> {
    let ctx = json!({ "user_id": edit.user_id, "source": edit.source, "batch_id": edit.batch_id });
    sqlx::query("SELECT set_config('own.edit', $1, true)")
        .bind(ctx.to_string())
        .execute(conn)
        .await
        .context("db: mark edit")?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
pub struct EditRow {
    pub id: Uuid,
    pub item_kind: String,
    pub item_id: Uuid,
    pub family_id: Option<Uuid>,
    pub user_id: Option<Uuid>,
    pub source: String,
    pub batch_id: Uuid,
    pub before: Value,
    pub after: Value,
    pub at: DateTime<Utc>,
    pub undone_at: Option<DateTime<Utc>>,
    /// How the item is called now, for a list.
    pub item_label: Option<String>,
    /// Whose song it is (tracks only).
    pub item_owner: Option<Uuid>,
}

const COLS: &str = "e.id, e.item_kind, e.item_id, e.family_id, e.user_id, e.source, e.batch_id, e.before, e.after,
    e.at, e.undone_at,
    COALESCE(t.title, a.title, c.name) AS item_label, t.user_id AS item_owner";

const JOINS: &str = "LEFT JOIN music_tracks_all t ON e.item_kind = 'track' AND t.id = e.item_id
    LEFT JOIN albums a ON e.item_kind = 'album' AND a.id = e.item_id
    LEFT JOIN contributors c ON e.item_kind = 'contributor' AND c.id = e.item_id";

/// What a member sees: the household's albums and contributors, their own
/// songs, and every change they made themselves.
const VISIBLE: &str = "(e.family_id = $1 OR e.user_id = $2)
    AND (e.item_kind <> 'track' OR e.user_id = $2 OR t.user_id = $2)";

#[derive(Debug, Default, Clone)]
pub struct Filter {
    pub item: Option<(String, Uuid)>,
    pub batch: Option<Uuid>,
    /// Only changes this user made.
    pub by: Option<Uuid>,
}

pub async fn list(
    pool: &PgPool,
    family_id: Uuid,
    caller: Uuid,
    filter: &Filter,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<EditRow>> {
    sqlx::query_as::<_, EditRow>(&format!(
        "SELECT {COLS} FROM metadata_edits e {JOINS}
          WHERE {VISIBLE}
            AND ($3::text IS NULL OR (e.item_kind = $3 AND e.item_id = $4))
            AND ($5::uuid IS NULL OR e.batch_id = $5)
            AND ($6::uuid IS NULL OR e.user_id = $6)
            AND ($7::timestamptz IS NULL OR (e.at, e.id) < ($7, $8))
          ORDER BY e.at DESC, e.id DESC
          LIMIT $9"
    ))
    .bind(family_id)
    .bind(caller)
    .bind(filter.item.as_ref().map(|i| i.0.clone()))
    .bind(filter.item.as_ref().map(|i| i.1))
    .bind(filter.batch)
    .bind(filter.by)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list edits")
}

/// The edits of a batch, or the one edit, newest first, as the caller sees them.
pub async fn find(pool: &PgPool, family_id: Uuid, caller: Uuid, id: Uuid, whole_batch: bool) -> anyhow::Result<Vec<EditRow>> {
    let by = if whole_batch { "e.batch_id = $3" } else { "e.id = $3" };
    sqlx::query_as::<_, EditRow>(&format!(
        "SELECT {COLS} FROM metadata_edits e {JOINS}
          WHERE {VISIBLE} AND {by}
          ORDER BY e.at DESC, e.id DESC"
    ))
    .bind(family_id)
    .bind(caller)
    .bind(id)
    .fetch_all(pool)
    .await
    .context("db: find edits")
}

/// A batch's summary for the history list: one row per save.
#[derive(Debug, Clone, FromRow)]
pub struct BatchRow {
    pub batch_id: Uuid,
    pub user_id: Option<Uuid>,
    pub source: String,
    pub at: DateTime<Utc>,
    pub items: i64,
    pub undone: bool,
    /// One item's name, for "Abbey Road and 11 more".
    pub first_label: Option<String>,
}

pub async fn batches(
    pool: &PgPool,
    family_id: Uuid,
    caller: Uuid,
    after: Option<(DateTime<Utc>, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<BatchRow>> {
    sqlx::query_as::<_, BatchRow>(&format!(
        "SELECT e.batch_id, (array_agg(e.user_id))[1] AS user_id, (array_agg(e.source))[1] AS source,
                max(e.at) AS at, count(DISTINCT (e.item_kind, e.item_id)) AS items,
                bool_and(e.undone_at IS NOT NULL) AS undone,
                (array_agg(COALESCE(t.title, a.title, c.name) ORDER BY e.at))[1] AS first_label
           FROM metadata_edits e {JOINS}
          WHERE {VISIBLE}
          GROUP BY e.batch_id
         HAVING $3::timestamptz IS NULL OR (max(e.at), e.batch_id) < ($3, $4)
          ORDER BY max(e.at) DESC, e.batch_id DESC
          LIMIT $5"
    ))
    .bind(family_id)
    .bind(caller)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list edit batches")
}

fn table(kind: &str) -> Option<(&'static str, &'static [&'static str])> {
    match kind {
        "track" => Some((
            "music_tracks_all",
            &[
                "title", "artist", "album", "album_artist", "genre", "track_number", "disc_number", "disc_total",
                "is_compilation", "musicbrainz_recording_id", "musicbrainz_release_id", "musicbrainz_artist_id",
                "musicbrainz_release_group_id", "cover_object_id",
            ],
        )),
        "album" => Some((
            "albums",
            &["title", "release_date", "country", "label", "barcode", "is_compilation", "musicbrainz_release_id", "release_group_id"],
        )),
        "contributor" => Some(("contributors", &["name", "sort_name", "kind", "musicbrainz_artist_id", "biography"])),
        _ => None,
    }
}

/// What [`release_identified_artist`] did with a contributor's MusicBrainz artist id.
#[derive(Debug, PartialEq, Eq)]
pub enum ArtistMbid {
    /// Taken away: only an identify gave it, and no song credited to them carries it now.
    /// The edits that gave it, now undone.
    Released(Vec<Uuid>),
    /// A song credited to them still carries it.
    InUse,
    /// Not given by an identify (a file's tags, a hand, an old server), or not theirs now.
    Kept,
}

/// An identify that names a song's MusicBrainz artist also gives the id to the
/// contributor the song is credited to (`music_contributor`, migration 0100), in the
/// identify's batch. When the song's identity is undone, the contributor's id goes
/// with it — unless another song credited to them still carries it, as the other
/// songs of an album identified one by one do; the last one undone takes it.
/// The `mb:` key goes too: left behind, it would point the next identify at this
/// contributor without giving the id back. Call in a marked transaction.
pub async fn release_identified_artist(conn: &mut PgConnection, contributor_id: Uuid, mbid: &str) -> anyhow::Result<ArtistMbid> {
    let family: Option<Uuid> =
        sqlx::query_scalar("SELECT family_id FROM contributors WHERE id = $1 AND musicbrainz_artist_id = $2 FOR UPDATE")
            .bind(contributor_id)
            .bind(mbid)
            .fetch_optional(&mut *conn)
            .await
            .context("db: contributor's artist id")?;
    let Some(family) = family else { return Ok(ArtistMbid::Kept) };
    let given: Vec<Uuid> = sqlx::query_scalar(
        "SELECT id FROM metadata_edits
          WHERE item_kind = 'contributor' AND item_id = $1 AND undone_at IS NULL
            AND source IN ('identify', 'run')
            AND after ->> 'musicbrainz_artist_id' = $2
            AND before -> 'musicbrainz_artist_id' = 'null'::jsonb",
    )
    .bind(contributor_id)
    .bind(mbid)
    .fetch_all(&mut *conn)
    .await
    .context("db: identify gave the artist id")?;
    if given.is_empty() {
        return Ok(ArtistMbid::Kept);
    }
    let in_use: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM credits c JOIN music_tracks_all t ON t.id = c.item_id
                         WHERE c.contributor_id = $1 AND c.item_kind = 'track' AND t.musicbrainz_artist_id = $2)",
    )
    .bind(contributor_id)
    .bind(mbid)
    .fetch_one(&mut *conn)
    .await
    .context("db: artist id still carried")?;
    if in_use {
        return Ok(ArtistMbid::InUse);
    }
    sqlx::query("UPDATE contributors SET musicbrainz_artist_id = NULL, updated_at = now() WHERE id = $1")
        .bind(contributor_id)
        .execute(&mut *conn)
        .await
        .context("db: release artist id")?;
    sqlx::query("DELETE FROM contributor_keys WHERE family_id = $1 AND key = 'mb:' || $2 AND contributor_id = $3")
        .bind(family)
        .bind(mbid)
        .bind(contributor_id)
        .execute(&mut *conn)
        .await
        .context("db: forget artist id key")?;
    sqlx::query("UPDATE metadata_edits SET undone_at = now() WHERE id = ANY($1)")
        .bind(&given)
        .execute(&mut *conn)
        .await
        .context("db: artist id edits undone")?;
    Ok(ArtistMbid::Released(given))
}

/// After a song's MusicBrainz artist id went from `mbid` back to what it was: the
/// contributor of the household who carries `mbid`, released if nothing else needs it.
pub async fn release_track_artist(conn: &mut PgConnection, track_id: Uuid, mbid: &str) -> anyhow::Result<ArtistMbid> {
    let holder: Option<Uuid> = sqlx::query_scalar(
        "SELECT k.id FROM music_tracks_all t
           JOIN contributors k ON k.family_id = music_owner_family(t.user_id) AND k.musicbrainz_artist_id = $2
          WHERE t.id = $1",
    )
    .bind(track_id)
    .bind(mbid)
    .fetch_optional(&mut *conn)
    .await
    .context("db: who carries the artist id")?;
    match holder {
        Some(holder) => release_identified_artist(conn, holder, mbid).await,
        None => Ok(ArtistMbid::Kept),
    }
}

/// The MusicBrainz artist id a track edit took away, when it took one away.
fn artist_id_taken(e: &EditRow) -> Option<&str> {
    let from = e.after.get("musicbrainz_artist_id")?.as_str()?;
    (e.item_kind == "track" && e.before.get("musicbrainz_artist_id").and_then(Value::as_str) != Some(from)).then_some(from)
}

/// A contributor's edit that only gave them a MusicBrainz artist id through an
/// identify: the id, for [`release_identified_artist`].
fn identified_artist_id(e: &EditRow) -> Option<&str> {
    if e.item_kind != "contributor" || !matches!(e.source.as_str(), "identify" | "run") {
        return None;
    }
    let after = e.after.as_object()?;
    if after.len() != 1 || !e.before.get("musicbrainz_artist_id").is_some_and(Value::is_null) {
        return None;
    }
    after.get("musicbrainz_artist_id")?.as_str()
}

#[derive(Debug, PartialEq)]
pub enum UndoOutcome {
    /// How many items went back.
    Undone { items: usize, batch_id: Uuid },
    /// Someone changed these fields since; nothing was undone.
    ChangedSince { item_kind: String, item_id: Uuid, fields: Vec<String> },
    AlreadyUndone,
    /// The item is gone.
    Gone,
}

/// Put back what `edits` changed, newest first, as one batch of its own
/// (`undo`, itself in the history). All or nothing.
pub async fn undo(pool: &PgPool, edits: &[EditRow], by: Uuid) -> anyhow::Result<UndoOutcome> {
    if edits.iter().all(|e| e.undone_at.is_some()) {
        return Ok(UndoOutcome::AlreadyUndone);
    }
    let edit = Edit::new(by, "undo");
    let mut tx = pool.begin().await.context("db: begin undo")?;
    mark(&mut tx, &edit).await?;
    let mut items = std::collections::HashSet::new();
    // Artist id edits a song's undo already took back, in this batch or another.
    let mut released = std::collections::HashSet::new();
    for e in edits.iter().filter(|e| e.undone_at.is_none()) {
        // Newest first: the songs come back before the artist id an identify gave with them.
        if let Some(mbid) = identified_artist_id(e) {
            if released.contains(&e.id) {
                items.insert((e.item_kind.clone(), e.item_id));
                continue;
            }
            match release_identified_artist(&mut tx, e.item_id, mbid).await? {
                ArtistMbid::Released(_) => {
                    items.insert((e.item_kind.clone(), e.item_id));
                    continue;
                }
                // Another song still names them so: the change stays, and stays undoable.
                ArtistMbid::InUse => continue,
                ArtistMbid::Kept => {}
            }
        }
        let Some((table, allowed)) = table(&e.item_kind) else { continue };
        let current: Option<Value> = sqlx::query_scalar(&format!("SELECT to_jsonb(t) FROM {table} t WHERE id = $1 FOR UPDATE"))
            .bind(e.item_id)
            .fetch_optional(&mut *tx)
            .await
            .context("db: read item to undo")?;
        let Some(current) = current else { return Ok(UndoOutcome::Gone) };
        let after = e.after.as_object().cloned().unwrap_or_default();
        let changed: Vec<String> = after
            .iter()
            .filter(|(k, v)| current.get(k.as_str()).unwrap_or(&Value::Null) != *v)
            .map(|(k, _)| k.clone())
            .collect();
        if !changed.is_empty() {
            return Ok(UndoOutcome::ChangedSince { item_kind: e.item_kind.clone(), item_id: e.item_id, fields: changed });
        }
        let fields: Vec<&str> = e
            .before
            .as_object()
            .map(|o| o.keys().filter_map(|k| allowed.iter().find(|a| **a == k).copied()).collect())
            .unwrap_or_default();
        if fields.is_empty() {
            continue;
        }
        let set = fields.iter().map(|f| format!("{f} = r.{f}")).collect::<Vec<_>>().join(", ");
        sqlx::query(&format!(
            "UPDATE {table} t SET {set}, updated_at = now()
               FROM jsonb_populate_record(NULL::{table}, $2) r
              WHERE t.id = $1"
        ))
        .bind(e.item_id)
        .bind(&e.before)
        .execute(&mut *tx)
        .await
        .context("db: undo edit")?;
        if let Some(mbid) = artist_id_taken(e) {
            if let ArtistMbid::Released(ids) = release_track_artist(&mut tx, e.item_id, mbid).await? {
                released.extend(ids);
            }
        }
        if e.item_kind == "track" && e.before.get("musicbrainz_recording_id").is_some_and(Value::is_null) {
            // The song is no longer identified: forget the identify, and a run's
            // accept of it is a decision to make again.
            sqlx::query("DELETE FROM music_track_identity WHERE track_id = $1 AND source = 'identify'")
                .bind(e.item_id)
                .execute(&mut *tx)
                .await
                .context("db: forget identity on undo")?;
            sqlx::query("UPDATE identify_proposals SET decision = 'pending', decided_at = NULL WHERE track_id = $1 AND decision = 'accepted'")
                .bind(e.item_id)
                .execute(&mut *tx)
                .await
                .context("db: reopen proposal on undo")?;
        }
        sqlx::query("UPDATE metadata_edits SET undone_at = now() WHERE id = $1")
            .bind(e.id)
            .execute(&mut *tx)
            .await
            .context("db: mark edit undone")?;
        items.insert((e.item_kind.clone(), e.item_id));
    }
    tx.commit().await.context("db: commit undo")?;
    Ok(UndoOutcome::Undone { items: items.len(), batch_id: edit.batch_id })
}

/// Keep a change made outside a marked transaction — a cover fetched by a
/// helper that writes on its own — as part of `edit`.
pub async fn record(
    pool: &PgPool,
    item_kind: &str,
    item_id: Uuid,
    family_id: Option<Uuid>,
    edit: &Edit,
    before: Value,
    after: Value,
) -> anyhow::Result<()> {
    if before == after {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO metadata_edits (item_kind, item_id, family_id, user_id, source, batch_id, before, after)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(item_kind)
    .bind(item_id)
    .bind(family_id)
    .bind(edit.user_id)
    .bind(edit.source)
    .bind(edit.batch_id)
    .bind(before)
    .bind(after)
    .execute(pool)
    .await
    .context("db: record edit")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(kind: &str, source: &str, before: Value, after: Value) -> EditRow {
        EditRow {
            id: Uuid::new_v4(),
            item_kind: kind.into(),
            item_id: Uuid::new_v4(),
            family_id: None,
            user_id: None,
            source: source.into(),
            batch_id: Uuid::new_v4(),
            before,
            after,
            at: Utc::now(),
            undone_at: None,
            item_label: None,
            item_owner: None,
        }
    }

    #[test]
    fn only_an_identify_that_gave_an_artist_their_id_is_released_with_the_song() {
        let given = |source| row("contributor", source, json!({"musicbrainz_artist_id": null}), json!({"musicbrainz_artist_id": "a1"}));
        assert_eq!(identified_artist_id(&given("identify")), Some("a1"));
        assert_eq!(identified_artist_id(&given("run")), Some("a1"));
        assert_eq!(identified_artist_id(&given("manual")), None, "set by hand: undone like any edit");
        let replaced = row("contributor", "identify", json!({"musicbrainz_artist_id": "a0"}), json!({"musicbrainz_artist_id": "a1"}));
        assert_eq!(identified_artist_id(&replaced), None);
        let more = row("contributor", "identify", json!({"musicbrainz_artist_id": null, "name": "A"}),
                       json!({"musicbrainz_artist_id": "a1", "name": "B"}));
        assert_eq!(identified_artist_id(&more), None);
    }

    #[test]
    fn a_track_undo_names_the_artist_id_it_takes_away() {
        let identify = row("track", "identify", json!({"musicbrainz_artist_id": null, "title": "x"}),
                           json!({"musicbrainz_artist_id": "a1", "title": "y"}));
        assert_eq!(artist_id_taken(&identify), Some("a1"));
        let retitle = row("track", "manual", json!({"title": "x"}), json!({"title": "y"}));
        assert_eq!(artist_id_taken(&retitle), None);
        let cleared = row("track", "manual", json!({"musicbrainz_artist_id": "a1"}), json!({"musicbrainz_artist_id": null}));
        assert_eq!(artist_id_taken(&cleared), None, "undoing a clear gives the id back, takes none");
        let same = row("contributor", "identify", json!({"musicbrainz_artist_id": null}), json!({"musicbrainz_artist_id": "a1"}));
        assert_eq!(artist_id_taken(&same), None);
    }
}
