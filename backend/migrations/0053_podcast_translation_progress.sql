-- Migration 0053: playback progress for personal-use podcast episode translations, so a
-- translation can appear in GET /library/continue like any other in-progress item.
-- duration_secs is captured once at assembly time (ffprobe on the final concatenated file);
-- progress_secs/completed/progress_updated_at mirror podcast_progress's shape closely enough
-- for the continue-listening query to treat them the same way, without sharing that table
-- (a translation id is not an episode id, and must never collide with the original episode's
-- own progress row).
ALTER TABLE podcast_episode_translations ADD COLUMN IF NOT EXISTS duration_secs INTEGER;
ALTER TABLE podcast_episode_translations ADD COLUMN IF NOT EXISTS progress_secs DOUBLE PRECISION NOT NULL DEFAULT 0;
ALTER TABLE podcast_episode_translations ADD COLUMN IF NOT EXISTS completed BOOLEAN NOT NULL DEFAULT false;
ALTER TABLE podcast_episode_translations ADD COLUMN IF NOT EXISTS progress_updated_at TIMESTAMPTZ;
