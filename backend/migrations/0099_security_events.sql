-- The audit trail of security events (security hardening plan §9): who
-- signed in or failed to, changed a password, turned two-factor on or off,
-- revoked sessions, changed a role, created or removed an account. Kept a
-- year (the worker's daily sweep drops older rows); a deleted user's rows
-- stay, without the user.
CREATE TABLE security_events (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    at         TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- Whose account the event is about.
    user_id    UUID        REFERENCES users(id) ON DELETE SET NULL,
    -- Who did it, when not the user (an admin); NULL otherwise.
    actor_id   UUID        REFERENCES users(id) ON DELETE SET NULL,
    kind       TEXT        NOT NULL,
    detail     JSONB       NOT NULL DEFAULT '{}'::jsonb,
    ip         TEXT,
    user_agent TEXT
);
CREATE INDEX security_events_user_at_idx ON security_events (user_id, at DESC);
CREATE INDEX security_events_at_idx ON security_events (at);
