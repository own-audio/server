-- Migration 0009: store the raw RSS image URL on podcast feeds
ALTER TABLE podcast_feeds ADD COLUMN IF NOT EXISTS image_url TEXT;
