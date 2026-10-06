-- Migration 0010: store raw RSS image URL on podcast episodes
ALTER TABLE podcast_episodes ADD COLUMN IF NOT EXISTS image_url TEXT;
