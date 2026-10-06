-- Migration 0002: users, auth identities, sessions
-- ─────────────────────────────────────────────────
-- Design rules:
--   • Ownership rows reference user.id (UUID), never usernames.
--   • One user may have multiple auth identities (local, google, microsoft).
--   • Sessions are stored server-side and can be individually revoked.

-- ── users ────────────────────────────────────────────────────────────────────
CREATE TABLE users (
    id           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    email        TEXT        NOT NULL UNIQUE,
    display_name TEXT        NOT NULL,
    role         TEXT        NOT NULL DEFAULT 'user'
                             CHECK (role IN ('user', 'admin')),
    is_active    BOOLEAN     NOT NULL DEFAULT true,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at   TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX users_email_idx ON users (email);

-- ── auth_identities ──────────────────────────────────────────────────────────
-- A user may authenticate via local credentials, Google, or Microsoft.
-- Each (provider, provider_subject) pair must be globally unique so that
-- the same external account cannot be linked to two different local users.
CREATE TABLE auth_identities (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id          UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    provider         TEXT        NOT NULL
                                 CHECK (provider IN ('local', 'google', 'microsoft')),
    -- OIDC subject from the external provider; NULL for local auth.
    provider_subject TEXT,
    -- Argon2 hash; only set for provider = 'local'.
    password_hash    TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- Each user has at most one identity per provider.
    CONSTRAINT auth_identities_user_provider_uq UNIQUE (user_id, provider),
    -- The (provider, subject) pair must be globally unique for OIDC providers.
    CONSTRAINT auth_identities_provider_subject_uq UNIQUE (provider, provider_subject)
);

-- ── sessions ─────────────────────────────────────────────────────────────────
-- Server-side session log. The JWT jti maps to sessions.id.
-- Revocation is checked on every authenticated request.
CREATE TABLE sessions (
    id           UUID        PRIMARY KEY,              -- = JWT jti
    user_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    created_at   TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at   TIMESTAMPTZ NOT NULL,
    revoked_at   TIMESTAMPTZ,                          -- NULL = active
    user_agent   TEXT,
    ip_address   INET
);

CREATE INDEX sessions_user_id_idx    ON sessions (user_id);
CREATE INDEX sessions_expires_at_idx ON sessions (expires_at);
