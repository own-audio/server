-- Migration 0079: companion files
-- ────────────────────────────────
-- docs/file-sync-plan.md §2 item 16. Images, PDF booklets, `.lrc`, `.cue`,
-- `.txt`/`.nfo` put next to music or in a book folder are kept as they are,
-- at their path, and synced like the audio. They belong to a folder, not to
-- an item — an album has no row of its own — so they are items of their own
-- kind, with the owner, visibility and trash every other item has.
--
-- Same shape as migration 0077: a base table `…_all` and a view hiding the
-- trash, so nothing but the trash code can see a trashed file.

CREATE TABLE companion_files_all (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    family_id   UUID        REFERENCES families (id) ON DELETE SET NULL,
    -- Which kind's member policy decides who in the family sees it: the
    -- top-level folder it sits in.
    media_kind  TEXT        NOT NULL CHECK (media_kind IN ('music', 'audiobook')),
    object_id   UUID        NOT NULL REFERENCES media_objects (id),
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    trashed_at  TIMESTAMPTZ,
    trashed_by  UUID        REFERENCES users (id) ON DELETE SET NULL,
    trash_batch UUID
);
CREATE INDEX companion_files_user_idx ON companion_files_all (user_id);
CREATE INDEX companion_files_trashed_idx ON companion_files_all (trashed_at) WHERE trashed_at IS NOT NULL;
CREATE VIEW companion_files AS
    SELECT * FROM companion_files_all WHERE trashed_at IS NULL;

-- Paths: one more kind in the per-owner namespace.
ALTER TABLE sync_paths DROP CONSTRAINT sync_paths_kind_check;
ALTER TABLE sync_paths ADD CONSTRAINT sync_paths_kind_check
    CHECK (kind IN ('audiobook', 'music_track', 'podcast_episode', 'companion_file'));

CREATE OR REPLACE VIEW sync_live_paths AS
    SELECT p.* FROM sync_paths p
     WHERE (p.kind = 'audiobook'
            AND EXISTS (SELECT 1 FROM audiobook_books b WHERE b.id = p.item_id))
        OR (p.kind = 'music_track'
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.id = p.item_id))
        OR (p.kind = 'podcast_episode'
            AND EXISTS (SELECT 1 FROM podcast_episodes e
                         WHERE e.id = p.item_id AND e.audio_object_id IS NOT NULL))
        OR (p.kind = 'companion_file'
            AND EXISTS (SELECT 1 FROM companion_files c WHERE c.id = p.item_id));

ALTER TABLE device_holdings DROP CONSTRAINT device_holdings_kind_check;
ALTER TABLE device_holdings ADD CONSTRAINT device_holdings_kind_check
    CHECK (kind IN ('audiobook', 'music_track', 'podcast_episode', 'companion_file'));

CREATE FUNCTION sync_log_companion() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM sync_log('companion_file', OLD.id, OLD.user_id);
        DELETE FROM sync_paths WHERE kind = 'companion_file' AND item_id = OLD.id;
        RETURN OLD;
    END IF;
    PERFORM sync_log('companion_file', NEW.id, NEW.user_id);
    RETURN NEW;
END $$;

CREATE TRIGGER sync_log_companion
    AFTER INSERT OR DELETE OR UPDATE OF user_id, family_id, trashed_at, object_id ON companion_files_all
    FOR EACH ROW EXECUTE FUNCTION sync_log_companion();
