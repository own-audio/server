-- Migration 0004: podcast feeds and episodes
-- ─────────────────────────────────────────────────
-- A podcast_feed is one show in a user's subscription list.
-- Each user's subscriptions are isolated: (user_id, feed_url) is unique.
-- podcast_episodes are scoped to a feed, identified by their RSS guid.

-- ── podcast_feeds ────────────────────────────────────────────────────────────
CREATE TABLE podcast_feeds (
    id                  UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id             UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    feed_url            TEXT        NOT NULL,
    title               TEXT        NOT NULL,
    description         TEXT,
    author              TEXT,
    link                TEXT,
    language            TEXT,
    image_object_id     UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    last_refreshed_at   TIMESTAMPTZ,
    refresh_error       TEXT,
    created_at          TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at          TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT podcast_feeds_user_url_uq UNIQUE (user_id, feed_url)
);

CREATE INDEX podcast_feeds_user_id_idx ON podcast_feeds (user_id);

-- ── podcast_episodes ─────────────────────────────────────────────────────────
CREATE TABLE podcast_episodes (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    feed_id          UUID        NOT NULL REFERENCES podcast_feeds (id) ON DELETE CASCADE,
    guid             TEXT        NOT NULL,
    title            TEXT        NOT NULL,
    description      TEXT,
    published_at     TIMESTAMPTZ,
    duration_secs    INTEGER,
    episode_number   INTEGER,
    season_number    INTEGER,
    -- Original remote audio URL from the feed.
    audio_url        TEXT,
    -- Set when the episode has been downloaded into the object store.
    audio_object_id  UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    image_object_id  UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT podcast_episodes_feed_guid_uq UNIQUE (feed_id, guid)
);

CREATE INDEX podcast_episodes_feed_id_idx       ON podcast_episodes (feed_id);
CREATE INDEX podcast_episodes_published_at_idx  ON podcast_episodes (published_at DESC);
