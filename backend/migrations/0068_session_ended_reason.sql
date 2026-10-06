-- Migration 0068: why a listening session ended
-- ─────────────────────────────────────────────
-- docs/music-signals-and-smart-playlists-plan.md §2.3 — the primitive the whole
-- preference model is built on.
--
-- `listening_sessions` records how long was listened, never why it stopped. A
-- twelve-second session might be a skip, a phone call, a killed process, or
-- someone sampling a track on purpose, and only the client can tell those
-- apart. Without this column a skip is indistinguishable from an interruption,
-- so no amount of later analysis can recover the signal.
--
-- Nullable on purpose. Every existing row, every client that has not shipped
-- the field yet, and every row derived from a progress update (which is
-- inference, not observation — see db::stats::derive_from_progress) keeps
-- working and simply contributes no preference signal.
--
--   completed  played to the end, or within the last few seconds
--   skipped    the user explicitly advanced to something else
--   stopped    ended without a skip: paused and abandoned, quit, interrupted
--   replaced   navigated away to something unrelated, not a next-track skip
--
-- Clients send the fact and the position; they do NOT send a judgement. How
-- much an early skip is worth is derived server-side from
-- seconds_listened / duration, so that curve can be retuned without shipping an
-- app update to six platforms.
--
-- Unlike device_kind (0065), an unrecognised value is NOT coerced to a
-- catch-all. A wrong bucket would teach the model something false; NULL only
-- costs a little signal. playback::normalize_ended_reason maps anything
-- unknown to NULL rather than risking the constraint.

ALTER TABLE listening_sessions
    ADD COLUMN ended_reason TEXT
        CONSTRAINT listening_sessions_ended_reason_check
        CHECK (ended_reason IN ('completed', 'skipped', 'stopped', 'replaced'));

-- The affinity rollup (§2.6) only ever reads music sessions that carry a
-- reason. Partial, because most rows will never have one.
CREATE INDEX listening_sessions_reason_idx
    ON listening_sessions (user_id, item_id, ended_reason)
    WHERE ended_reason IS NOT NULL AND media_kind = 'music';
