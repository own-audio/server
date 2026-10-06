-- Migration 0062: a Subsonic API key for every user
-- ─────────────────────────────────────────────────
-- The key was created lazily, on the first `GET /users/me/subsonic-key`.
-- Until someone opened Settings → Subsonic in the web console there was no
-- key row at all, so `/rest` auth rejected them with error 40 "Wrong username
-- or password" no matter what they typed — indistinguishable, from inside a
-- Subsonic client, from a genuinely wrong credential. Every account now has a
-- key from the moment it exists (see `db::users::insert`); this backfills the
-- accounts created before that.
--
-- gen_random_uuid() is Postgres' CSPRNG-backed generator; stripped of its
-- dashes it yields exactly the 32 alphanumeric characters `generate_key()`
-- produces in Rust.

INSERT INTO subsonic_api_keys (user_id, api_key)
SELECT u.id, replace(gen_random_uuid()::text, '-', '')
FROM users u
WHERE NOT EXISTS (
    SELECT 1 FROM subsonic_api_keys k WHERE k.user_id = u.id
);
