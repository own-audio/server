-- Migration 0021: play-queue sync, delete tombstones, push registration
-- ─────────────────────────────────────────────────────────────────────────
-- Everything here exists to make phone clients practical, and is written to
-- serve iOS and Android equally.

-- ── Play queue ───────────────────────────────────────────────────────────────
-- One queue per user, so "what I'm playing next" follows them between phone,
-- tablet, and the web player. Stored as an ordered JSONB array rather than a
-- child table: a queue is always read and written whole, never queried into.
CREATE TABLE play_queues (
    user_id          UUID        PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    -- [{ "media_kind": "...", "item_id": "...", "part_id": "..." }, ...]
    items            JSONB       NOT NULL DEFAULT '[]'::jsonb,
    current_index    INTEGER     NOT NULL DEFAULT 0,
    position_secs    DOUBLE PRECISION NOT NULL DEFAULT 0,
    -- Which device wrote last, so a client can tell "this came from elsewhere".
    updated_by_device TEXT,
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- ── Delete tombstones ────────────────────────────────────────────────────────
-- A delta sync (`?since=`) can report changed rows, but a row that was deleted
-- is simply absent — indistinguishable from one the client already has. These
-- tombstones let a client purge its local cache instead of keeping ghosts.
--
-- `family_id` is captured at deletion time so members who could see a shared
-- item learn it is gone, not just its owner.
CREATE TABLE deleted_items (
    id         BIGSERIAL   PRIMARY KEY,
    media_kind TEXT        NOT NULL
                           CHECK (media_kind IN ('audiobook', 'podcast', 'music', 'playlist')),
    item_id    UUID        NOT NULL,
    user_id    UUID        NOT NULL,
    family_id  UUID,
    deleted_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX deleted_items_user_idx   ON deleted_items (user_id, deleted_at DESC);
CREATE INDEX deleted_items_family_idx ON deleted_items (family_id, deleted_at DESC)
    WHERE family_id IS NOT NULL;

-- ── Push registration ────────────────────────────────────────────────────────
-- Both transports are first-class: APNs for iOS, FCM for Android. A token is
-- globally unique — if a device is handed to another user, re-registering
-- moves it rather than duplicating it.
CREATE TABLE device_push_tokens (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    platform     TEXT        NOT NULL CHECK (platform IN ('apns', 'fcm')),
    token        TEXT        NOT NULL UNIQUE,
    device_name  TEXT,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_seen_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX device_push_tokens_user_idx ON device_push_tokens (user_id);

-- ── Pending notifications ────────────────────────────────────────────────────
-- Notifications are queued here rather than pushed inline, so the trigger
-- (a feed refresh finding new episodes) never blocks on a push provider, and
-- an undeliverable message can be retried or inspected.
CREATE TABLE pending_notifications (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind       TEXT        NOT NULL,
    title      TEXT        NOT NULL,
    body       TEXT,
    -- Deep-link payload, e.g. {"media_kind":"podcast","item_id":"..."}.
    data       JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    delivered_at TIMESTAMPTZ,
    error      TEXT
);

CREATE INDEX pending_notifications_undelivered_idx
    ON pending_notifications (created_at)
    WHERE delivered_at IS NULL;
