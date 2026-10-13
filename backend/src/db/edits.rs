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
    for e in edits.iter().filter(|e| e.undone_at.is_none()) {
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
