-- Migration 0018: families, membership, invites
-- ──────────────────────────────────────────────
-- Design rules:
--   • A family is a user group. Every user belongs to exactly one family
--     (UNIQUE on family_members.user_id); solo users get a "personal family
--     of one" so handler code never has to branch on "has a family or not".
--   • Family roles are scoped to the family and independent of the global
--     users.role: a family_admin is not an instance admin, and vice versa.
--   • Invites are bearer codes bound to an email address; accepting one
--     moves the user out of their previous family.

CREATE TABLE families (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    name       TEXT        NOT NULL,
    created_by UUID        REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE family_members (
    family_id     UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    -- UNIQUE (not just PK-member): one family per user in v1.
    user_id       UUID        NOT NULL UNIQUE REFERENCES users (id) ON DELETE CASCADE,
    role          TEXT        NOT NULL DEFAULT 'member'
                              CHECK (role IN ('family_admin', 'member')),
    -- Optional in-family label ("Dad", "Ida") shown instead of display_name.
    display_label TEXT,
    joined_at     TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (family_id, user_id)
);

CREATE INDEX family_members_user_idx ON family_members (user_id);

CREATE TABLE family_invites (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id   UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    -- The invite is only redeemable by an account with this email.
    email       TEXT        NOT NULL,
    code        TEXT        NOT NULL UNIQUE,
    role        TEXT        NOT NULL DEFAULT 'member'
                            CHECK (role IN ('family_admin', 'member')),
    created_by  UUID        REFERENCES users (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at  TIMESTAMPTZ NOT NULL,
    accepted_at TIMESTAMPTZ,
    accepted_by UUID        REFERENCES users (id) ON DELETE SET NULL
);

CREATE INDEX family_invites_family_idx ON family_invites (family_id);
CREATE INDEX family_invites_email_idx  ON family_invites (lower(email));

-- ── Backfill ─────────────────────────────────────────────────────────────────
-- Every existing user becomes the family_admin of their own personal family.
-- created_by doubles as the join key: each user creates exactly one row here.
WITH created AS (
    INSERT INTO families (name, created_by, created_at)
    SELECT u.display_name, u.id, u.created_at
    FROM users u
    RETURNING id, created_by
)
INSERT INTO family_members (family_id, user_id, role)
SELECT c.id, c.created_by, 'family_admin'
FROM created c
WHERE c.created_by IS NOT NULL;
