-- Migration 0020: listening history + statistics
-- ───────────────────────────────────────────────
-- Progress rows only ever hold the LATEST position per item, which cannot
-- answer "how much did I listen last week". This adds an append-only session
-- log plus a daily rollup for cheap range queries.
--
-- Client-agnostic by design (iOS, Android, web, Subsonic):
--   • `client_session_id` makes reporting idempotent. Android's WorkManager
--     and iOS background tasks both retry aggressively; a retried batch must
--     not double-count, so the client generates the id and we dedupe on it.
--   • Sessions are reported in batches, so a device that was offline or in
--     Doze can flush everything it accumulated in one request.
--   • Timestamps are stored in UTC; day bucketing happens at query time
--     against the caller's timezone offset.

CREATE TABLE listening_sessions (
    id                UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id           UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    media_kind        TEXT        NOT NULL
                                  CHECK (media_kind IN ('audiobook', 'podcast', 'music')),
    -- The top-level item: book, feed, or track.
    item_id           UUID        NOT NULL,
    -- Optional finer grain: audiobook file or podcast episode.
    part_id           UUID,
    started_at        TIMESTAMPTZ NOT NULL,
    ended_at          TIMESTAMPTZ NOT NULL,
    -- Wall-clock audio consumed, which is not ended_at - started_at when the
    -- listener paused or played at a non-1.0 speed.
    seconds_listened  INTEGER     NOT NULL CHECK (seconds_listened >= 0),
    playback_speed    DOUBLE PRECISION,
    device_kind       TEXT        NOT NULL DEFAULT 'other'
                                  CHECK (device_kind IN ('web', 'ios', 'android', 'subsonic', 'other')),
    -- 'reported' = the client sent an explicit session; 'derived' = inferred
    -- from a progress update (see db::stats::derive_from_progress).
    source            TEXT        NOT NULL DEFAULT 'reported'
                                  CHECK (source IN ('reported', 'derived')),
    -- Client-generated idempotency key; NULL for derived rows. Postgres
    -- treats NULLs as distinct, so many derived rows coexist happily.
    client_session_id TEXT,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT listening_sessions_client_uq UNIQUE (user_id, client_session_id),
    CONSTRAINT listening_sessions_span_ck CHECK (ended_at >= started_at)
);

CREATE INDEX listening_sessions_user_time_idx ON listening_sessions (user_id, started_at DESC);
CREATE INDEX listening_sessions_item_idx      ON listening_sessions (media_kind, item_id);
CREATE INDEX listening_sessions_source_idx    ON listening_sessions (user_id, source, started_at DESC);

-- ── Daily rollup ─────────────────────────────────────────────────────────────
-- Keeps "last 365 days" queries cheap. Rebuilt incrementally by the
-- `stats_rollup` job; the stats endpoints read sessions directly for short
-- ranges and the rollup for long ones.
CREATE TABLE listening_daily (
    user_id          UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    day              DATE        NOT NULL,
    media_kind       TEXT        NOT NULL
                                 CHECK (media_kind IN ('audiobook', 'podcast', 'music')),
    seconds_listened BIGINT      NOT NULL DEFAULT 0,
    session_count    INTEGER     NOT NULL DEFAULT 0,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, day, media_kind)
);

CREATE INDEX listening_daily_user_day_idx ON listening_daily (user_id, day DESC);

-- ── Stats privacy (decision D2) ──────────────────────────────────────────────
-- Listening history is personal. Family admins do NOT see a member's stats by
-- default; the member opts in, or an admin may enable it for a *restricted*
-- account (one carrying a deny_all policy — in practice a child's), which is
-- the parental-supervision case the product exists for.
ALTER TABLE family_members
    ADD COLUMN stats_visibility TEXT NOT NULL DEFAULT 'private'
        CHECK (stats_visibility IN ('private', 'family_admin'));
