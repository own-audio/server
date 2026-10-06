-- Migration 0061: Phase 6 of docs/podcast-recommendations-plan.md — the switch
-- that turns listening-derived recommendations on.
--
-- **`DEFAULT false` is the entire point of this migration.** Recommendations
-- start off for everyone. A later migration that flips this default, or a
-- backfill that sets existing rows true, silently reverses a decision the
-- product made deliberately — see the plan's section 0.1.
--
-- The device-side architecture means an on-by-default version would have been
-- lawful: nothing is computed about a user on a server, so nothing new is
-- collected by switching it on. It starts off anyway, because the promise on
-- the marketing page is unconditional and a product that has to explain why
-- its default is technically fine has already lost the argument.
--
-- Per user, never per family. `family_members.stats_visibility` (migration
-- 0020) already defaults to 'private' so a family admin cannot read a member's
-- listening; the same reasoning says an admin cannot switch this on for them.

ALTER TABLE users
    ADD COLUMN recommendations_enabled BOOLEAN NOT NULL DEFAULT false,
    -- When the user last changed it. Not a consent record in the GDPR Art. 7
    -- sense — no server-side processing of personal data is being consented
    -- to, because there is none — but the timestamp costs nothing and is the
    -- only evidence available if that ever needs showing.
    ADD COLUMN recommendations_changed_at TIMESTAMPTZ;
