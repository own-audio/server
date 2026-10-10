-- A presigned upload that was never completed leaves an object in storage with
-- no media_objects row, so nothing billed it and nothing swept it (security
-- hardening plan C1). Every presign now records an intent; `complete` removes
-- it; the daily storage sweep deletes the object and the row for intents older
-- than a day.
CREATE TABLE upload_intents (
    object_key    TEXT        PRIMARY KEY,
    family_id     UUID        NOT NULL,
    user_id       UUID        NOT NULL,
    declared_size BIGINT,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX upload_intents_created_idx ON upload_intents (created_at);
