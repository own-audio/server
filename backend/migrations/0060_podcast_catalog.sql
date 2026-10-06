-- Migration 0060: Phase 2 of docs/podcast-recommendations-plan.md — carry the
-- Podcast Index catalogue's answer for each subscribed feed.
--
-- `podcast_feeds` has stored no category since it was created (migration
-- 0004): `channel.itunes_ext()` is read at subscribe time for the author and
-- the image, and the categories on the same struct are discarded. That is why
-- there is no "more like this" over a household's own subscriptions today —
-- not because it is hard, but because the one field it needs was never kept.
--
-- Everything here is nullable and best-effort. A feed subscribed while the
-- metadata service is unreachable is a normal feed with no categories, not a
-- failed subscribe.

ALTER TABLE podcast_feeds
    -- The catalogue's own id, so a refresh can re-ask about this exact feed
    -- without repeating the URL normalization.
    ADD COLUMN catalog_id          BIGINT,
    -- Stable cross-host identity, when the publisher declares one. Note what
    -- this does NOT do: Podcast Index synthesizes a UUIDv5 *from the feed URL*
    -- when a feed declares no <podcast:guid>, so two different URLs for the
    -- same show usually carry two different GUIDs (verified 2026-08-29 against
    -- This American Life's two catalogue entries). It identifies one show
    -- across hosting moves and across two people subscribing to the same URL;
    -- it does not recognize an aggregator's copy as the original.
    ADD COLUMN podcast_guid        TEXT,
    ADD COLUMN itunes_id           BIGINT,
    ADD COLUMN categories          TEXT[] NOT NULL DEFAULT '{}',
    ADD COLUMN popularity_score    INTEGER,
    -- The language tag folded to its subtag. `language` above it stays raw,
    -- straight from the feed: the wild carries 112 spellings of English
    -- (`en`, `en-us`, `en-US`, …), so a filter on the raw column matches a
    -- fraction of what it should. Every language filter reads this one.
    ADD COLUMN language_base       TEXT,
    -- NULL = never asked. Distinct from "asked, and the catalogue does not
    -- have it", which is a row with this set and `catalog_id` still NULL —
    -- the difference between work not done and work done with no result, and
    -- the thing the backfill job in P4 needs to avoid re-asking forever.
    ADD COLUMN catalog_checked_at  TIMESTAMPTZ;

CREATE INDEX podcast_feeds_categories_idx ON podcast_feeds USING gin (categories);

CREATE INDEX podcast_feeds_guid_idx ON podcast_feeds (podcast_guid)
    WHERE podcast_guid IS NOT NULL;

CREATE INDEX podcast_feeds_language_idx ON podcast_feeds (language_base)
    WHERE language_base IS NOT NULL;
