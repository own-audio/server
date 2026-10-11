-- The last position on each device (DESIGN §7.6; docs/api-v2-plan.md, V4), so
-- an app can offer "continue from the car (40:02)?". The item's own progress
-- row stays the one every device resumes from; this only remembers where each
-- device last was. No foreign key on the item (three kinds share the table):
-- the orphan sweep removes rows whose item is gone.
CREATE TABLE progress_devices (
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    item_kind   TEXT        NOT NULL CHECK (item_kind IN ('track', 'book', 'episode')),
    item_id     UUID        NOT NULL,
    -- The session's refresh chain, or a personal token's id.
    device_id   UUID        NOT NULL,
    device_name TEXT,
    device_kind TEXT        NOT NULL DEFAULT 'other',
    position_ms BIGINT      NOT NULL CHECK (position_ms >= 0),
    finished    BOOLEAN     NOT NULL DEFAULT false,
    at          TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (user_id, item_kind, item_id, device_id)
);
