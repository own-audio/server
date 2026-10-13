-- Every change to a song's, an album's or a contributor's metadata made by
-- someone (docs/metadata-organizer-plan.md, O0): who, how, in which batch, and
-- the fields before and after, so any of it can be undone. A write marks its
-- transaction with `own.edit` (db::edits::mark); unmarked writes — a scan
-- reading a file, a background lookup — are not edits and leave no row.

CREATE TABLE metadata_edits (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    item_kind  TEXT        NOT NULL CHECK (item_kind IN ('track', 'album', 'contributor')),
    item_id    UUID        NOT NULL,
    family_id  UUID,
    user_id    UUID,
    -- 'manual', 'bulk', 'identify', 'run', 'revert', 'undo'.
    source     TEXT        NOT NULL,
    -- One save, one run or one undo: undone together.
    batch_id   UUID        NOT NULL,
    -- Only the fields that changed.
    before     JSONB       NOT NULL,
    after      JSONB       NOT NULL,
    at         TIMESTAMPTZ NOT NULL DEFAULT clock_timestamp(),
    undone_at  TIMESTAMPTZ
);
CREATE INDEX metadata_edits_item_idx ON metadata_edits (item_kind, item_id, at DESC);
CREATE INDEX metadata_edits_batch_idx ON metadata_edits (batch_id);
CREATE INDEX metadata_edits_family_idx ON metadata_edits (family_id, at DESC);

-- TG_ARGV: the item kind, then the columns that count as its metadata.
CREATE FUNCTION metadata_edit_capture() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    ctx JSONB := NULLIF(current_setting('own.edit', true), '')::jsonb;
    o   JSONB;
    n   JSONB;
    b   JSONB := '{}';
    a   JSONB := '{}';
    i   INT;
BEGIN
    IF ctx IS NULL THEN
        RETURN NULL;
    END IF;
    o := to_jsonb(OLD);
    n := to_jsonb(NEW);
    FOR i IN 1 .. TG_NARGS - 1 LOOP
        IF o -> TG_ARGV[i] IS DISTINCT FROM n -> TG_ARGV[i] THEN
            b := b || jsonb_build_object(TG_ARGV[i], o -> TG_ARGV[i]);
            a := a || jsonb_build_object(TG_ARGV[i], n -> TG_ARGV[i]);
        END IF;
    END LOOP;
    IF a = '{}' THEN
        RETURN NULL;
    END IF;
    INSERT INTO metadata_edits (item_kind, item_id, family_id, user_id, source, batch_id, before, after)
    VALUES (TG_ARGV[0], NEW.id, (n ->> 'family_id')::uuid, (ctx ->> 'user_id')::uuid,
            ctx ->> 'source', (ctx ->> 'batch_id')::uuid, b, a);
    RETURN NULL;
END $$;

CREATE TRIGGER music_tracks_edit AFTER UPDATE ON music_tracks_all
    FOR EACH ROW EXECUTE FUNCTION metadata_edit_capture(
        'track', 'title', 'artist', 'album', 'album_artist', 'genre', 'track_number', 'disc_number',
        'disc_total', 'is_compilation', 'musicbrainz_recording_id', 'musicbrainz_release_id',
        'musicbrainz_artist_id', 'musicbrainz_release_group_id', 'cover_object_id');

CREATE TRIGGER albums_edit AFTER UPDATE ON albums
    FOR EACH ROW EXECUTE FUNCTION metadata_edit_capture(
        'album', 'title', 'release_date', 'country', 'label', 'barcode', 'is_compilation',
        'musicbrainz_release_id', 'release_group_id');

CREATE TRIGGER contributors_edit AFTER UPDATE ON contributors
    FOR EACH ROW EXECUTE FUNCTION metadata_edit_capture(
        'contributor', 'name', 'sort_name', 'kind', 'musicbrainz_artist_id', 'biography');

-- An item gone for good takes its history with it.
CREATE FUNCTION metadata_edits_forget() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM metadata_edits WHERE item_kind = TG_ARGV[0] AND item_id = OLD.id;
    RETURN NULL;
END $$;

CREATE TRIGGER music_tracks_edit_forget AFTER DELETE ON music_tracks_all
    FOR EACH ROW EXECUTE FUNCTION metadata_edits_forget('track');
CREATE TRIGGER albums_edit_forget AFTER DELETE ON albums
    FOR EACH ROW EXECUTE FUNCTION metadata_edits_forget('album');
CREATE TRIGGER contributors_edit_forget AFTER DELETE ON contributors
    FOR EACH ROW EXECUTE FUNCTION metadata_edits_forget('contributor');
