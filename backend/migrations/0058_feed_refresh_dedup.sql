-- Migration 0058: prevent duplicate in-flight feed_refresh jobs
-- ─────────────────────────────────────────────────
-- enqueue_due_feed_refreshes() re-checks every 30s for feeds whose
-- last_refreshed_at hasn't advanced, but last_refreshed_at only updates
-- once a refresh *succeeds* — so a single slow/stuck refresh got a fresh
-- duplicate job enqueued every tick until it finally unblocked, then all
-- of them ran at once. This index makes a second pending/running
-- feed_refresh job for the same feed_id impossible at the DB level,
-- regardless of which worker container is polling.

-- Any database that ran the old enqueue already holds the duplicates this
-- index forbids, and CREATE UNIQUE INDEX refuses to build over them — the
-- local assembler crash-looped on exactly that on 2026-08-27. Keep the
-- newest in-flight row per feed and fail the rest first; they were never
-- going to finish anyway (a row stuck in 'running' belongs to a worker
-- that restarted without it).
UPDATE jobs
SET status = 'failed',
    error = 'superseded duplicate feed_refresh job (migration 0058)',
    completed_at = now()
WHERE id IN (
    SELECT id FROM (
        SELECT id,
               row_number() OVER (
                   PARTITION BY payload ->> 'feed_id'
                   ORDER BY created_at DESC, id DESC
               ) AS rn
        FROM jobs
        WHERE job_type = 'feed_refresh' AND status IN ('pending', 'running')
    ) ranked
    WHERE rn > 1
);

CREATE UNIQUE INDEX jobs_feed_refresh_inflight_idx
    ON jobs (job_type, (payload ->> 'feed_id'))
    WHERE job_type = 'feed_refresh' AND status IN ('pending', 'running');
