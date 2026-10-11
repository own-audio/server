-- Personal access tokens (docs/api-v2-plan.md, V2; DESIGN §5.1): for scripts.
-- Named, scoped, shown once, stored as a SHA-256 hash, revocable, and good on
-- /api/v2 only.
CREATE TABLE personal_tokens (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name         TEXT        NOT NULL,
    token_hash   TEXT        NOT NULL UNIQUE,
    scopes       TEXT[]      NOT NULL,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    last_used_at TIMESTAMPTZ,
    expires_at   TIMESTAMPTZ,
    revoked_at   TIMESTAMPTZ
);
CREATE INDEX personal_tokens_user_idx ON personal_tokens (user_id);
