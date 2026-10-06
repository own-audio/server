-- Migration 0070: per-user, per-track preference weight
-- ─────────────────────────────────────────────────────
-- docs/music-signals-and-smart-playlists-plan.md §2.5–2.6.
--
-- `listening_sessions` is an append-only log. Aggregating it every time a queue
-- refills would be wrong, so this is the rollup selection actually reads,
-- rebuilt by the existing `stats_rollup` job.
--
-- The raw counters live here beside the derived `weight` on purpose: the
-- formula is provisional and will be retuned against a real library, and
-- storing its inputs means retuning is an UPDATE rather than a replay of the
-- whole session log.
--
-- Privacy: decision D2 in 0020 makes a member's listening history personal, and
-- family admins do not see it by default. A preference weight is at least as
-- personal as a play count, so it inherits that rule — this table is never
-- exposed through any family-facing endpoint.
--
-- Nothing here is per-family. `music_tracks` is shared (0019) but taste is not:
-- one pool, N profiles.

CREATE TABLE music_track_affinity (
    user_id         UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id        UUID        NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,

    -- Raw, replayable counters.
    play_count      INTEGER     NOT NULL DEFAULT 0,
    -- Skips weighted by position: a skip 5 s in is "wrong pick", a skip three
    -- minutes into a four-minute track is effectively a completion. Counted
    -- separately so the cutoff can move without re-reading the log.
    early_skips     INTEGER     NOT NULL DEFAULT 0,
    late_skips      INTEGER     NOT NULL DEFAULT 0,
    last_played_at  TIMESTAMPTZ,
    last_skipped_at TIMESTAMPTZ,

    -- Derived. Read constantly, recomputed rarely.
    weight          REAL        NOT NULL DEFAULT 1.0
                                CHECK (weight >= 0.0 AND weight <= 1.0),

    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);

-- Selection orders candidates by weight within one user's rows.
CREATE INDEX music_track_affinity_weight_idx
    ON music_track_affinity (user_id, weight DESC);
