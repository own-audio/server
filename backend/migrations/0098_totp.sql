-- Two-factor sign-in with an authenticator app (security hardening plan
-- §5.1): a TOTP secret per user, kept encrypted (`auth::at_rest`), pending
-- until the first code proves the app has it; recovery codes as hashes; and
-- the short-lived challenge a sign-in waits in between the password and the
-- code.
CREATE TABLE user_totp (
    user_id        UUID        PRIMARY KEY REFERENCES users(id) ON DELETE CASCADE,
    secret         TEXT        NOT NULL,
    enabled_at     TIMESTAMPTZ,
    -- The last 30-second step a code was accepted for: a code works once.
    last_used_step BIGINT      NOT NULL DEFAULT 0,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE user_recovery_codes (
    user_id   UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    code_hash TEXT        NOT NULL,
    used_at   TIMESTAMPTZ,
    PRIMARY KEY (user_id, code_hash)
);

CREATE TABLE mfa_challenges (
    token_hash  TEXT        PRIMARY KEY,
    user_id     UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    device_name TEXT,
    device_kind TEXT,
    expires_at  TIMESTAMPTZ NOT NULL,
    used_at     TIMESTAMPTZ,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);
