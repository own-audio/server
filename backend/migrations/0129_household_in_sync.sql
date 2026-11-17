-- The household in the change feed, and the content reports' answers
-- (docs/api-v2-completion-plan.md, phase 7).
--
-- 1. The household becomes a kind of its own, `family`: its name and picture for everyone in
--    it, a member's own role and rights for that member (the record is `GET /family`, which
--    carries the reader's membership).
-- 2. A member blocked or let back in: their `member` record (it says `blocked`), as a new
--    name or picture already does (0126).
-- 3. A report's resolution may carry a note, and the list reads resolved reports too.

-- ── 1. The household ─────────────────────────────────────────────────────

CREATE FUNCTION change_log_family() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('family', NEW.id, NEW.id, NULL);
    RETURN NULL;
END $$;

CREATE TRIGGER families_change AFTER UPDATE OF name, avatar_object_id ON families
    FOR EACH ROW WHEN (OLD.name IS DISTINCT FROM NEW.name OR OLD.avatar_object_id IS DISTINCT FROM NEW.avatar_object_id)
    EXECUTE FUNCTION change_log_family();

-- What a member may do is part of their own copy of the household, nobody else's.
CREATE FUNCTION change_log_membership() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('family', NEW.family_id, NULL, NEW.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER family_members_rights_change
    AFTER UPDATE OF role, can_upload, can_generate, age_bracket ON family_members
    FOR EACH ROW WHEN (OLD.role IS DISTINCT FROM NEW.role OR OLD.can_upload IS DISTINCT FROM NEW.can_upload
                       OR OLD.can_generate IS DISTINCT FROM NEW.can_generate OR OLD.age_bracket IS DISTINCT FROM NEW.age_bracket)
    EXECUTE FUNCTION change_log_membership();

-- ── 2. Blocked ───────────────────────────────────────────────────────────

DROP TRIGGER users_member_change ON users;
CREATE TRIGGER users_member_change AFTER UPDATE OF display_name, avatar_object_id, is_active ON users
    FOR EACH ROW WHEN (OLD.display_name IS DISTINCT FROM NEW.display_name OR OLD.avatar_object_id IS DISTINCT FROM NEW.avatar_object_id
                       OR OLD.is_active IS DISTINCT FROM NEW.is_active)
    EXECUTE FUNCTION change_log_user();

-- ── 3. Reports ───────────────────────────────────────────────────────────

ALTER TABLE content_reports ADD COLUMN resolution_note TEXT;

-- Every report of a household, newest first, for the list's `state=all` and `state=resolved`;
-- the open ones keep 0057's partial index.
CREATE INDEX content_reports_family_idx ON content_reports (family_id, created_at DESC, id DESC);
