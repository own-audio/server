-- The orders /api/v2 pages books, podcasts and episodes in (docs/api-v2-plan.md, V3b),
-- the id the tie-breaker, as 0106 does for music.
CREATE INDEX audiobook_books_title_idx ON audiobook_books_all (lower(title), id);
CREATE INDEX audiobook_books_created_idx ON audiobook_books_all (created_at DESC, id DESC);
CREATE INDEX podcast_feeds_title_idx ON podcast_feeds (lower(title), id);
CREATE INDEX podcast_episodes_feed_order_idx ON podcast_episodes (feed_id, COALESCE(published_at, created_at) DESC, id DESC);
