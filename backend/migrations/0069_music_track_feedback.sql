-- Migration 0069: per-user negative feedback on a track
-- ─────────────────────────────────────────────────────
-- docs/music-signals-and-smart-playlists-plan.md §2.4.
--
-- Two meanings, deliberately not one button:
--
--   dislike  a strong negative preference. Heavily down-weighted in automatic
--            selection, but it recovers over months — a track that did not suit
--            the moment is not a track you never want to hear again.
--   banned   never select this automatically. A hard filter applied before
--            scoring, which does not recover until the user undoes it.
--
-- Collapsing these into one control would make the second meaning impossible to
-- express and the first impossible to recover from.
--
-- NEITHER DELETES ANYTHING. The track stays in the family library, stays
-- visible to its owner, and is untouched for every other member; only this
-- user's automatic selection skips it. That is the same distinction the product
-- draws everywhere else between private and shared — "not for me" is a
-- preference, not a permission. No lock icon, no removal, no side effect
-- anyone else can observe.
--
-- One row per (user, track): the two kinds are exclusive, and moving between
-- them is an upsert rather than a second row.

CREATE TABLE music_track_feedback (
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id   UUID        NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    kind       TEXT        NOT NULL CHECK (kind IN ('dislike', 'banned')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);

-- Selection subtracts banned tracks before it scores anything, so that filter
-- is the hot path, not the lookup of one track's feedback.
CREATE INDEX music_track_feedback_banned_idx
    ON music_track_feedback (user_id, track_id)
    WHERE kind = 'banned';
