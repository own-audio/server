-- The own.audio folder's remainder on /api/v2 (docs/api-v2-completion-plan.md, phase 11).
--
-- 1. Companion files are a kind of the change feed, `file`: a new file, a share, a move in
--    the folder, the trash and its restore and purge each send the file again (or its
--    removal). 0079 logged them for v1's sync tree only.
-- 2. A listener's folder shortcuts are their own state, `shortcut`: one device adds one, the
--    others learn it from the feed.
-- 3. An uploaded show goes to the trash with its episodes, as books and tracks do (0077):
--    `podcast_feeds` becomes a view over `podcast_feeds_all` that hides the trash, so every
--    query that names the show keeps working and cannot see a trashed one. A followed show is
--    still unfollowed outright; only uploaded ones are trashed.

-- ── 1. Companion files ──────────────────────────────────────────────────

CREATE TRIGGER companion_files_change AFTER INSERT OR UPDATE OR DELETE ON companion_files_all
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('file');

-- 0126's function, with companion files: a move or rename is a change of the file.
CREATE OR REPLACE FUNCTION change_log_path() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log(CASE r.kind WHEN 'music_track' THEN 'track' WHEN 'audiobook' THEN 'book'
                                   WHEN 'podcast_episode' THEN 'episode' WHEN 'companion_file' THEN 'file' END,
                       r.item_id, music_owner_family(r.user_id), NULL)
     WHERE r.kind IN ('music_track', 'audiobook', 'podcast_episode', 'companion_file');
    RETURN NULL;
END $$;

-- ── 2. Shortcuts ────────────────────────────────────────────────────────

CREATE FUNCTION change_log_shortcut() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log('shortcut', r.id, NULL, r.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER sync_shortcuts_change AFTER INSERT OR UPDATE OR DELETE ON sync_shortcuts
    FOR EACH ROW EXECUTE FUNCTION change_log_shortcut();

-- ── 3. Uploaded shows in the trash ──────────────────────────────────────

ALTER TABLE podcast_feeds
    ADD COLUMN trashed_at  TIMESTAMPTZ,
    ADD COLUMN trashed_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN trash_batch UUID;
ALTER TABLE podcast_feeds RENAME TO podcast_feeds_all;
CREATE VIEW podcast_feeds AS
    SELECT * FROM podcast_feeds_all WHERE trashed_at IS NULL;
CREATE INDEX podcast_feeds_trashed_idx ON podcast_feeds_all (trashed_at) WHERE trashed_at IS NOT NULL;

-- A show in the trash keeps its guid, but a new show of the same guid may be made meanwhile;
-- the restore then brings the old one back without it (`db::trash::restore`).
DROP INDEX podcast_feeds_upload_guid_uq;
CREATE UNIQUE INDEX podcast_feeds_upload_guid_uq ON podcast_feeds_all (user_id, podcast_guid)
    WHERE source_type = 'upload' AND podcast_guid IS NOT NULL AND trashed_at IS NULL;

-- Trashing or restoring a show sends it again with its episodes (0126), and v1's sync tree
-- its stored episodes (0078).
DROP TRIGGER podcast_feeds_seen_change ON podcast_feeds_all;
CREATE TRIGGER podcast_feeds_seen_change
    AFTER UPDATE OF user_id, family_id, trashed_at ON podcast_feeds_all
    FOR EACH ROW WHEN (OLD.user_id IS DISTINCT FROM NEW.user_id OR OLD.family_id IS DISTINCT FROM NEW.family_id
                       OR OLD.trashed_at IS DISTINCT FROM NEW.trashed_at)
    EXECUTE FUNCTION change_log_shelf_seen('podcast');
DROP TRIGGER sync_log_feed ON podcast_feeds_all;
CREATE TRIGGER sync_log_feed
    AFTER UPDATE OF user_id, family_id, trashed_at ON podcast_feeds_all
    FOR EACH ROW EXECUTE FUNCTION sync_log_feed();
