-- The play queue, listening settings and the notification inbox in the change feed
-- (docs/api-v2-completion-plan.md, phase 8; optimisation 9). Each is a listener's own state,
-- logged for that listener only, so another device learns of it from `/sync/changes` and a
-- `library` event on `/events` instead of polling.
--
-- 1. The queue: one row per listener, `queue` with the listener's id. A revision counts every
--    write, whoever makes it (v1, Subsonic's savePlayQueue, v2), so `PUT /api/v2/me/queue`
--    can refuse a write made against an older queue (`If-Match`).
-- 2. Settings: the player's settings and each device kind's equaliser are one record,
--    `settings` with the listener's id.
-- 3. Notifications: `notification` with the notification's id, when one arrives and when it is
--    read. The inbox is read newest first per listener, so it gets that index.

-- ── 1. The queue ─────────────────────────────────────────────────────────

ALTER TABLE play_queues ADD COLUMN revision BIGINT NOT NULL DEFAULT 0;
-- Rows written before this migration start at 1, so `0` only ever means "no queue yet".
UPDATE play_queues SET revision = 1;

CREATE FUNCTION play_queue_revision() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'UPDATE' THEN
        NEW.revision := OLD.revision + 1;
    ELSE
        NEW.revision := 1;
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER play_queues_revision BEFORE INSERT OR UPDATE ON play_queues
    FOR EACH ROW EXECUTE FUNCTION play_queue_revision();

-- A listener's record of one kind, named by the listener's id.
CREATE FUNCTION change_log_listener() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log(TG_ARGV[0], r.user_id, NULL, r.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER play_queues_change AFTER INSERT OR UPDATE OR DELETE ON play_queues
    FOR EACH ROW EXECUTE FUNCTION change_log_listener('queue');

-- ── 2. Settings ──────────────────────────────────────────────────────────

CREATE TRIGGER user_settings_change AFTER INSERT OR UPDATE OR DELETE ON user_settings
    FOR EACH ROW EXECUTE FUNCTION change_log_listener('settings');
CREATE TRIGGER eq_settings_change AFTER INSERT OR UPDATE OR DELETE ON eq_settings
    FOR EACH ROW EXECUTE FUNCTION change_log_listener('settings');

-- ── 3. Notifications ─────────────────────────────────────────────────────

CREATE FUNCTION change_log_notification() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log('notification', r.id, NULL, r.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER pending_notifications_change AFTER INSERT OR UPDATE OR DELETE ON pending_notifications
    FOR EACH ROW EXECUTE FUNCTION change_log_notification();

CREATE INDEX pending_notifications_user_time_idx ON pending_notifications (user_id, created_at DESC, id DESC);
