-- Migration 0056: who in a family may add content, and an age bracket
-- ────────────────────────────────────────────────────────────────────
-- See docs/family-permissions-plan.md. Distinct from the `member_policy`
-- tables (db::access): those govern what a member may *see*, these govern
-- what a member may *create*.
--
-- Driven by Google Play's Families Policy as much as by product need — the
-- Android app is declared for children, and an app where any member can
-- upload arbitrary content and generate AI narration is unmoderated UGC.
--
-- Defaults are deliberate: every existing row becomes adult/true/true, so
-- applying this changes nobody's behaviour. Restrictions only ever arrive by
-- an admin choosing them.
--
-- Bracket, never a birthdate: a date of birth is itself a child's personal
-- data and brings obligations a coarse bracket does not. It is set by the
-- parent at invite time, so the child never answers an age question.

ALTER TABLE family_members
    ADD COLUMN age_bracket  TEXT    NOT NULL DEFAULT 'adult'
        CHECK (age_bracket IN ('adult', 'teen', 'child')),
    ADD COLUMN can_upload   BOOLEAN NOT NULL DEFAULT TRUE,
    ADD COLUMN can_generate BOOLEAN NOT NULL DEFAULT TRUE;

-- The compliance invariant, as a constraint rather than a default an admin
-- (or a future endpoint) could silently undo: a child never produces content.
ALTER TABLE family_members ADD CONSTRAINT family_members_child_no_ugc
    CHECK (age_bracket <> 'child' OR (can_upload = FALSE AND can_generate = FALSE));

-- Deliberately NOT adding these to `family_invites` yet: nothing would read
-- them, and dead schema reads as done. A provisioned account cannot be signed
-- into until it is claimed (it has no auth identity), so there is no window in
-- which a new child member could upload before an admin sets the bracket --
-- two calls are safe. Invite-time brackets land with the admin UI.
