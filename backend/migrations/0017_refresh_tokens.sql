-- Migration 0017: refresh tokens + device session chains
-- ────────────────────────────────────────────────────────
-- Design rules:
--   • A "chain" is one signed-in device. Tokens rotate on every refresh but
--     keep the same chain_id; presenting an already-rotated token is treated
--     as theft and revokes the whole chain.
--   • Only a SHA-256 hash of the token secret is stored, never the secret.
--   • sessions.chain_id links access-JWT sessions to their device chain so
--     revoking a device kills its access tokens too (middleware checks
--     sessions.revoked_at on every request).

CREATE TABLE refresh_tokens (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    chain_id     UUID        NOT NULL,
    token_hash   TEXT        NOT NULL UNIQUE,
    device_name  TEXT,
    device_kind  TEXT        NOT NULL DEFAULT 'other'
                             CHECK (device_kind IN ('web', 'ios', 'android', 'other')),
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,                          -- NULL = active
    replaced_by  UUID        REFERENCES refresh_tokens (id)
);

CREATE INDEX refresh_tokens_user_idx  ON refresh_tokens (user_id);
CREATE INDEX refresh_tokens_chain_idx ON refresh_tokens (chain_id);

ALTER TABLE sessions ADD COLUMN chain_id UUID;
CREATE INDEX sessions_chain_idx ON sessions (chain_id);
