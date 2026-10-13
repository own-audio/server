// SPDX-License-Identifier: AGPL-3.0-or-later
//! Identify runs, their groups and proposals (migration 0112).

use anyhow::Context;
use serde_json::Value;
use sqlx::{FromRow, PgPool};
use uuid::Uuid;

#[derive(Debug, Clone, FromRow)]
pub struct Run {
    pub id: Uuid,
    pub user_id: Uuid,
    pub family_id: Option<Uuid>,
    pub scope_kind: String,
    pub scope_id: Option<Uuid>,
    pub scope_track_ids: Option<Vec<Uuid>>,
    pub scope_label: Option<String>,
    pub status: String,
    pub error: Option<String>,
    pub tracks: i32,
    pub groups_total: i32,
    pub groups_done: i32,
    pub backoff_level: i16,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub started_at: Option<chrono::DateTime<chrono::Utc>>,
    pub finished_at: Option<chrono::DateTime<chrono::Utc>>,
}

const RUN_COLS: &str = "id, user_id, family_id, scope_kind, scope_id, scope_track_ids, scope_label, status,
    error, tracks, groups_total, groups_done, backoff_level, created_at, started_at, finished_at";

#[allow(clippy::too_many_arguments)]
pub async fn create_run(
    pool: &PgPool,
    user_id: Uuid,
    family_id: Option<Uuid>,
    scope_kind: &str,
    scope_id: Option<Uuid>,
    scope_track_ids: Option<&[Uuid]>,
    scope_label: Option<&str>,
) -> anyhow::Result<Run> {
    sqlx::query_as::<_, Run>(&format!(
        "INSERT INTO identify_runs (user_id, family_id, scope_kind, scope_id, scope_track_ids, scope_label)
         VALUES ($1, $2, $3, $4, $5, $6) RETURNING {RUN_COLS}"
    ))
    .bind(user_id)
    .bind(family_id)
    .bind(scope_kind)
    .bind(scope_id)
    .bind(scope_track_ids)
    .bind(scope_label)
    .fetch_one(pool)
    .await
    .context("db: create identify run")
}

pub async fn find_run(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Run>> {
    sqlx::query_as::<_, Run>(&format!("SELECT {RUN_COLS} FROM identify_runs WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find identify run")
}

pub async fn find_run_owned(pool: &PgPool, id: Uuid, user_id: Uuid) -> anyhow::Result<Option<Run>> {
    sqlx::query_as::<_, Run>(&format!("SELECT {RUN_COLS} FROM identify_runs WHERE id = $1 AND user_id = $2"))
        .bind(id)
        .bind(user_id)
        .fetch_optional(pool)
        .await
        .context("db: find own identify run")
}

pub async fn list_runs(pool: &PgPool, user_id: Uuid, limit: i64) -> anyhow::Result<Vec<Run>> {
    sqlx::query_as::<_, Run>(&format!(
        "SELECT {RUN_COLS} FROM identify_runs WHERE user_id = $1 ORDER BY created_at DESC LIMIT $2"
    ))
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list identify runs")
}

pub async fn set_status(pool: &PgPool, id: Uuid, status: &str, error: Option<&str>) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE identify_runs
            SET status = $2, error = $3,
                started_at = CASE WHEN $2 = 'running' THEN COALESCE(started_at, now()) ELSE started_at END,
                finished_at = CASE WHEN $2 IN ('ready', 'failed', 'cancelled') THEN now() ELSE NULL END,
                backoff_level = CASE WHEN $2 = 'waiting' THEN backoff_level + 1 ELSE 0 END
          WHERE id = $1 AND status NOT IN ('cancelled')",
    )
    .bind(id)
    .bind(status)
    .bind(error)
    .execute(pool)
    .await
    .context("db: set identify run status")?;
    Ok(())
}

/// A track as a run reads it.
#[derive(Debug, Clone, FromRow)]
pub struct ScopeTrack {
    pub id: Uuid,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub album_artist: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub duration_secs: Option<i32>,
    pub album_id: Option<Uuid>,
}

const SCOPE_COLS: &str = "t.id, t.title, t.artist, t.album, t.album_artist, t.track_number, t.disc_number,
    t.duration_secs, t.album_id";

/// The owner's tracks in a run's scope that have no MusicBrainz identity yet —
/// neither matched here nor carrying ids in their files.
pub async fn scope_tracks(pool: &PgPool, run: &Run) -> anyhow::Result<Vec<ScopeTrack>> {
    scope_tracks_of(pool, run.user_id, &run.scope_kind, run.scope_id, run.scope_track_ids.as_deref()).await
}

/// [`scope_tracks`] for a scope with no run yet, to quote it.
pub async fn scope_tracks_of(
    pool: &PgPool,
    user_id: Uuid,
    scope_kind: &str,
    scope_id: Option<Uuid>,
    track_ids: Option<&[Uuid]>,
) -> anyhow::Result<Vec<ScopeTrack>> {
    let filter = match scope_kind {
        "library" => "",
        "folder" => "AND t.id IN (SELECT item_id FROM library_files
                                   WHERE folder_id = $2 AND item_kind = 'track' AND NOT missing)",
        "album" => "AND t.album_id = $2",
        "playlist" => "AND t.id IN (SELECT track_id FROM music_playlist_tracks WHERE playlist_id = $2)",
        "artist" => "AND EXISTS (SELECT 1 FROM credits c
                                  WHERE c.contributor_id = $2
                                    AND ((c.item_kind = 'track' AND c.item_id = t.id)
                                      OR (c.item_kind = 'album' AND c.item_id = t.album_id)))",
        "tracks" => "AND t.id = ANY($3)",
        other => anyhow::bail!("unknown identify scope {other}"),
    };
    sqlx::query_as::<_, ScopeTrack>(&format!(
        "SELECT {SCOPE_COLS} FROM music_tracks t
          WHERE t.user_id = $1
            AND NOT EXISTS (SELECT 1 FROM music_track_identity i WHERE i.track_id = t.id)
            {filter}
          ORDER BY t.album NULLS LAST, t.disc_number NULLS FIRST, t.track_number NULLS LAST, t.title"
    ))
    .bind(user_id)
    .bind(scope_id)
    .bind(track_ids)
    .fetch_all(pool)
    .await
    .context("db: identify scope tracks")
}

pub async fn tracks_by_id(pool: &PgPool, user_id: Uuid, ids: &[Uuid]) -> anyhow::Result<Vec<ScopeTrack>> {
    sqlx::query_as::<_, ScopeTrack>(&format!(
        "SELECT {SCOPE_COLS} FROM music_tracks t WHERE t.user_id = $1 AND t.id = ANY($2)"
    ))
    .bind(user_id)
    .bind(ids)
    .fetch_all(pool)
    .await
    .context("db: identify group tracks")
}

/// Store the groups a run will look up, and its size.
pub async fn insert_groups(
    pool: &PgPool,
    run_id: Uuid,
    groups: &[(&str, String, Vec<Uuid>)],
    tracks: i32,
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    for (i, (kind, label, ids)) in groups.iter().enumerate() {
        sqlx::query(
            "INSERT INTO identify_groups (run_id, kind, label, track_ids, position) VALUES ($1, $2, $3, $4, $5)",
        )
        .bind(run_id)
        .bind(kind)
        .bind(label)
        .bind(ids)
        .bind(i as i32)
        .execute(&mut *tx)
        .await
        .context("db: insert identify group")?;
    }
    sqlx::query("UPDATE identify_runs SET tracks = $2, groups_total = $3 WHERE id = $1")
        .bind(run_id)
        .bind(tracks)
        .bind(groups.len() as i32)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
pub struct Group {
    pub id: Uuid,
    pub run_id: Uuid,
    pub kind: String,
    pub label: String,
    pub track_ids: Vec<Uuid>,
    pub candidates: Value,
    pub chosen: Option<i16>,
}

const GROUP_COLS: &str = "id, run_id, kind, label, track_ids, candidates, chosen";

pub async fn pending_groups(pool: &PgPool, run_id: Uuid, limit: i64) -> anyhow::Result<Vec<Group>> {
    sqlx::query_as::<_, Group>(&format!(
        "SELECT {GROUP_COLS} FROM identify_groups WHERE run_id = $1 AND NOT done ORDER BY position LIMIT $2"
    ))
    .bind(run_id)
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: pending identify groups")
}

pub async fn find_group(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Group>> {
    sqlx::query_as::<_, Group>(&format!("SELECT {GROUP_COLS} FROM identify_groups WHERE id = $1"))
        .bind(id)
        .fetch_optional(pool)
        .await
        .context("db: find identify group")
}

/// One proposal as stored.
pub struct NewProposal<'a> {
    pub track_id: Uuid,
    pub class: &'a str,
    pub reasons: Vec<&'a str>,
    pub proposal: Option<Value>,
    pub keep_album: bool,
    /// Other recordings a single-track lookup found, best first.
    pub alternatives: Value,
}

/// A looked-up group: its candidates, the one chosen, and the proposals for its
/// tracks, replacing any earlier ones for those tracks that are still pending.
pub async fn save_group(
    pool: &PgPool,
    group: &Group,
    candidates: &Value,
    chosen: Option<i16>,
    proposals: &[NewProposal<'_>],
) -> anyhow::Result<()> {
    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE identify_groups SET candidates = $2, chosen = $3, done = true WHERE id = $1")
        .bind(group.id)
        .bind(candidates)
        .bind(chosen)
        .execute(&mut *tx)
        .await
        .context("db: save identify group")?;
    for p in proposals {
        sqlx::query(
            "INSERT INTO identify_proposals (run_id, group_id, track_id, class, reasons, proposal, keep_album, alternatives)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
             ON CONFLICT (run_id, track_id) DO UPDATE
                SET group_id = EXCLUDED.group_id, class = EXCLUDED.class, reasons = EXCLUDED.reasons,
                    proposal = EXCLUDED.proposal, keep_album = EXCLUDED.keep_album,
                    alternatives = EXCLUDED.alternatives
              WHERE identify_proposals.decision = 'pending' AND NOT identify_proposals.chosen_by_owner",
        )
        .bind(group.run_id)
        .bind(group.id)
        .bind(p.track_id)
        .bind(p.class)
        .bind(&p.reasons)
        .bind(&p.proposal)
        .bind(p.keep_album)
        .bind(&p.alternatives)
        .execute(&mut *tx)
        .await
        .context("db: save identify proposal")?;
    }
    sqlx::query(
        "UPDATE identify_runs
            SET groups_done = (SELECT count(*) FROM identify_groups WHERE run_id = $1 AND done)
          WHERE id = $1",
    )
    .bind(group.run_id)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

#[derive(Debug, Clone, FromRow)]
pub struct ProposalRow {
    pub id: Uuid,
    pub run_id: Uuid,
    pub group_id: Uuid,
    pub track_id: Uuid,
    pub class: String,
    pub reasons: Vec<String>,
    pub proposal: Option<Value>,
    pub keep_album: bool,
    pub decision: String,
    pub chosen_by_owner: bool,
    pub alternatives_count: i32,
    pub group_label: String,
    pub group_kind: String,
    pub group_position: i32,
    /// The track as it is now, for showing the change.
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub track_number: Option<i32>,
    pub disc_number: Option<i32>,
    pub duration_secs: Option<i32>,
}

const PROPOSAL_COLS: &str = "p.id, p.run_id, p.group_id, p.track_id, p.class, p.reasons, p.proposal,
    p.keep_album, p.decision, p.chosen_by_owner, jsonb_array_length(p.alternatives) AS alternatives_count,
    g.label AS group_label, g.kind AS group_kind, g.position AS group_position,
    t.title, t.artist, t.album, t.track_number, t.disc_number, t.duration_secs";

/// A run's proposals, a group's together (ordered within it by the caller),
/// filtered by class and decision when given.
pub async fn list_proposals(
    pool: &PgPool,
    run_id: Uuid,
    class: Option<&str>,
    decision: Option<&str>,
    after: Option<(i32, Uuid)>,
    limit: i64,
) -> anyhow::Result<Vec<ProposalRow>> {
    let rows = sqlx::query_as::<_, ProposalRow>(&format!(
        "SELECT {PROPOSAL_COLS}
           FROM identify_proposals p
           JOIN identify_groups g ON g.id = p.group_id
           JOIN music_tracks_all t ON t.id = p.track_id
          WHERE p.run_id = $1
            AND ($2::text IS NULL OR p.class = $2)
            AND ($3::text IS NULL OR p.decision = $3)
            AND ($4::int IS NULL OR (g.position, p.id) > ($4, $5))
          ORDER BY g.position, p.id
          LIMIT $6"
    ))
    .bind(run_id)
    .bind(class)
    .bind(decision)
    .bind(after.map(|a| a.0))
    .bind(after.map(|a| a.1))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list identify proposals")?;
    Ok(rows)
}

pub async fn find_proposal(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<ProposalRow>> {
    sqlx::query_as::<_, ProposalRow>(&format!(
        "SELECT {PROPOSAL_COLS}
           FROM identify_proposals p
           JOIN identify_groups g ON g.id = p.group_id
           JOIN music_tracks_all t ON t.id = p.track_id
          WHERE p.id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("db: find identify proposal")
}

pub async fn set_decision(pool: &PgPool, id: Uuid, decision: &str) -> anyhow::Result<()> {
    sqlx::query("UPDATE identify_proposals SET decision = $2, decided_at = now() WHERE id = $1")
        .bind(id)
        .bind(decision)
        .execute(pool)
        .await
        .context("db: decide identify proposal")?;
    Ok(())
}

pub async fn set_keep_album(pool: &PgPool, id: Uuid, keep: bool) -> anyhow::Result<()> {
    sqlx::query("UPDATE identify_proposals SET keep_album = $2 WHERE id = $1 AND decision = 'pending'")
        .bind(id)
        .bind(keep)
        .execute(pool)
        .await
        .context("db: set keep_album")?;
    Ok(())
}

/// The other recordings stored with a proposal.
pub async fn alternatives(pool: &PgPool, id: Uuid) -> anyhow::Result<Value> {
    sqlx::query_scalar("SELECT alternatives FROM identify_proposals WHERE id = $1")
        .bind(id)
        .fetch_one(pool)
        .await
        .context("db: identify alternatives")
}

/// What the owner's own search found, in place of the run's.
pub async fn set_alternatives(pool: &PgPool, id: Uuid, alternatives: &Value) -> anyhow::Result<()> {
    sqlx::query("UPDATE identify_proposals SET alternatives = $2 WHERE id = $1")
        .bind(id)
        .bind(alternatives)
        .execute(pool)
        .await
        .context("db: set identify alternatives")?;
    Ok(())
}

/// The owner picked the recording: it becomes the proposal, pending again if it
/// was rejected, and stays theirs when the group gets another edition. Not for
/// an accepted proposal: that is undone first.
pub async fn set_owner_choice(pool: &PgPool, id: Uuid, proposal: &Value) -> anyhow::Result<bool> {
    let n = sqlx::query(
        "UPDATE identify_proposals
            SET proposal = $2, class = 'review', reasons = '{chosen}', keep_album = false,
                chosen_by_owner = true, decision = 'pending', decided_at = NULL
          WHERE id = $1 AND decision <> 'accepted'",
    )
    .bind(id)
    .bind(proposal)
    .execute(pool)
    .await
    .context("db: owner's identify choice")?
    .rows_affected();
    Ok(n == 1)
}

/// Pending proposals of a run in one class, for accepting them all.
pub async fn pending_ids(pool: &PgPool, run_id: Uuid, class: &str) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT id FROM identify_proposals WHERE run_id = $1 AND class = $2 AND decision = 'pending'",
    )
    .bind(run_id)
    .bind(class)
    .fetch_all(pool)
    .await
    .context("db: pending identify proposals")
}

/// Counts per class and decision, for a run's summary.
pub async fn counts(pool: &PgPool, run_id: Uuid) -> anyhow::Result<Vec<(String, String, i64)>> {
    sqlx::query_as(
        "SELECT class, decision, count(*) FROM identify_proposals WHERE run_id = $1 GROUP BY class, decision",
    )
    .bind(run_id)
    .fetch_all(pool)
    .await
    .context("db: identify run counts")
}

/// Tracks a run's accepts identified that still carry that identity, for
/// reverting the run as one.
pub async fn identified_by_run(pool: &PgPool, run_id: Uuid) -> anyhow::Result<Vec<Uuid>> {
    sqlx::query_scalar("SELECT track_id FROM music_track_identity WHERE run_id = $1")
        .bind(run_id)
        .fetch_all(pool)
        .await
        .context("db: tracks identified by run")
}

pub async fn delete_run(pool: &PgPool, run_id: Uuid) -> anyhow::Result<()> {
    sqlx::query("DELETE FROM identify_runs WHERE id = $1")
        .bind(run_id)
        .execute(pool)
        .await
        .context("db: delete identify run")?;
    Ok(())
}
