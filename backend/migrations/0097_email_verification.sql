-- Whether an account's email address was ever proven (security hardening plan
-- §5.1, H6): by a sign-in provider that vouches for it, by claiming an invite
-- sent to it, by the admin who typed it, or by the mailed link. Everyone who
-- exists already counts as verified — made by an admin, an invite or a
-- provider, or registered before there was anything to verify against.
ALTER TABLE users ADD COLUMN email_verified_at TIMESTAMPTZ;
UPDATE users SET email_verified_at = created_at;

-- The mailed link's secret, stored as a hash; one use, 24 hours.
CREATE TABLE email_verifications (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id    UUID        NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    token_hash TEXT        NOT NULL UNIQUE,
    expires_at TIMESTAMPTZ NOT NULL,
    used_at    TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX email_verifications_user_idx ON email_verifications (user_id);
