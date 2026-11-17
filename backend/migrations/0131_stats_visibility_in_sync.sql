-- Who sees a listener's statistics is part of their `settings` record
-- (docs/api-v2-completion-plan.md, phase 9): a change, whether the listener's own or a household
-- admin's for an account they manage, reaches the listener's devices through the change feed.
-- The plays history and the statistics read `listening_sessions` through its
-- `(user_id, started_at)` index; no new index (CAPACITY.md).

CREATE FUNCTION change_log_stats_visibility() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('settings', NEW.user_id, NULL, NEW.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER family_members_stats_visibility_change AFTER UPDATE OF stats_visibility ON family_members
    FOR EACH ROW WHEN (OLD.stats_visibility IS DISTINCT FROM NEW.stats_visibility)
    EXECUTE FUNCTION change_log_stats_visibility();
