-- `Idempotency-Key` on /api/v2 writes (docs/api-v2-plan.md, V9; DESIGN §10): a
-- retried request answers what the first one did, kept 24 hours.
CREATE TABLE idempotency_keys (
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    key          TEXT        NOT NULL,
    -- Method, path and body: the same key with another request is refused.
    request_hash TEXT        NOT NULL,
    -- NULL while the first request is still being answered.
    status       SMALLINT,
    content_type TEXT,
    body         BYTEA,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, key)
);
CREATE INDEX idempotency_keys_created_idx ON idempotency_keys (created_at);
