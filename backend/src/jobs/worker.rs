// SPDX-License-Identifier: AGPL-3.0-or-later
/// Background job worker — polls the `jobs` table and executes due tasks.
///
/// Currently handles:
///   - `feed_refresh`: re-fetches an RSS or YouTube-backed feed and syncs new episodes
///   - `media_checksum`: DEDUPLICATION_PLAN.md P2 — hashes one object's stored bytes
use anyhow::Context;
use crate::app::AppConfig;
use crate::db;
use crate::hooks::{Hooks, JobHandler};
use crate::podcasts; // access to fetch_rss + ingest_episodes helpers
use crate::storage::ObjectStore;
use sqlx::PgPool;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{debug, error, info, warn};

/// How often the worker polls for new jobs when the queue was empty (seconds).
const POLL_INTERVAL_SECS: u64 = 30;

/// How long to wait before looking again when the last pass *did* find work.
///
/// The poll interval is a fine idle cadence and a terrible drain rate: with one
/// concurrent slot it means one job every 30 s however fast each job is. Audio
/// analysis takes about a second per track, so a library backfill would have
/// run at 30 s per track — 20 000 tracks in 167 hours instead of 6.
///
/// This does not weaken the memory bound. In-flight work is capped by the
/// semaphore, not by the sleep; polling sooner only closes the gap between
/// sequential jobs.
const BUSY_POLL_INTERVAL_SECS: u64 = 1;

/// How often each feed should be refreshed automatically (seconds).
const FEED_REFRESH_INTERVAL_SECS: i64 = 3600; // 1 hour

/// How often the checksum backfill sweep runs (seconds) — deliberately much
/// less often than the main poll: this is a slow-burn catch-up over
/// potentially the whole library, not time-sensitive the way a feed refresh
/// is. 1 hour, same cadence as feed refresh.
const CHECKSUM_SWEEP_INTERVAL_SECS: u64 = 3600;

/// How often the podcast catalogue backfill runs (seconds). Six hours: the
/// catalogue itself is only refreshed weekly, so anything faster is asking the
/// same question about the same feeds and getting the same answer.
const CATALOG_SYNC_INTERVAL_SECS: u64 = 6 * 3600;
/// How often `stats_rollup` rebuilds the listening rollup and the per-track
/// affinity that smart playlists filter and weight on ("not played lately",
/// "the family plays", weighted shuffle). Nothing enqueued it before, so those
/// signals never reflected real listening.
const STATS_ROLLUP_INTERVAL_SECS: u64 = 3600;
/// The window both rebuilds read. Every pass overwrites play counts with the
/// counts inside this window, so it has to be the longest one a rule asks
/// about — a year, for "forgotten favourites" and Wrapped.
const STATS_ROLLUP_DAYS: i32 = 365;

/// How stale a feed's catalogue metadata may get before it is re-asked. Seven
/// days, matching the dump's own refresh cadence.
const CATALOG_STALE_AFTER_DAYS: i64 = 7;

/// Feeds enriched per catalogue-sync pass. Each one is an HTTP call to the
/// metadata service, so this is deliberately small — the next pass continues
/// where this one stopped.
const CATALOG_SYNC_BATCH_SIZE: i64 = 100;

/// How often the audiobook-duration sweep runs (seconds). Hourly, like the other catch-ups:
/// a book with no length is wrong on every screen that lists it, but it is wrong slowly.
const BOOK_DURATION_SWEEP_INTERVAL_SECS: u64 = 3600;
const RELEASE_BACKFILL_SWEEP_INTERVAL_SECS: u64 = 3600;
const RELEASE_BACKFILL_BATCH_SIZE: i64 = 200;

/// Files measured per pass. Each is one ffprobe over a presigned URL, so this can be larger
/// than the analysis batch — the probe reads a header, not the file.
const BOOK_DURATION_SWEEP_BATCH_SIZE: i64 = 200;

/// How often the audio-analysis sweep runs (seconds). Same reasoning as the
/// checksum sweep: a slow-burn catch-up over a library, not time-sensitive.
const ANALYSIS_SWEEP_INTERVAL_SECS: u64 = 3600;

/// How many tracks one analysis sweep pass enqueues. Each is a fetch and a
/// decode, so this is deliberately modest; the next pass continues where this
/// one stopped, and the queue itself is a query (`analysis_version IS NULL`),
/// so nothing is lost if the worker dies mid-pass.
const ANALYSIS_SWEEP_BATCH_SIZE: i64 = 200;

/// How many missing-checksum objects one sweep pass enqueues — caps the
/// burst of work a single tick can create against a large pre-existing
/// library; the next sweep picks up wherever this one left off.
const CHECKSUM_SWEEP_BATCH_SIZE: i64 = 200;

/// Launch the worker as a detached Tokio task.
/// Call this once from `app::run()` after the pool is ready.
pub fn spawn(pool: PgPool, storage: ObjectStore, config: AppConfig, hooks: Arc<dyn Hooks>) {
    tokio::spawn(async move {
        run_loop(pool, storage, config, hooks).await;
    });
}

/// Job types contributed by the edition (`Hooks::job_handlers`), consulted
/// before the core's own `match` in `execute_job`.
type Handlers = Arc<std::collections::HashMap<&'static str, Arc<dyn JobHandler>>>;

async fn run_loop(pool: PgPool, storage: ObjectStore, config: AppConfig, hooks: Arc<dyn Hooks>) {
    let handlers: Handlers = Arc::new(hooks.job_handlers().into_iter().collect());
    let job_types = config.worker_job_types();
    // `WORKER_JOB_TYPES=""` is how the API-only container opts out entirely:
    // no claims, no sweeps, no polling.
    if job_types.as_ref().is_some_and(|types| types.is_empty()) {
        info!("job worker disabled: WORKER_JOB_TYPES is empty");
        return;
    }
    // The claim loop used to take every due job in a single tick and spawn
    // them all, so a checksum sweep could put CHECKSUM_SWEEP_BATCH_SIZE
    // downloads in flight at once — on a shared 4 GB host that is an OOM,
    // not a throughput win. Unclaimed jobs just stay pending for the next tick.
    let max_concurrent = config.worker_max_concurrent_jobs();
    info!(job_types = ?job_types, max_concurrent, "job worker started");
    // Not a fixed `interval`: the delay depends on whether the previous pass
    // found anything, so that a queue with work in it drains at the speed of
    // the work rather than at the idle cadence.
    let mut claimed_last_pass = false;
    // Ticks rather than its own `tokio::time::interval` — keeps the sweep simple to reason
    // about alongside the feed-refresh enqueue right below it, both driven off the same poll
    // cadence instead of two independently-firing timers.
    let sweep_every_n_ticks = (CHECKSUM_SWEEP_INTERVAL_SECS / POLL_INTERVAL_SECS).max(1);
    let mut ticks_since_sweep: u64 = sweep_every_n_ticks; // sweep on the first tick too
    // Whether the last sweep found work. While it did, the next batch goes in
    // as soon as the previous one has drained instead of an hour later: at 200
    // an hour a 600,000-track first import took about four months.
    let mut checksum_backlog = false;
    let mut analysis_backlog = false;

    let analysis_sweep_every_n_ticks = (ANALYSIS_SWEEP_INTERVAL_SECS / POLL_INTERVAL_SECS).max(1);
    let mut ticks_since_analysis_sweep: u64 = analysis_sweep_every_n_ticks;
    let duration_sweep_every_n_ticks =
        (BOOK_DURATION_SWEEP_INTERVAL_SECS / POLL_INTERVAL_SECS).max(1);
    let mut ticks_since_duration_sweep: u64 = duration_sweep_every_n_ticks;
    let release_sweep_every_n_ticks =
        (RELEASE_BACKFILL_SWEEP_INTERVAL_SECS / POLL_INTERVAL_SECS).max(1);
    let mut ticks_since_release_sweep: u64 = release_sweep_every_n_ticks;

    let catalog_sync_every_n_ticks = (CATALOG_SYNC_INTERVAL_SECS / POLL_INTERVAL_SECS).max(1);
    // Runs on the first tick too, so a deployment that has just gained the
    // catalogue backfills without waiting six hours for the first pass.
    let mut ticks_since_catalog_sync: u64 = catalog_sync_every_n_ticks;
    let stats_rollup_every_n_ticks = (STATS_ROLLUP_INTERVAL_SECS / POLL_INTERVAL_SECS).max(1);
    let mut ticks_since_stats_rollup: u64 = stats_rollup_every_n_ticks;

    // The UTC day the billing enqueue last ran for — in-process only, so a
    // restart re-checks (harmlessly: the jobs-table lookup and the ledger's
    // unique index both absorb repeats).
    let mut sweep_checked_for: Option<chrono::NaiveDate> = None;
    let mut family_cleanup_checked_for: Option<chrono::NaiveDate> = None;
    let mut trash_purge_checked_for: Option<chrono::NaiveDate> = None;

    let slots = Arc::new(Semaphore::new(max_concurrent));

    loop {
        tokio::time::sleep(Duration::from_secs(if claimed_last_pass {
            BUSY_POLL_INTERVAL_SECS
        } else {
            POLL_INTERVAL_SECS
        }))
        .await;

        // 1. Enqueue feed_refresh jobs for feeds due for a refresh
        match db::jobs::enqueue_due_feed_refreshes(&pool, FEED_REFRESH_INTERVAL_SECS).await {
            Ok(n) if n > 0 => debug!(count = n, "enqueued feed refresh jobs"),
            Ok(_) => {}
            Err(e) => warn!("failed to enqueue feed refreshes: {e}"),
        }

        // 1b. DEDUPLICATION_PLAN.md P2's backfill — periodically enqueue a checksum job for
        // any audio object that still doesn't have one (objects that predate this column, or
        // slipped in through an upload path that doesn't enqueue its own). Batched and on a
        // much longer cadence than the main poll: this is a slow-burn catch-up over
        // potentially the whole library, not time-sensitive work.
        //
        // Gated on this container actually handling `media_checksum` (or claiming everything,
        // `job_types: None`) — every worker instance runs this same loop (the `assembler`
        // container included, per `docker-compose.yml`'s split), and without this check each
        // one independently re-discovers and re-enqueues the same still-unhashed objects on
        // every sweep tick, producing duplicate jobs that only one of them can ever claim.
        // Caught live: a fresh restart of both containers hashed several tracks twice before
        // this gate was added.
        // Same gate, same reason as the checksum sweep below: every worker
        // container runs this loop, and an ungated enqueue means each one
        // independently queues the same pass.
        let handles_catalog_sync = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "podcast_catalog_sync"));
        ticks_since_catalog_sync += 1;
        if handles_catalog_sync && ticks_since_catalog_sync >= catalog_sync_every_n_ticks {
            ticks_since_catalog_sync = 0;
            match db::jobs::enqueue_podcast_catalog_sync(&pool).await {
                Ok(true) => debug!("enqueued podcast catalog sync"),
                Ok(false) => {}
                Err(e) => warn!("failed to enqueue podcast catalog sync: {e}"),
            }
        }

        let handles_stats_rollup = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "stats_rollup"));
        ticks_since_stats_rollup += 1;
        if handles_stats_rollup && ticks_since_stats_rollup >= stats_rollup_every_n_ticks {
            ticks_since_stats_rollup = 0;
            match db::jobs::enqueue_stats_rollup(&pool, STATS_ROLLUP_DAYS).await {
                Ok(true) => debug!("enqueued stats rollup"),
                Ok(false) => {}
                Err(e) => warn!("failed to enqueue stats rollup: {e}"),
            }
        }

        let handles_media_checksum = job_types.as_ref().is_none_or(|types| types.iter().any(|t| t == "media_checksum"));
        ticks_since_sweep += 1;
        if handles_media_checksum
            && (ticks_since_sweep >= sweep_every_n_ticks
                || (checksum_backlog && db::jobs::pending_count(&pool, "media_checksum").await.is_ok_and(|n| n == 0)))
        {
            ticks_since_sweep = 0;
            checksum_backlog = false;
            match db::media::audio_objects_missing_checksum(&pool, CHECKSUM_SWEEP_BATCH_SIZE).await {
                Ok(ids) if !ids.is_empty() => {
                    checksum_backlog = true;
                    let count = ids.len();
                    for object_id in ids {
                        let payload = serde_json::json!({ "media_object_id": object_id.to_string() });
                        if let Err(e) = db::jobs::enqueue(&pool, "media_checksum", Some(&payload), None).await {
                            warn!(%object_id, "failed to enqueue media checksum job: {e}");
                        }
                    }
                    info!(count, "checksum sweep enqueued jobs for objects missing a hash");
                }
                Ok(_) => {}
                Err(e) => warn!("checksum sweep query failed: {e}"),
            }
        }

        // 1b'. The edition's own scheduled work (the hosted edition enqueues
        // its daily storage charge here). Errors are logged, never fatal.
        if let Err(e) = hooks.worker_tick(&pool, &storage, chrono::Utc::now()).await {
            warn!("edition worker tick failed: {e:#}");
        }

        // 1c. Daily family storage billing — one `storage_billing` job per UTC
        // day, enqueued on the first tick at or after 00:00 UTC so the charge
        // reflects the storage stand at (within a poll interval of) midnight.
        // Same multi-container gate as the checksum sweep; the day-level
        // idempotency itself lives in the ledger's unique index, so even a
        // duplicate job double-executes into a no-op rather than a double
        // charge. Days the worker slept through entirely are deliberately not
        // back-charged: a charge is a snapshot at its own midnight, and that
        // snapshot can't be reconstructed later — skipping is the
        // customer-favorable failure.
        // 1d. Daily storage sweep — reclaim objects nothing references. One a
        // day is plenty: this is cleanup, not something anyone waits on, and
        // the age threshold means a day's delay changes nothing.
        let handles_sweep = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "storage_sweep"));
        if handles_sweep {
            let today = chrono::Utc::now().date_naive();
            if sweep_checked_for != Some(today) {
                match db::jobs::storage_sweep_job_exists(&pool, today).await {
                    Ok(true) => sweep_checked_for = Some(today),
                    Ok(false) => {
                        let payload = serde_json::json!({ "run_date": today.to_string() });
                        match db::jobs::enqueue(&pool, "storage_sweep", Some(&payload), None).await {
                            Ok(_) => {
                                info!(%today, "storage sweep job enqueued");
                                sweep_checked_for = Some(today);
                            }
                            Err(e) => warn!("failed to enqueue storage sweep job: {e}"),
                        }
                    }
                    Err(e) => warn!("storage sweep enqueue check failed: {e}"),
                }
            }
        }

        // 1e. Daily empty-family cleanup sweep — a pure safety net, not the
        // primary mechanism. Every path that can empty a family already
        // prunes it inline (see db::families::delete_if_empty's doc comment);
        // this only exists to catch whatever a future bug in one of those
        // misses. Same day-gated enqueue shape as the storage sweep above.
        let handles_family_cleanup = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "family_cleanup_sweep"));
        if handles_family_cleanup {
            let today = chrono::Utc::now().date_naive();
            if family_cleanup_checked_for != Some(today) {
                match db::jobs::family_cleanup_job_exists(&pool, today).await {
                    Ok(true) => family_cleanup_checked_for = Some(today),
                    Ok(false) => {
                        let payload = serde_json::json!({ "run_date": today.to_string() });
                        match db::jobs::enqueue(&pool, "family_cleanup_sweep", Some(&payload), None).await {
                            Ok(_) => {
                                info!(%today, "family cleanup sweep job enqueued");
                                family_cleanup_checked_for = Some(today);
                            }
                            Err(e) => warn!("failed to enqueue family cleanup sweep job: {e}"),
                        }
                    }
                    Err(e) => warn!("family cleanup sweep enqueue check failed: {e}"),
                }
            }
        }

        // 1f. Daily trash purge — deletes what has been in the trash for 30
        // days, with its objects, and prunes old deletion tombstones. Same
        // day-gated enqueue shape as the sweeps above.
        let handles_trash_purge = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "trash_purge"));
        if handles_trash_purge {
            let today = chrono::Utc::now().date_naive();
            if trash_purge_checked_for != Some(today) {
                match db::jobs::daily_job_exists(&pool, "trash_purge", today).await {
                    Ok(true) => trash_purge_checked_for = Some(today),
                    Ok(false) => {
                        let payload = serde_json::json!({ "run_date": today.to_string() });
                        match db::jobs::enqueue(&pool, "trash_purge", Some(&payload), None).await {
                            Ok(_) => {
                                info!(%today, "trash purge job enqueued");
                                trash_purge_checked_for = Some(today);
                            }
                            Err(e) => warn!("failed to enqueue trash purge job: {e}"),
                        }
                    }
                    Err(e) => warn!("trash purge enqueue check failed: {e}"),
                }
            }
        }

        // 1c. Audio-analysis catch-up for families that asked for it.
        //
        // UNLIKE the checksum sweep above, this one is GUARDED. `media_checksum`
        // runs for every object unconditionally; measuring a library is work
        // someone opted into (migration 0073), so this looks only at families
        // that enabled it. Dropping the guard would make the opt-in decoration
        // and measure everything anyway — the easiest mistake to make when
        // copying the sweep above.
        let handles_analysis = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "analyze_audio"));

        if handles_analysis
            && (ticks_since_analysis_sweep >= analysis_sweep_every_n_ticks
                || (analysis_backlog && db::jobs::pending_count(&pool, "analyze_audio").await.is_ok_and(|n| n == 0)))
        {
            ticks_since_analysis_sweep = 0;
            analysis_backlog = false;
            match db::music::families_wanting_analysis(&pool).await {
                Ok(families) => {
                    for family_id in families {
                        match db::music::enqueue_family_analysis(
                            &pool,
                            family_id,
                            crate::music::analysis::ANALYSIS_VERSION,
                            ANALYSIS_SWEEP_BATCH_SIZE,
                        )
                        .await
                        {
                            Ok(n) if n > 0 => {
                                analysis_backlog = true;
                                info!(%family_id, count = n, "analysis sweep enqueued tracks")
                            }
                            Ok(_) => {}
                            Err(e) => warn!(%family_id, "analysis sweep failed: {e}"),
                        }
                    }
                }
                Err(e) => warn!("could not list families wanting analysis: {e}"),
            }
        } else if handles_analysis {
            ticks_since_analysis_sweep += 1;
        }

        // 1d. Audiobook files whose duration nobody recorded.
        //
        // Unguarded, unlike 1c: a book's length is not a measurement someone opts into, it is
        // what every shelf and detail screen shows. The column is filled from a field the
        // *uploading client* supplies, so it is only ever as good as the client that wrote it —
        // Audiobookshelf imports dropped it entirely until audio2-mac d6531ac, and the books
        // that had already arrived stayed blank because nothing ever went back for them.
        let handles_duration = job_types
            .as_ref()
            .is_none_or(|types| types.iter().any(|t| t == "book_file_duration"));

        if handles_duration && ticks_since_duration_sweep >= duration_sweep_every_n_ticks {
            ticks_since_duration_sweep = 0;
            match db::audiobooks::enqueue_missing_file_durations(
                &pool,
                BOOK_DURATION_SWEEP_BATCH_SIZE,
            )
            .await
            {
                Ok(n) if n > 0 => info!(count = n, "duration sweep enqueued audiobook files"),
                Ok(_) => {}
                Err(e) => warn!("audiobook duration sweep failed: {e}"),
            }
        } else if handles_duration {
            ticks_since_duration_sweep += 1;
        }

        // 1e. Album artist for tracks identified before the metadata service returned the
        // release's artist credit (docs/album-artist-plan.md A2). Unguarded like 1d — it is what
        // files a track under its album — but pointless without a metadata service at all.
        let handles_release_backfill = config.metadata.is_some()
            && job_types
                .as_ref()
                .is_none_or(|types| types.iter().any(|t| t == "music_release_backfill"));
        if handles_release_backfill && ticks_since_release_sweep >= release_sweep_every_n_ticks {
            ticks_since_release_sweep = 0;
            match db::music::enqueue_release_backfill(&pool, RELEASE_BACKFILL_BATCH_SIZE).await {
                Ok(n) if n > 0 => info!(count = n, "release backfill enqueued identified tracks"),
                Ok(_) => {}
                Err(e) => warn!("music release backfill sweep failed: {e}"),
            }
        } else if handles_release_backfill {
            ticks_since_release_sweep += 1;
        }

        // 2. Claim and execute pending jobs, up to `max_concurrent` in flight.
        // The permit is taken before the claim so a job is never marked running
        // with nowhere to run it.
        claimed_last_pass = false;
        loop {
            let Ok(permit) = slots.clone().try_acquire_owned() else {
                // Every slot is busy. That still counts as "there was work", so
                // the next look comes quickly rather than 30 s after a job that
                // may finish in one.
                claimed_last_pass = true;
                break;
            };
            match db::jobs::claim_next(&pool, job_types.as_deref()).await {
                Ok(Some(job)) => {
                    claimed_last_pass = true;
                    let pool2 = pool.clone();
                    let storage2 = storage.clone();
                    let config2 = config.clone();
                    let handlers2 = handlers.clone();
                    let job_id = job.id;
                    let job_type = job.job_type.clone();
                    tokio::spawn(async move {
                        let _permit = permit;
                        debug!(%job_id, %job_type, "executing job");
                        match execute_job(&pool2, &storage2, &config2, &handlers2, &job).await {
                            Ok(()) => {
                                info!(%job_id, %job_type, "job completed");
                                let _ = db::jobs::complete(&pool2, job_id, None).await;
                            }
                            Err(e) => {
                                // `{e:#}` — the whole chain, not just the outermost context.
                                // A failure recorded as "probing audiobook file <id>" says what
                                // was being attempted and omits the only part worth reading,
                                // which was "Connection refused".
                                let detail = format!("{e:#}");
                                error!(%job_id, %job_type, error = %detail, "job failed");
                                let _ = db::jobs::fail(&pool2, job_id, &detail).await;
                            }
                        }
                    });
                }
                Ok(None) => break, // no more pending jobs
                Err(e) => {
                    warn!("failed to claim job: {e}");
                    break;
                }
            }
        }
    }
}

async fn execute_job(
    pool: &PgPool,
    storage: &ObjectStore,
    config: &AppConfig,
    handlers: &Handlers,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    if let Some(handler) = handlers.get(job.job_type.as_str()) {
        return handler.run(pool, storage, job).await;
    }
    match job.job_type.as_str() {
        "feed_refresh" => execute_feed_refresh(pool, storage, job).await,
        "stats_rollup" => execute_stats_rollup(pool, job).await,
        "media_checksum" => execute_media_checksum(pool, storage, job).await,
        "analyze_audio" => execute_analyze_audio(pool, storage, job).await,
        "book_file_duration" => execute_book_file_duration(pool, storage, job).await,
        "music_release_backfill" => execute_music_release_backfill(pool, config, job).await,
        "podcast_catalog_sync" => execute_podcast_catalog_sync(pool, config, job).await,
        "storage_sweep" => execute_storage_sweep(pool, storage, job).await,
        "trash_purge" => execute_trash_purge(pool, storage).await,
        "episode_download" => execute_episode_download(pool, storage, job).await,
        "storage_reconcile" => execute_storage_reconcile(pool, storage, job).await,
        "family_cleanup_sweep" => execute_family_cleanup_sweep(pool, job).await,
        // Neither the core nor the edition handles this type. Fail it rather
        // than complete it: a silently "done" job hid misconfigured
        // WORKER_JOB_TYPES lists for a day once, and an edition that forgot to
        // register a handler should find out at the first job, not never.
        other => anyhow::bail!("unhandled job type {other}"),
    }
}

/// DEDUPLICATION_PLAN.md P2 — reads the object's stored bytes back from S3/Garage and records
/// their SHA-256. Streams the body through the hasher rather than loading it: this used to
/// buffer whole files, and a run of large audiobooks left the canary holding 1.5 GB it never
/// gave back (see `ObjectStore::sha256_hex`).
/// Measure one audiobook file and fill in the duration nobody recorded.
///
/// Reads the object through a presigned URL rather than fetching it: ffprobe only needs the
/// header, and an audiobook file is large enough that downloading whole books to learn their
/// length would be the expensive way to answer a cheap question.
///
/// **A probe that could not read the file fails the job.** The temptation is to record zero and
/// move on so the queue cannot loop, but the two cases are not the same: ffprobe answering "this
/// container has no duration" is a fact about the file, while ffprobe not reaching the storage at
/// all is a fact about the deployment. Recording zero for the second writes a wrong answer that
/// looks like a real one and that no later sweep will revisit, because the column is no longer
/// NULL. Caught on the local stack, where presigned URLs resolve to `localhost:3900` inside the
/// container and every file would have been stamped zero seconds. Failing instead lets the job
/// retry and, once `max_attempts` is spent, leaves the reason in `jobs.error` where it can be
/// read.
/// Re-reads one identified track's release and stores its artist credit as the album artist.
///
/// A service that doesn't send a release-group id is older than the field: the track is left
/// unmarked so the sweep asks again once the service is updated, rather than being stamped as
/// checked with nothing learned.
async fn execute_music_release_backfill(
    pool: &PgPool,
    config: &AppConfig,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let track_id: uuid::Uuid = job
        .payload
        .as_ref()
        .and_then(|p| p.get("track_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing track_id in payload"))?
        .parse()?;
    let Some(metadata) = config.metadata.as_ref() else {
        return Ok(());
    };
    let Some((recording_id, release_id)) = db::music::release_backfill_target(pool, track_id).await? else {
        debug!(%track_id, "track gone or no longer identified, skipping release backfill");
        return Ok(());
    };

    let detail = crate::metadata::mirror::MetadataMirror::new(metadata)
        .recording(&recording_id, release_id.as_deref())
        .await
        .with_context(|| format!("re-reading the release of track {track_id}"))?;

    if release_id.is_some() && detail.mb_release_group_id.is_none() {
        debug!(%track_id, "metadata service sent no release group yet; leaving for a later sweep");
        return Ok(());
    }
    db::music::set_release_info(
        pool,
        track_id,
        detail.album_artist.as_deref(),
        detail.mb_release_group_id.as_deref(),
    )
    .await
}

async fn execute_book_file_duration(
    pool: &PgPool,
    storage: &ObjectStore,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let file_id: uuid::Uuid = job
        .payload
        .as_ref()
        .and_then(|p| p.get("file_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing file_id in payload"))?
        .parse()?;

    let Some((book_id, object_key)) = db::audiobooks::file_audio_key(pool, file_id).await? else {
        debug!(%file_id, "audiobook file or its audio object is gone, skipping");
        return Ok(());
    };

    let url = storage.presigned_get(&object_key, 3600).await?;
    let secs = probe_duration_secs(&url)
        .await
        .with_context(|| format!("probing audiobook file {file_id}"))?
        .unwrap_or_else(|| {
            // Probed fine, but the container declares no duration — a property of this file, and
            // an answer. Zero is not NULL, so the sweep stops offering it.
            warn!(%file_id, "ffprobe read the file but found no duration, recording zero");
            0
        });

    db::audiobooks::set_file_duration(pool, file_id, Some(secs)).await?;
    db::audiobooks::recalculate_total_duration(pool, book_id).await?;

    debug!(%file_id, %book_id, secs, "audiobook file duration measured");
    Ok(())
}

async fn probe_duration_secs(input: &str) -> anyhow::Result<Option<i32>> {
    let out = tokio::process::Command::new("ffprobe")
        .args([
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=nw=1:nk=1",
            input,
        ])
        .output()
        .await
        .context("spawn ffprobe")?;

    if !out.status.success() {
        // ffprobe echoes the URL it was given, signature and all, and this message is written to
        // `jobs.error` where it stays. The query string is what makes it a credential; the host
        // and path are what make it diagnosable, so only the query string goes.
        let stderr = String::from_utf8_lossy(&out.stderr);
        let redacted = stderr
            .split_whitespace()
            .map(|word| match word.split_once('?') {
                Some((before, _)) if word.starts_with("http") => format!("{before}?…"),
                _ => word.to_string(),
            })
            .collect::<Vec<_>>()
            .join(" ");
        anyhow::bail!("ffprobe failed: {}", redacted.trim());
    }

    Ok(String::from_utf8_lossy(&out.stdout)
        .trim()
        .parse::<f64>()
        .ok()
        .filter(|d| d.is_finite() && *d >= 0.0)
        .map(|d| d.round() as i32))
}

async fn execute_media_checksum(
    pool: &PgPool,
    storage: &ObjectStore,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let media_object_id_str = job
        .payload
        .as_ref()
        .and_then(|p| p.get("media_object_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing media_object_id in payload"))?;
    let media_object_id: uuid::Uuid = media_object_id_str.parse()?;

    let Some(object_key) = db::media::object_key(pool, media_object_id).await? else {
        // The object row is gone (track deleted between enqueue and now) — nothing to hash,
        // and not a failure: the job did exactly what it could with a target that no longer
        // exists.
        debug!(%media_object_id, "media object no longer exists, skipping checksum");
        return Ok(());
    };

    let sha256_hex = storage.sha256_hex(&object_key).await?;
    db::media::set_checksum(pool, media_object_id, &sha256_hex).await?;

    debug!(%media_object_id, sha256 = %sha256_hex, "media checksum computed");
    Ok(())
}

/// Measure one track's acoustic properties.
///
/// Same shape as `media_checksum`: fetch the object, compute, store. The
/// difference is that a result of "nothing measurable" is still a result — a
/// file that will not decode, or music with no detectable beat, gets its
/// `analysis_version` written with NULL measurements so the queue does not hand
/// it back forever.
///
/// The whole object is fetched today. Analysis only needs ~60 s from the
/// middle, so a ranged GET would cut this by roughly an order of magnitude
/// against R2 — `ObjectStore` has no range method yet, and the plan tracks it
/// as an open item rather than a silent inefficiency.
async fn execute_analyze_audio(
    pool: &PgPool,
    storage: &ObjectStore,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let track_id: uuid::Uuid = job
        .payload
        .as_ref()
        .and_then(|p| p.get("track_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing track_id in payload"))?
        .parse()?;

    let Some(object_key) = db::music::track_audio_key(pool, track_id).await? else {
        // Deleted between enqueue and now. Not a failure.
        debug!(%track_id, "track or audio object is gone, skipping analysis");
        return Ok(());
    };

    // Streamed to disk, not held: a FLAC is 100–300 MB, times the job concurrency.
    let temp = storage.download_to_temp(&object_key).await?;

    let version = crate::music::analysis::ANALYSIS_VERSION;
    let analysis = match crate::music::analysis::analyze(temp.path()).await {
        Ok(a) => a,
        Err(e) => {
            // Decode failures are a property of the file, not a transient
            // fault. Record the attempt so the queue moves on, and say why.
            warn!(%track_id, "audio analysis failed, recording as unmeasurable: {e:#}");
            crate::music::analysis::TrackAnalysis::default()
        }
    };

    db::music::set_track_analysis(pool, track_id, &analysis, version).await?;

    // Energy is a percentile over the whole library, so every measurement
    // shifts it for every track — but refreshing per track would rebuild the
    // view twenty thousand times during a backfill. So refresh once, when the
    // queue drains.
    //
    // This is not an optimisation, it is the difference between the feature
    // working and appearing empty. Leaving it to the nightly rollup meant that
    // after a backfill finished, every energy-filtered playlist returned
    // nothing until the small hours — and the UI, correctly, reported that as
    // "nothing in the library fits". Found by running a real request against a
    // freshly measured library, not by a test.
    //
    // The count is one indexed query against a job queue, next to a job that
    // just decoded a minute of audio.
    match db::jobs::pending_count(pool, "analyze_audio").await {
        // This job is already marked running, so it counts itself.
        Ok(n) if n <= 1 => {
            if let Err(e) = db::music::refresh_energy(pool).await {
                warn!("could not refresh energy after the analysis pass: {e:#}");
            } else {
                info!("analysis queue drained, energy scale rebuilt");
            }
        }
        Ok(_) => {}
        Err(e) => warn!("could not check the analysis queue: {e:#}"),
    }

    debug!(
        %track_id,
        bpm = ?analysis.bpm,
        lufs = ?analysis.loudness_lufs,
        "track analysed"
    );
    Ok(())
}

/// Delete objects nothing references any more, bytes first.
///
/// Deleting a row never used to free its object, and some paths never delete
/// at all — an upload abandoned before `from-uploads` leaves the file behind
/// with nothing pointing at it. Reclaiming at each delete site helps future
/// deletes but cannot touch what has already accumulated, so this is the
/// backstop that actually bounds storage.
///
/// Bytes go before the row: if the row went first and the delete failed, the
/// object would be left with nothing recording that it exists. This order
/// leaves at worst a row whose object is already gone, which the next sweep
/// finishes off, and every step is idempotent.
async fn execute_storage_sweep(
    pool: &PgPool,
    storage: &ObjectStore,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let payload = job.payload.as_ref();
    let older_than_days = payload
        .and_then(|p| p.get("older_than_days"))
        .and_then(|v| v.as_i64())
        .unwrap_or(7);
    let limit = payload
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_i64())
        .unwrap_or(500);

    let candidates = crate::db::media::find_unreferenced(pool, older_than_days, limit).await?;
    if candidates.is_empty() {
        info!(older_than_days, "storage sweep: nothing to reclaim");
        return Ok(());
    }

    let mut freed = 0usize;
    let mut failed = 0usize;
    for (id, key) in &candidates {
        if let Err(e) = storage.delete(key).await {
            warn!(%key, error = %e, "storage sweep: could not delete object, leaving its row alone");
            failed += 1;
            continue;
        }
        if let Err(e) = crate::db::media::delete_object_row(pool, *id).await {
            warn!(%key, error = %e, "storage sweep: object deleted but its row stayed");
            failed += 1;
            continue;
        }
        freed += 1;
    }

    info!(
        freed,
        failed,
        considered = candidates.len(),
        older_than_days,
        "storage sweep finished"
    );
    Ok(())
}

/// Safety-net sweep for families the normal inline pruning missed — see the
/// enqueue site's comment. Writes what it deleted to the job's `result`
/// (visible in the admin console's Jobs tab) only when it actually found
/// something; an empty run leaves `result` null, same as every other job
/// type, so a quiet day doesn't clutter the list with "deleted 0" rows.
async fn execute_family_cleanup_sweep(
    pool: &PgPool,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let candidates = crate::db::families::find_empty(pool).await?;
    if candidates.is_empty() {
        info!("family cleanup sweep: nothing to remove");
        return Ok(());
    }

    let mut deleted_names = Vec::new();
    for (id, name) in &candidates {
        match crate::db::families::delete_if_empty(pool, *id).await {
            // A concurrent request may have given it a member since the
            // query above ran — not an error, just nothing to do here.
            Ok(true) => deleted_names.push(name.clone()),
            Ok(false) => {}
            Err(e) => warn!(%id, error = %e, "family cleanup sweep: could not delete"),
        }
    }

    info!(
        deleted = deleted_names.len(),
        considered = candidates.len(),
        "family cleanup sweep finished"
    );

    if !deleted_names.is_empty() {
        let result = serde_json::json!({ "deleted_families": deleted_names });
        crate::db::jobs::set_result(pool, job.id, &result).await?;
    }
    Ok(())
}

/// Delete bucket objects the database has never heard of.
///
/// The twin of `storage_sweep`, from the other side. The sweep works down
/// from `media_objects` and so can only see objects the database still
/// records; an upload that was PUT through a presigned URL and never attached
/// by `from-uploads` has no row at all, and is invisible to it. Those are the
/// bytes nothing will ever reclaim.
///
/// This is the most dangerous job in the system, so it is the most hedged:
///
/// - **Age is not optional.** `presign` writes no row — the object exists in
///   the bucket, unrecorded, for as long as the transfer plus however long
///   the user takes to finish the wizard. Deleting on "unknown to the
///   database" alone would delete uploads out from under people mid-transfer.
///   The presigned URL alone is good for 6 hours and an upload may be 5 GB.
/// - **An empty key set aborts.** If the database read returned nothing, the
///   whole bucket looks unknown. That is a failed query, not an empty library.
/// - **`dry_run` is the default.** It reports what it would delete and
///   changes nothing, so the first run on any environment is an inventory.
async fn execute_storage_reconcile(
    pool: &PgPool,
    storage: &ObjectStore,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let payload = job.payload.as_ref();
    let older_than_days = payload
        .and_then(|p| p.get("older_than_days"))
        .and_then(|v| v.as_i64())
        .unwrap_or(7);
    let limit = payload
        .and_then(|p| p.get("limit"))
        .and_then(|v| v.as_i64())
        .unwrap_or(500) as usize;
    // Opt in to deleting. Anything else, including a missing field, only reports.
    let dry_run = payload
        .and_then(|p| p.get("dry_run"))
        .and_then(|v| v.as_bool())
        .unwrap_or(true);

    if !crate::db::media::any_media_object(pool).await? {
        anyhow::bail!("media_objects is empty; refusing to reconcile the whole bucket against nothing");
    }

    let cutoff = chrono::Utc::now().timestamp() - older_than_days * 86_400;

    let mut listed = 0usize;
    let mut unknown = 0usize;
    let mut too_recent = 0usize;
    let mut deleted = 0usize;
    let mut failed = 0usize;
    // A page of the listing at a time, checked against the database in one
    // query: a 600,000-track library held both lists whole (~300 MB).
    let mut token = None;
    loop {
        let (page, next) = storage.list_page(token).await?;
        listed += page.len();
        let page_keys: Vec<String> = page.iter().map(|(k, _)| k.clone()).collect();
        let known = crate::db::media::known_object_keys(pool, &page_keys).await?;
        for (key, modified) in &page {
            if known.contains(key) {
                continue;
            }
            unknown += 1;
            if *modified > cutoff {
                // Very likely an upload still in progress.
                too_recent += 1;
                continue;
            }
            if deleted + failed >= limit {
                continue;
            }
            if dry_run {
                info!(%key, "storage reconcile (dry run): would delete");
                deleted += 1;
                continue;
            }
            match storage.delete(key).await {
                Ok(()) => deleted += 1,
                Err(e) => {
                    warn!(%key, error = %e, "storage reconcile: delete failed");
                    failed += 1;
                }
            }
        }

        match next {
            Some(t) => token = Some(t),
            None => break,
        }
    }

    info!(
        bucket_objects = listed,
        known = listed - unknown,
        unknown,
        too_recent,
        deleted,
        failed,
        dry_run,
        older_than_days,
        "storage reconcile finished"
    );
    Ok(())
}

/// Refresh the daily listening rollup. Re-runs are safe and intentional:
/// a phone that was offline for a week flushes old sessions when it comes
/// back, so recent days must be recomputed rather than appended to.
async fn execute_stats_rollup(
    pool: &PgPool,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let days = job
        .payload
        .as_ref()
        .and_then(|p| p.get("days"))
        .and_then(|v| v.as_i64())
        .unwrap_or(30) as i32;

    let rows = crate::db::stats::rebuild_daily_rollup(pool, days).await?;
    info!(days, rows, "listening rollup rebuilt");

    // Preference weights ride on the same schedule: both read the session log,
    // and nothing downstream wants one fresh and the other stale.
    let affinity = crate::db::music::rebuild_affinity(pool, days).await?;
    info!(days, rows = affinity, "music affinity rebuilt");

    crate::db::music::refresh_energy(pool).await?;

    Ok(())
}

/// Deletes everything whose 30 days in the trash are up, a page at a time,
/// and prunes old deletion tombstones. Tombstones only serve clients catching
/// up; they are kept well past the trash period (`TOMBSTONE_RETENTION_DAYS`)
/// so a device that was offline for a while still learns what went.
async fn execute_trash_purge(pool: &PgPool, storage: &ObjectStore) -> anyhow::Result<()> {
    let mut purged = 0usize;
    loop {
        let expired = crate::db::trash::list_expired(pool, 200).await?;
        if expired.is_empty() {
            break;
        }
        let mut progressed = false;
        for item in &expired {
            match crate::trash::purge(pool, storage, item.kind(), item.id).await {
                Ok(true) => {
                    purged += 1;
                    progressed = true;
                }
                Ok(false) => {}
                Err(e) => warn!(kind = item.kind().as_str(), id = %item.id, error = %e, "trash purge: could not purge"),
            }
        }
        // A page where nothing could be purged would come back unchanged.
        if !progressed {
            break;
        }
    }

    let pruned = crate::db::sync::prune_tombstones(pool, crate::db::trash::TOMBSTONE_RETENTION_DAYS as i32).await?;
    // The sync feed's change log, kept as long as tombstones: a cursor older
    // than that gets a fresh snapshot instead (filesync::tree).
    let pruned_changes = sqlx::query(
        "DELETE FROM sync_changes WHERE changed_at < now() - make_interval(days => $1::int)",
    )
    .bind(crate::filesync::tree::LOG_RETENTION_DAYS as i32)
    .execute(pool)
    .await?
    .rows_affected();
    info!(purged, pruned_tombstones = pruned, pruned_changes, "trash purge finished");
    Ok(())
}

/// Store one episode of an auto-store show (docs/file-sync-plan.md §5.6),
/// under the show owner's family like a download they asked for.
async fn execute_episode_download(pool: &PgPool, storage: &ObjectStore, job: &crate::jobs::models::Job) -> anyhow::Result<()> {
    let episode_id: uuid::Uuid = job
        .payload
        .as_ref()
        .and_then(|p| p.get("episode_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing episode_id in payload"))?
        .parse()?;
    let Some(episode) = db::podcasts::find_episode(pool, episode_id).await? else {
        return Ok(());
    };
    // Stored meanwhile, or its copy already deleted: nothing to do.
    let trashed: bool = sqlx::query_scalar("SELECT trashed_at IS NOT NULL FROM podcast_episodes WHERE id = $1")
        .bind(episode_id)
        .fetch_one(pool)
        .await?;
    if episode.audio_object_id.is_some() || trashed {
        return Ok(());
    }
    let Some(feed) = sqlx::query_as::<_, crate::podcasts::models::PodcastFeed>(&format!(
        "SELECT {} FROM podcast_feeds WHERE id = $1",
        db::podcasts::FEED_COLS
    ))
    .bind(episode.feed_id)
    .fetch_optional(pool)
    .await?
    else {
        return Ok(());
    };
    let Some(membership) = db::families::find_membership(pool, feed.user_id).await? else {
        return Ok(());
    };
    crate::podcasts::fetch_and_store_episode_audio(pool, storage, membership.family_id, &feed, &episode).await
}

/// Backfill Podcast Index metadata onto subscribed feeds.
///
/// Catches the two populations `subscribe` cannot: feeds subscribed before
/// migration 0060, and feeds whose catalogue entry has since changed. Also the
/// only thing that ever enriches a re-subscribe, which returns early by design
/// rather than doing network work on a no-op.
///
/// Every per-feed failure is logged and skipped rather than failing the job. A
/// single unreachable feed must not strand the other ninety-nine, and the next
/// pass retries it anyway.
async fn execute_podcast_catalog_sync(
    pool: &PgPool,
    config: &AppConfig,
    _job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let Some(metadata) = config.metadata.as_ref() else {
        debug!("no metadata service configured — skipping podcast catalog sync");
        return Ok(());
    };

    let feeds = crate::db::podcasts::feeds_needing_catalog_sync(
        pool,
        CATALOG_STALE_AFTER_DAYS,
        CATALOG_SYNC_BATCH_SIZE,
    )
    .await?;

    if feeds.is_empty() {
        return Ok(());
    }

    let mirror = crate::metadata::mirror::MetadataMirror::new(metadata);
    let (mut enriched, mut unknown) = (0u32, 0u32);

    for (feed_id, feed_url) in &feeds {
        match mirror.podcast_lookup(Some(feed_url), None, None).await {
            Ok(Some(entry)) => {
                let language_base = entry.language_base.clone();
                if let Err(e) = crate::db::podcasts::set_catalog_metadata(
                    pool,
                    *feed_id,
                    Some(entry.id),
                    entry.podcast_guid.as_deref(),
                    entry.itunes_id,
                    &entry.categories,
                    Some(entry.popularity_score),
                    language_base.as_deref(),
                )
                .await
                {
                    warn!(%feed_id, "storing catalog metadata failed: {e}");
                    continue;
                }
                enriched += 1;
            }
            Ok(None) => {
                // Stamp the check anyway. Without it a feed the catalogue does
                // not have is re-asked on every single pass, forever.
                if let Err(e) = crate::db::podcasts::set_catalog_metadata(
                    pool, *feed_id, None, None, None, &[], None, None,
                )
                .await
                {
                    warn!(%feed_id, "stamping catalog check failed: {e}");
                    continue;
                }
                unknown += 1;
            }
            Err(e) => {
                warn!(%feed_id, %feed_url, "catalog lookup failed: {e}");
            }
        }
    }

    debug!(enriched, unknown, batch = feeds.len(), "podcast catalog sync complete");

    // Detection only — see `duplicate_feeds_by_guid`. Merging touches
    // episodes, progress and downloaded files and is not a background sweep's
    // decision to make.
    match crate::db::podcasts::duplicate_feeds_by_guid(pool).await {
        Ok(dupes) if !dupes.is_empty() => {
            let total: i64 = dupes.iter().map(|(_, n)| n - 1).sum();
            warn!(
                shows = dupes.len(),
                redundant_feeds = total,
                "the same podcast is subscribed more than once in a household — \
                 not merged, see docs/podcast-recommendations-plan.md P4"
            );
        }
        Ok(_) => {}
        Err(e) => warn!("duplicate feed check failed: {e}"),
    }

    Ok(())
}

async fn execute_feed_refresh(
    pool: &PgPool,
    storage: &ObjectStore,
    job: &crate::jobs::models::Job,
) -> anyhow::Result<()> {
    let feed_id_str = job
        .payload
        .as_ref()
        .and_then(|p| p.get("feed_id"))
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("missing feed_id in payload"))?;

    let feed_id: uuid::Uuid = feed_id_str.parse()?;

    // Refresh the feed from its source (RSS or YouTube)
    podcasts::refresh_feed_job(pool, storage, feed_id).await?;

    debug!(%feed_id, "feed refresh complete");
    Ok(())
}
