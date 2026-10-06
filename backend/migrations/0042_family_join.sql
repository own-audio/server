-- Migration 0042: frictionless family join (docs/family-join-qr-plan.md)
-- ──────────────────────────────────────────────────────────────────────
-- Three invite kinds in one table:
--   • email — today's flow: bound to one address, single-use.
--   • link  — email-free shareable link/QR, optionally multi-use,
--             member role only (an admin role must never be grantable
--             by whoever happens to photograph the fridge QR).
--   • claim — bound to a pre-provisioned account (created by a family
--             admin with no password); redeeming it sets the password
--             and signs the member in.
-- `use_count`/`max_uses` supersede `accepted_at IS NULL` as the spent
-- check; `accepted_at`/`accepted_by` keep recording the *latest*
-- acceptance for audit.

ALTER TABLE family_invites
    ALTER COLUMN email DROP NOT NULL;

ALTER TABLE family_invites
    ADD COLUMN kind          TEXT    NOT NULL DEFAULT 'email'
        CHECK (kind IN ('email', 'link', 'claim')),
    ADD COLUMN max_uses      INTEGER NOT NULL DEFAULT 1
        CHECK (max_uses BETWEEN 1 AND 20),
    ADD COLUMN use_count     INTEGER NOT NULL DEFAULT 0
        CHECK (use_count >= 0),
    ADD COLUMN label         TEXT,
    ADD COLUMN claim_user_id UUID REFERENCES users (id) ON DELETE CASCADE;

-- Backfill spent markers so existing accepted invites read as exhausted
-- under the new check.
UPDATE family_invites SET use_count = 1 WHERE accepted_at IS NOT NULL;

ALTER TABLE family_invites
    ADD CONSTRAINT family_invites_kind_shape CHECK (
        (kind = 'email' AND email IS NOT NULL AND claim_user_id IS NULL AND max_uses = 1)
     OR (kind = 'link'  AND email IS NULL     AND claim_user_id IS NULL AND role = 'member')
     OR (kind = 'claim' AND email IS NULL     AND claim_user_id IS NOT NULL AND max_uses = 1
         AND role = 'member')
    );

CREATE INDEX family_invites_claim_user_idx
    ON family_invites (claim_user_id) WHERE claim_user_id IS NOT NULL;
