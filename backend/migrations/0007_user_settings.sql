-- Migration 0007: user settings
-- ─────────────────────────────────────────────────
-- One row per user; created on first access (upsert pattern).

CREATE TABLE user_settings (
    user_id             UUID             PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- Playback speed multiplier (0.5 – 3.0, default 1.0).
    playback_speed      DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    -- Seconds to skip forward at episode start (skip intro).
    skip_intro_secs     INTEGER          NOT NULL DEFAULT 0,
    -- Seconds to cut from episode end (skip outro).
    skip_outro_secs     INTEGER          NOT NULL DEFAULT 0,
    updated_at          TIMESTAMPTZ      NOT NULL DEFAULT now()
);
