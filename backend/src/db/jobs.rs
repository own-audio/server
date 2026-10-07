// SPDX-License-Identifier: AGPL-3.0-or-later
use crate::jobs::models::Job;
use anyhow::Context;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

const JOB_COLS: &str =
    "id, job_type, status, payload, result, error,
     attempts, max_attempts, scheduled_at, started_at, completed_at, created_at";

pub async fn enqueue(
    pool: &PgPool,
    job_type: &str,
    payload: Option<&Value>,
    scheduled_at: Option<DateTime<Utc>>,
) -> anyhow::Result<Job> {
    let scheduled_at = scheduled_at.unwrap_or_else(Utc::now);
    sqlx::query_as::<_, Job>(&format!(
        "INSERT INTO jobs (job_type, payload, scheduled_at)
         VALUES ($1, $2, $3)
         RETURNING {JOB_COLS}"
    ))
    .bind(job_type)
    .bind(payload)
    .bind(scheduled_at)
    .fetch_one(pool)
    .await
    .context("db: enqueue job")
}

/// Claims the next due job, optionally restricted to `job_types`. `None`
/// claims any type — the single-container default. `FOR UPDATE SKIP LOCKED`
/// makes this safe when more than one process polls the table concurrently
/// (the `assembler` container added alongside the API container): without
/// it, two pollers' subqueries could both pick the same row before either
/// commits its `UPDATE`.
/// How many jobs of one kind are still waiting or running.
///
/// Used to spot the moment a batch drains, which is when work that only makes
/// sense once per pass — rebuilding a derived view, say — should happen.
pub async fn pending_count(pool: &PgPool, job_type: &str) -> anyhow::Result<i64> {
    sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(*)::bigint FROM jobs
         WHERE job_type = $1 AND status IN ('pending', 'running')",
    )
    .bind(job_type)
    .fetch_one(pool)
    .await
    .context("db: pending job count")
}

pub async fn claim_next(pool: &PgPool, job_types: Option<&[String]>) -> anyhow::Result<Option<Job>> {
    let query = match job_types {
        Some(_) => format!(
            "UPDATE jobs SET status = 'running', started_at = CURRENT_TIMESTAMP, attempts = attempts + 1
             WHERE id = (
                 SELECT id FROM jobs
                 WHERE status = 'pending'
                   AND scheduled_at <= CURRENT_TIMESTAMP
                   AND attempts < max_attempts
                   AND job_type = ANY($1)
                 ORDER BY scheduled_at
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING {JOB_COLS}"
        ),
        None => format!(
            "UPDATE jobs SET status = 'running', started_at = CURRENT_TIMESTAMP, attempts = attempts + 1
             WHERE id = (
                 SELECT id FROM jobs
                 WHERE status = 'pending'
                   AND scheduled_at <= CURRENT_TIMESTAMP
                   AND attempts < max_attempts
                 ORDER BY scheduled_at
                 LIMIT 1
                 FOR UPDATE SKIP LOCKED
             )
             RETURNING {JOB_COLS}"
        ),
    };

    let mut q = sqlx::query_as::<_, Job>(&query);
    if let Some(types) = job_types {
        q = q.bind(types);
    }
    q.fetch_optional(pool).await.context("db: claim next job")
}

pub async fn complete(pool: &PgPool, id: Uuid, result: Option<&Value>) -> anyhow::Result<()> {
    sqlx::query(
        // COALESCE, not a plain overwrite: a job that already called
        // `set_result` (only the caller passes None here today) must keep it.
        "UPDATE jobs SET status = 'completed', result = COALESCE($2, result), completed_at = CURRENT_TIMESTAMP WHERE id = $1",
    )
    .bind(id)
    .bind(result)
    .execute(pool)
    .await
    .context("db: complete job")?;
    Ok(())
}

pub async fn fail(pool: &PgPool, id: Uuid, error: &str) -> anyhow::Result<()> {
    sqlx::query(
        "UPDATE jobs SET status = 'failed', error = $2, completed_at = CURRENT_TIMESTAMP WHERE id = $1",
    )
    .bind(id)
    .bind(error)
    .execute(pool)
    .await
    .context("db: fail job")?;
    Ok(())
}

pub async fn list_recent(pool: &PgPool, limit: i64) -> anyhow::Result<Vec<Job>> {
    sqlx::query_as::<_, Job>(&format!(
        "SELECT {JOB_COLS} FROM jobs ORDER BY created_at DESC LIMIT $1"
    ))
    .bind(limit)
    .fetch_all(pool)
    .await
    .context("db: list recent jobs")
}

pub async fn find_by_id(pool: &PgPool, id: Uuid) -> anyhow::Result<Option<Job>> {
    sqlx::query_as::<_, Job>(&format!(
        "SELECT {JOB_COLS} FROM jobs WHERE id = $1"
    ))
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("db: find job by id")
}

/// Whether a `storage_billing` job for this UTC day already exists in any
/// state. Keeps multiple all-claiming worker containers from each enqueuing
/// the same daily sweep; actual charge idempotency is the ledger's unique
/// index, this is just noise reduction.
pub async fn storage_billing_job_exists(
    pool: &PgPool,
    charge_date: chrono::NaiveDate,
) -> anyhow::Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM jobs
             WHERE job_type = 'storage_billing'
               AND payload ->> 'charge_date' = $1
         )",
    )
    .bind(charge_date.to_string())
    .fetch_one(pool)
    .await
    .context("db: check for existing storage billing job")
}

/// Whether today's storage sweep has already been queued. Same day-level
/// idempotency as the billing gate, so several worker containers polling at
/// once still produce one sweep a day.
pub async fn storage_sweep_job_exists(
    pool: &PgPool,
    run_date: chrono::NaiveDate,
) -> anyhow::Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM jobs
             WHERE job_type = 'storage_sweep'
               AND payload ->> 'run_date' = $1
         )",
    )
    .bind(run_date.to_string())
    .fetch_one(pool)
    .await
    .context("db: check for existing storage sweep job")
}

/// Same day-level idempotency for any daily job whose payload carries
/// `run_date`.
pub async fn daily_job_exists(
    pool: &PgPool,
    job_type: &str,
    run_date: chrono::NaiveDate,
) -> anyhow::Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM jobs WHERE job_type = $1 AND payload ->> 'run_date' = $2
         )",
    )
    .bind(job_type)
    .bind(run_date.to_string())
    .fetch_one(pool)
    .await
    .context("db: check for existing daily job")
}

/// Same day-level idempotency, for the empty-family cleanup sweep.
pub async fn family_cleanup_job_exists(
    pool: &PgPool,
    run_date: chrono::NaiveDate,
) -> anyhow::Result<bool> {
    sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (
             SELECT 1 FROM jobs
             WHERE job_type = 'family_cleanup_sweep'
               AND payload ->> 'run_date' = $1
         )",
    )
    .bind(run_date.to_string())
    .fetch_one(pool)
    .await
    .context("db: check for existing family cleanup sweep job")
}

/// Record a job's output before it finishes — `complete()` preserves this
/// (`COALESCE`) rather than overwriting it with the `None` the generic
/// execute-job wrapper always passes. Lets one job type report something
/// useful (e.g. what the cleanup sweep actually deleted) without changing
/// every other job executor's return type just to thread a result through.
pub async fn set_result(pool: &PgPool, id: Uuid, result: &Value) -> anyhow::Result<()> {
    sqlx::query("UPDATE jobs SET result = $2 WHERE id = $1")
        .bind(id)
        .bind(result)
        .execute(pool)
        .await
        .context("db: set job result")?;
    Ok(())
}

/// Enqueue a feed_refresh job for every active feed that hasn't been refreshed
/// in the last `interval_secs` seconds. Returns the count of new jobs enqueued.
///
/// `last_refreshed_at` only advances once a refresh *succeeds*, so a feed
/// whose refresh is slow or stuck would otherwise look "due" again on every
/// poll and pile up duplicate jobs. `jobs_feed_refresh_inflight_idx` (a
/// partial unique index on (job_type, feed_id) for pending/running rows)
/// makes that impossible at the DB level — `ON CONFLICT DO NOTHING` here
/// just means "skip feeds that already have one in flight."
/// Enqueue one catalogue-sync pass, unless one is already waiting.
///
/// A single batch job rather than one per feed: this is slow-burn catch-up
/// over the whole subscription list, and one job that walks a batch needs no
/// per-payload dedup index of its own.
pub async fn enqueue_podcast_catalog_sync(pool: &PgPool) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "INSERT INTO jobs (job_type, payload, scheduled_at)
         SELECT 'podcast_catalog_sync', '{}'::jsonb, CURRENT_TIMESTAMP
         WHERE NOT EXISTS (
             SELECT 1 FROM jobs
             WHERE job_type = 'podcast_catalog_sync'
               AND status IN ('pending', 'running')
         )",
    )
    .execute(pool)
    .await
    .context("db: enqueue podcast catalog sync")?;

    Ok(result.rows_affected() > 0)
}

/// One `stats_rollup` at a time: the rebuild is idempotent, so a pass that is
/// already queued or running covers this one.
pub async fn enqueue_stats_rollup(pool: &PgPool, days: i32) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "INSERT INTO jobs (job_type, payload, scheduled_at)
         SELECT 'stats_rollup', jsonb_build_object('days', $1::int), CURRENT_TIMESTAMP
         WHERE NOT EXISTS (
             SELECT 1 FROM jobs
             WHERE job_type = 'stats_rollup'
               AND status IN ('pending', 'running')
         )",
    )
    .bind(days)
    .execute(pool)
    .await
    .context("db: enqueue stats rollup")?;

    Ok(result.rows_affected() > 0)
}

pub async fn enqueue_due_feed_refreshes(
    pool: &PgPool,
    interval_secs: i64,
) -> anyhow::Result<u64> {
    let now = Utc::now();
    // Every feed used to be polled on the same flat hourly timer regardless of
    // how often it actually publishes, which is most of the refresh load for
    // no benefit: a show that posts weekly was checked 168 times per episode.
    // The cadence now follows the feed's own most recent episode. Read from
    // our own episodes rather than the catalogue's `newestItemPubdate` on
    // purpose — the catalogue is a weekly snapshot and ours is current.
    //
    // Anything that published in the last week keeps the base interval, so
    // active shows are no slower to arrive than before.
    let feed_ids = sqlx::query_scalar::<_, Uuid>(
        "SELECT f.id
         FROM podcast_feeds f
         LEFT JOIN LATERAL (
             SELECT max(published_at) AS newest
             FROM podcast_episodes e WHERE e.feed_id = f.id
         ) latest ON TRUE
         WHERE f.last_refreshed_at IS NULL
            OR f.last_refreshed_at < CURRENT_TIMESTAMP - make_interval(secs =>
                   $1::bigint * CASE
                       -- No episodes yet: a feed that has never produced one
                       -- may be new rather than dead, so do not slow it down.
                       WHEN latest.newest IS NULL                                     THEN 1
                       WHEN latest.newest > CURRENT_TIMESTAMP - interval '7 days'     THEN 1
                       WHEN latest.newest > CURRENT_TIMESTAMP - interval '30 days'    THEN 6
                       ELSE 24
                   END)",
    )
    .bind(interval_secs)
    .fetch_all(pool)
    .await
    .context("db: list feeds due for refresh")?;

    let mut rows_affected = 0;
    for feed_id in feed_ids {
        let payload = serde_json::json!({ "feed_id": feed_id.to_string() });
        let result = sqlx::query(
            "INSERT INTO jobs (job_type, payload, scheduled_at) VALUES ($1, $2, $3)
             ON CONFLICT (job_type, (payload ->> 'feed_id'))
                 WHERE job_type = 'feed_refresh' AND status IN ('pending', 'running')
             DO NOTHING",
        )
        .bind("feed_refresh")
        .bind(payload)
        .bind(now)
        .execute(pool)
        .await
        .context("db: enqueue feed refresh job")?;

        rows_affected += result.rows_affected();
    }

    Ok(rows_affected)
}

