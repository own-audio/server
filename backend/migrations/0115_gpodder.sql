-- Migration 0115: the gPodder sync API (docs/gpodder.md).
--
-- Podcast apps that speak gpodder.net's API v2 (AntennaPod, Kasts, gPodder,
-- own.audio Podcasts) keep their subscriptions and listening positions here.
-- The API names everything by URL and asks "what changed since <timestamp>",
-- so it gets logs of its own:
--
--   * gpodder_subscription_changes: every RSS feed a listener starts or stops
--     owning, whichever way it happened (this API, the console, the apps,
--     Subsonic), written by triggers so no path can forget it.
--   * gpodder_episode_actions: what the apps report (play positions and the
--     rest) and every position this server records for a listener in an
--     episode, so a position set in an own.audio app reaches a gPodder app.
--
-- Timestamps the API hands out are unix seconds of `changed_at` /
-- `received_at`. Readers ask from a few seconds before the `since` they are
-- given, so a transaction that commits late is still seen; the API's actions
-- are idempotent, so the overlap costs nothing.

CREATE TABLE gpodder_devices (
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    -- What the app calls itself; letters, digits, '.', '-' and '_'.
    device_id   TEXT        NOT NULL CHECK (device_id ~ '^[A-Za-z0-9._-]{1,64}$'),
    caption     TEXT        NOT NULL DEFAULT '',
    -- desktop, laptop, mobile, server or other.
    kind        TEXT        NOT NULL DEFAULT 'other',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, device_id)
);

CREATE TABLE gpodder_subscription_changes (
    id          BIGSERIAL   PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    feed_url    TEXT        NOT NULL,
    added       BOOLEAN     NOT NULL,
    -- The app that made the change through the API, which is not told about
    -- it again; NULL for a change made anywhere else.
    device_id   TEXT,
    changed_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX gpodder_subscription_changes_user_idx ON gpodder_subscription_changes (user_id, changed_at);

CREATE TABLE gpodder_episode_actions (
    id          BIGSERIAL   PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    podcast_url TEXT        NOT NULL,
    episode_url TEXT        NOT NULL,
    guid        TEXT,
    action      TEXT        NOT NULL CHECK (action IN ('download', 'play', 'delete', 'new')),
    -- NULL for a position this server recorded itself.
    device_id   TEXT,
    -- When it happened, as the app says.
    happened_at TIMESTAMPTZ NOT NULL,
    started     INTEGER,
    position    INTEGER,
    total       INTEGER,
    -- When this server learned of it: what `since` compares with.
    received_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX gpodder_episode_actions_user_idx ON gpodder_episode_actions (user_id, received_at);

-- ── Subscriptions: a feed the listener owns is a subscription ──────────────

-- `own_audio.gpodder_device` names the app while the API applies its changes.
CREATE FUNCTION gpodder_log_subscription() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    device TEXT := NULLIF(current_setting('own_audio.gpodder_device', true), '');
BEGIN
    IF TG_OP IN ('DELETE', 'UPDATE') AND OLD.source_type = 'rss' THEN
        IF TG_OP = 'DELETE' OR OLD.feed_url IS DISTINCT FROM NEW.feed_url OR OLD.user_id IS DISTINCT FROM NEW.user_id THEN
            INSERT INTO gpodder_subscription_changes (user_id, feed_url, added, device_id) VALUES (OLD.user_id, OLD.feed_url, false, device);
        END IF;
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') AND NEW.source_type = 'rss' THEN
        IF TG_OP = 'INSERT' OR OLD.feed_url IS DISTINCT FROM NEW.feed_url OR OLD.user_id IS DISTINCT FROM NEW.user_id THEN
            INSERT INTO gpodder_subscription_changes (user_id, feed_url, added, device_id) VALUES (NEW.user_id, NEW.feed_url, true, device);
        END IF;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER podcast_feeds_gpodder AFTER INSERT OR UPDATE OF feed_url, user_id OR DELETE ON podcast_feeds
    FOR EACH ROW EXECUTE FUNCTION gpodder_log_subscription();

-- The feeds that exist already are where every listener's log starts.
INSERT INTO gpodder_subscription_changes (user_id, feed_url, added, changed_at)
SELECT user_id, feed_url, true, created_at FROM podcast_feeds WHERE source_type = 'rss';

-- ── Positions recorded here become play actions ────────────────────────────

-- Skipped while the gPodder API itself applies an app's action
-- (`own_audio.gpodder_apply`): that action is logged as the app sent it.
CREATE FUNCTION gpodder_log_progress() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF current_setting('own_audio.gpodder_apply', true) IS NOT DISTINCT FROM 'on' THEN
        RETURN NULL;
    END IF;
    INSERT INTO gpodder_episode_actions (user_id, podcast_url, episode_url, guid, action, happened_at, started, position, total)
    SELECT NEW.user_id, f.feed_url, e.audio_url, e.guid, 'play', NEW.updated_at, 0,
           CASE WHEN NEW.completed AND e.duration_secs IS NOT NULL THEN e.duration_secs
                ELSE round(NEW.position_secs)::int END,
           COALESCE(e.duration_secs, round(NEW.position_secs)::int)
      FROM podcast_episodes e JOIN podcast_feeds f ON f.id = e.feed_id
     WHERE e.id = NEW.episode_id AND e.audio_url IS NOT NULL AND f.source_type = 'rss';
    RETURN NULL;
END $$;

CREATE TRIGGER podcast_progress_gpodder AFTER INSERT OR UPDATE OF position_secs, completed ON podcast_progress
    FOR EACH ROW EXECUTE FUNCTION gpodder_log_progress();
