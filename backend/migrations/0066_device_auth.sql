-- Signing in on a device with no comfortable keyboard: the TV shows a short code,
-- and someone already signed in approves it on a phone or the web (RFC 8628 in
-- shape, not to the letter — this is our own client talking to our own server).
--
-- The TV polls with `device_code`, which is a credential: only its SHA-256 is
-- stored, exactly as refresh tokens are (migration 0017). `user_code` is the
-- short thing a human reads off the screen and is stored in the clear, because
-- the approver has to be able to look it up by typing it.
CREATE TABLE device_auth_requests (
    id                UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    device_code_hash  TEXT        NOT NULL UNIQUE,
    user_code         TEXT        NOT NULL UNIQUE,
    device_name       TEXT,
    device_kind       TEXT        NOT NULL DEFAULT 'other',
    status            TEXT        NOT NULL DEFAULT 'pending'
                      CHECK (status IN ('pending', 'approved', 'denied', 'consumed')),
    approved_user_id  UUID        REFERENCES users(id) ON DELETE CASCADE,
    -- Rate limiting for the poll loop: a client that polls faster than the
    -- interval it was given is told to slow down rather than served.
    last_polled_at    TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at        TIMESTAMPTZ NOT NULL
);

-- Looking a pending request up by the code the person typed.
CREATE INDEX device_auth_requests_user_code_idx ON device_auth_requests (user_code);
-- Sweeping expired rows.
CREATE INDEX device_auth_requests_expires_at_idx ON device_auth_requests (expires_at);
