-- Migration 0011: YouTube channel feed support
-- podcast_feeds now supports two source types: 'rss' and 'youtube'.
-- source_type  — distinguishes how the feed is fetched and how episodes are downloaded.
-- youtube_channel_id — YouTube Data API channel ID (UC…), used for refresh.

ALTER TABLE podcast_feeds
    ADD COLUMN IF NOT EXISTS source_type        TEXT NOT NULL DEFAULT 'rss',
    ADD COLUMN IF NOT EXISTS youtube_channel_id TEXT;
