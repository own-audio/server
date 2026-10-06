-- Migration 0013: audiobook player settings
-- ──────────────────────────────────────────
-- User-level defaults for audiobook playback.
-- Per-audiobook overrides (null = use user default).

-- ── User audiobook defaults ──────────────────────────────────────────────────
ALTER TABLE user_settings
  ADD COLUMN ab_skip_forward_secs  INTEGER NOT NULL DEFAULT 30,
  ADD COLUMN ab_skip_backward_secs INTEGER NOT NULL DEFAULT 15,
  ADD COLUMN ab_playback_speed     DOUBLE PRECISION NOT NULL DEFAULT 1.0;

-- ── Per-audiobook overrides ──────────────────────────────────────────────────
CREATE TABLE audiobook_settings (
    user_id            UUID             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    book_id            UUID             NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    skip_forward_secs  INTEGER,
    skip_backward_secs INTEGER,
    playback_speed     DOUBLE PRECISION,
    updated_at         TIMESTAMPTZ      NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, book_id)
);
