-- Migration 0050: capture the Podcasting 2.0 <podcast:transcript> URL, when a
-- feed publishes one, so podcast-translation-plan.md's translation feature
-- can be offered without ever running speech-to-text (see docs/podcast-translation-plan.md).
ALTER TABLE podcast_episodes ADD COLUMN IF NOT EXISTS transcript_url TEXT;
ALTER TABLE podcast_episodes ADD COLUMN IF NOT EXISTS transcript_type TEXT;
