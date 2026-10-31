-- OpenSubsonic's apiKeyAuthentication sends the key alone, without a user
-- name, so the server has to find the person from the key. The key itself is
-- stored encrypted (auth::at_rest) and can't be searched; its SHA-256 can.
-- Filled in for existing keys at start-up (db::subsonic::backfill_key_hashes).
ALTER TABLE subsonic_api_keys ADD COLUMN IF NOT EXISTS key_hash TEXT;
CREATE UNIQUE INDEX IF NOT EXISTS subsonic_api_keys_key_hash
    ON subsonic_api_keys (key_hash) WHERE key_hash IS NOT NULL;
