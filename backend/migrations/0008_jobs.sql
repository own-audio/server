-- Migration 0008: background jobs
-- ─────────────────────────────────────────────────
-- Simple job queue for feed polling, metadata fetching, thumbnail processing,
-- and import tasks. Workers poll for pending rows ordered by scheduled_at.

CREATE TABLE jobs (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- 'feed_refresh' | 'book_import' | 'thumbnail_fetch' | 'episode_download' | 'cleanup'
    job_type      TEXT        NOT NULL,
    -- 'pending' | 'running' | 'completed' | 'failed'
    status        TEXT        NOT NULL DEFAULT 'pending'
                              CHECK (status IN ('pending', 'running', 'completed', 'failed')),
    -- Arbitrary structured input for the job handler.
    payload       JSONB,
    -- Structured output from the job handler on completion.
    result        JSONB,
    error         TEXT,
    attempts      INTEGER     NOT NULL DEFAULT 0,
    max_attempts  INTEGER     NOT NULL DEFAULT 3,
    scheduled_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at    TIMESTAMPTZ,
    completed_at  TIMESTAMPTZ,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Workers query: WHERE status = 'pending' AND scheduled_at <= now() ORDER BY scheduled_at
CREATE INDEX jobs_pending_idx ON jobs (status, scheduled_at)
    WHERE status = 'pending';
