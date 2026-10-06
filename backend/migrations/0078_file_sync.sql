-- Migration 0078: file sync
-- ─────────────────────────
-- docs/file-sync-plan.md §5.3–§5.9. The server side of the own.audio folder:
-- where every item sits in it, a change log the sync feed reads, family
-- shortcuts, episodes the server stores on its own, and which devices hold
-- what.

-- ── Paths (§5.8) ────────────────────────────────────────────────────────────
--
-- The path belongs to the user: a file stays where and under the name it was
-- put. An item created without one gets a default from its metadata once, and
-- nothing changes it afterwards. A book's path is its folder; its files carry
-- their path inside it.
--
-- A separate table rather than a column on each kind: uniqueness is per owner
-- across kinds, and the book and track tables sit behind the trash views,
-- where a new column would mean recreating the views (CLAUDE.md §6).
--
-- A trashed item keeps its row, so a restore puts it back where it was. That
-- is also why uniqueness is enforced in code over the live items only
-- (`sync_live_paths`): a file deleted in Finder must not block a new file of
-- the same name. Comparison is case-insensitive, as on macOS and Windows.

CREATE TABLE sync_paths (
    kind       TEXT        NOT NULL CHECK (kind IN ('audiobook', 'music_track', 'podcast_episode')),
    item_id    UUID        NOT NULL,
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    path       TEXT        NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (kind, item_id)
);
CREATE INDEX sync_paths_owner_idx ON sync_paths (user_id, lower(path));

ALTER TABLE audiobook_files ADD COLUMN relative_path TEXT;

CREATE VIEW sync_live_paths AS
    SELECT p.* FROM sync_paths p
     WHERE (p.kind = 'audiobook'
            AND EXISTS (SELECT 1 FROM audiobook_books b WHERE b.id = p.item_id))
        OR (p.kind = 'music_track'
            AND EXISTS (SELECT 1 FROM music_tracks t WHERE t.id = p.item_id))
        OR (p.kind = 'podcast_episode'
            AND EXISTS (SELECT 1 FROM podcast_episodes e
                         WHERE e.id = p.item_id AND e.audio_object_id IS NOT NULL));

-- ── Change log (§5.3) ───────────────────────────────────────────────────────
--
-- The sync feed hands out "what changed since your cursor". A timestamp cursor
-- misses rows written by a long transaction (a book upload stamps `updated_at`
-- when it starts and commits minutes later), so triggers log every change that
-- matters for the tree with the writing transaction's id. The cursor is the
-- oldest transaction still running when the feed was read; the next read
-- starts there, so a late commit is always picked up and at worst an item is
-- sent twice.
--
-- A row only says "look at this item again"; the feed reads its current state
-- and sends it, or sends it as removed. That covers edits, trash, restore,
-- purge and a change of sharing alike.

CREATE TABLE sync_changes (
    seq             BIGSERIAL   PRIMARY KEY,
    xid             xid8        NOT NULL DEFAULT pg_current_xact_id(),
    kind            TEXT        NOT NULL,
    item_id         UUID        NOT NULL,
    -- No foreign keys: a deleted account's items must still reach its family.
    owner_id        UUID        NOT NULL,
    owner_family_id UUID,
    changed_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX sync_changes_xid_idx ON sync_changes (xid);
CREATE INDEX sync_changes_changed_at_idx ON sync_changes (changed_at);

CREATE FUNCTION sync_log(p_kind TEXT, p_item UUID, p_owner UUID) RETURNS VOID
LANGUAGE sql AS $$
    INSERT INTO sync_changes (kind, item_id, owner_id, owner_family_id)
    VALUES (p_kind, p_item, p_owner,
            (SELECT fm.family_id FROM family_members fm WHERE fm.user_id = p_owner));
$$;

CREATE FUNCTION sync_log_book() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM sync_log('audiobook', OLD.id, OLD.user_id);
        DELETE FROM sync_paths WHERE kind = 'audiobook' AND item_id = OLD.id;
        RETURN OLD;
    END IF;
    PERFORM sync_log('audiobook', NEW.id, NEW.user_id);
    IF TG_OP = 'UPDATE' AND OLD.user_id IS DISTINCT FROM NEW.user_id THEN
        PERFORM sync_log('audiobook', OLD.id, OLD.user_id);
    END IF;
    RETURN NEW;
END $$;

CREATE TRIGGER sync_log_book
    AFTER INSERT OR DELETE OR UPDATE OF user_id, family_id, trashed_at ON audiobook_books_all
    FOR EACH ROW EXECUTE FUNCTION sync_log_book();

CREATE FUNCTION sync_log_book_file() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    b UUID;
BEGIN
    IF TG_OP = 'DELETE' THEN b := OLD.book_id; ELSE b := NEW.book_id; END IF;
    PERFORM sync_log('audiobook', b, x.user_id) FROM audiobook_books_all x WHERE x.id = b;
    RETURN NULL;
END $$;

CREATE TRIGGER sync_log_book_file
    AFTER INSERT OR DELETE OR UPDATE OF position, relative_path, audio_object_id ON audiobook_files
    FOR EACH ROW EXECUTE FUNCTION sync_log_book_file();

CREATE FUNCTION sync_log_track() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM sync_log('music_track', OLD.id, OLD.user_id);
        DELETE FROM sync_paths WHERE kind = 'music_track' AND item_id = OLD.id;
        RETURN OLD;
    END IF;
    PERFORM sync_log('music_track', NEW.id, NEW.user_id);
    RETURN NEW;
END $$;

CREATE TRIGGER sync_log_track
    AFTER INSERT OR DELETE OR UPDATE OF user_id, family_id, trashed_at, audio_object_id ON music_tracks_all
    FOR EACH ROW EXECUTE FUNCTION sync_log_track();

-- An episode is in the tree only while it is stored, so only the stored copy's
-- comings and goings are logged — not every feed refresh touching the row.
CREATE FUNCTION sync_log_episode() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        DELETE FROM sync_paths WHERE kind = 'podcast_episode' AND item_id = OLD.id;
        IF OLD.audio_object_id IS NOT NULL OR OLD.trashed_at IS NOT NULL THEN
            PERFORM sync_log('podcast_episode', OLD.id, f.user_id) FROM podcast_feeds f WHERE f.id = OLD.feed_id;
        END IF;
        RETURN NULL;
    END IF;
    PERFORM sync_log('podcast_episode', NEW.id, f.user_id) FROM podcast_feeds f WHERE f.id = NEW.feed_id;
    RETURN NULL;
END $$;

CREATE TRIGGER sync_log_episode
    AFTER DELETE OR UPDATE OF audio_object_id, trashed_at ON podcast_episodes
    FOR EACH ROW EXECUTE FUNCTION sync_log_episode();

-- Sharing a show shares its stored episodes; the feed expands this row.
CREATE FUNCTION sync_log_feed() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    PERFORM sync_log('podcast_feed', NEW.id, NEW.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER sync_log_feed
    AFTER UPDATE OF user_id, family_id ON podcast_feeds
    FOR EACH ROW EXECUTE FUNCTION sync_log_feed();

CREATE FUNCTION sync_log_path() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP = 'DELETE' THEN
        PERFORM sync_log(OLD.kind, OLD.item_id, OLD.user_id);
    ELSE
        PERFORM sync_log(NEW.kind, NEW.item_id, NEW.user_id);
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER sync_log_path
    AFTER INSERT OR UPDATE OR DELETE ON sync_paths
    FOR EACH ROW EXECUTE FUNCTION sync_log_path();

-- A per-item grant changes who sees one item; log that item. Whole-kind
-- policies are left to the client's periodic reconciliation (§5.4).
CREATE FUNCTION sync_log_grant() RETURNS TRIGGER LANGUAGE plpgsql AS $$
DECLARE
    g content_grants;
BEGIN
    IF TG_OP = 'DELETE' THEN g := OLD; ELSE g := NEW; END IF;
    IF g.media_kind = 'audiobook' THEN
        PERFORM sync_log('audiobook', x.id, x.user_id) FROM audiobook_books_all x WHERE x.id = g.item_id;
    ELSIF g.media_kind = 'music' THEN
        PERFORM sync_log('music_track', x.id, x.user_id) FROM music_tracks_all x WHERE x.id = g.item_id;
    ELSIF g.media_kind = 'podcast' THEN
        PERFORM sync_log('podcast_feed', x.id, x.user_id) FROM podcast_feeds x WHERE x.id = g.item_id;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER sync_log_grant
    AFTER INSERT OR UPDATE OR DELETE ON content_grants
    FOR EACH ROW EXECUTE FUNCTION sync_log_grant();

-- ── Family shortcuts (§5.5) ─────────────────────────────────────────────────
--
-- "Add to own.audio folder" on a member's whole kind, or on one book, album
-- or show. An album has no id: identified albums are matched by release
-- group, others by album artist + album name.

CREATE TABLE sync_shortcuts (
    id             UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id        UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    member_id      UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind           TEXT        NOT NULL CHECK (kind IN ('audiobook', 'music', 'podcast')),
    container_kind TEXT        CHECK (container_kind IN ('book', 'album', 'show')),
    container_id   UUID,
    release_group  TEXT,
    album_artist   TEXT,
    album          TEXT,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK (member_id <> user_id),
    CHECK (container_kind IS NOT NULL OR (container_id IS NULL AND release_group IS NULL
                                          AND album_artist IS NULL AND album IS NULL))
);
CREATE UNIQUE INDEX sync_shortcuts_target_uq ON sync_shortcuts (
    user_id, member_id, kind, COALESCE(container_kind, ''), COALESCE(container_id::TEXT, ''),
    COALESCE(release_group, ''), COALESCE(lower(album_artist), ''), COALESCE(lower(album), ''));

-- Leaving a family ends every shortcut to or from the one who left.
CREATE FUNCTION sync_drop_shortcuts() RETURNS TRIGGER LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM sync_shortcuts WHERE user_id = OLD.user_id OR member_id = OLD.user_id;
    RETURN NULL;
END $$;

CREATE TRIGGER sync_drop_shortcuts
    AFTER DELETE ON family_members
    FOR EACH ROW EXECUTE FUNCTION sync_drop_shortcuts();

-- ── Storing new episodes on the server (§5.6) ───────────────────────────────

ALTER TABLE podcast_feeds ADD COLUMN auto_store_since TIMESTAMPTZ;
-- Set once auto-store has queued an episode, so an episode the family
-- deleted is never stored again by a later refresh.
ALTER TABLE podcast_episodes ADD COLUMN auto_stored_at TIMESTAMPTZ;

-- ── Device holdings (§5.9) ──────────────────────────────────────────────────
--
-- A device is the refresh-token chain of its session. Groundwork for "where
-- is this file"; a user only ever sees their own devices.

CREATE TABLE device_holdings (
    chain_id UUID        NOT NULL,
    user_id  UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind     TEXT        NOT NULL CHECK (kind IN ('audiobook', 'music_track', 'podcast_episode')),
    item_id  UUID        NOT NULL,
    since    TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, kind, item_id)
);
CREATE INDEX device_holdings_item_idx ON device_holdings (user_id, kind, item_id);
