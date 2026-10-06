-- Migration 0016: subsonic_api_keys
-- ────────────────────────────────────
-- A per-user credential used only by OpenSubsonic-compatible clients
-- (DSub, Symfonium, play:Sub, ...). Kept separate from the real account
-- password: Subsonic token auth is `token = md5(api_key + salt)`, which
-- requires the server to hold the key in a comparable (plaintext) form —
-- something that must never be true of the argon2-hashed login password.
-- Revocable/regenerable independently of the account password.

CREATE TABLE subsonic_api_keys (
    user_id    UUID        PRIMARY KEY REFERENCES users (id) ON DELETE CASCADE,
    api_key    TEXT        NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
