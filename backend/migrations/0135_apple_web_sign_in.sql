-- Sign in with Apple in a browser, for apps without Apple's SDK (Android).
-- `GET /api/v2/auth/apple/start` records a row and sends the browser to Apple;
-- Apple posts its answer to `/api/v2/auth/apple/return`, which checks the id
-- token, keeps who signed in and hands the app a one-time code; the app spends
-- the code with its PKCE verifier at `POST /api/v2/auth/token`. Rows live five
-- minutes and are deleted when the code is spent, so the identity is kept no
-- longer than the sign-in takes.
--
-- Secrets are stored as SHA-256 only (the state Apple carries, the app's code),
-- as for refresh tokens (0017) and device codes (0066). The nonce is kept in
-- the clear: it comes back inside the id token anyway.
CREATE TABLE apple_web_sign_ins (
    id              UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    state_hash      TEXT        NOT NULL UNIQUE,
    nonce           TEXT        NOT NULL,
    code_challenge  TEXT        NOT NULL,
    redirect_uri    TEXT        NOT NULL,
    app_state       TEXT,
    status          TEXT        NOT NULL DEFAULT 'started'
                    CHECK (status IN ('started', 'returned', 'signed')),
    code_hash       TEXT        UNIQUE,
    subject         TEXT,
    email           TEXT,
    email_verified  BOOLEAN     NOT NULL DEFAULT false,
    full_name       TEXT,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at      TIMESTAMPTZ NOT NULL
);

-- Sweeping old rows.
CREATE INDEX apple_web_sign_ins_expires_at_idx ON apple_web_sign_ins (expires_at);
