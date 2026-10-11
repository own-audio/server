-- The change feed (docs/api-v2-plan.md, V5; DESIGN §8): every change to the
-- catalogue and to a listener's own state, for offline clients. The model is
-- the file-sync log of 0078, generalised: triggers write one row per item per
-- transaction, and `GET /api/v2/sync/changes` reads the rows after a cursor of
-- transaction ids and sends each item's current state, or its removal.
--
-- A row names the household that may see it (`family_id`) or, for a
-- listener's own state, the listener (`user_id`). What a reader may actually
-- see is decided when it reads, with the usual visibility rules.

CREATE TABLE changes (
    seq        BIGSERIAL   PRIMARY KEY,
    xid        xid8        NOT NULL DEFAULT pg_current_xact_id(),
    -- `track`, `album`, …; `favourite.track`, `progress.book`, …; `bookmark`;
    -- `reset` when a household's library changed too much to send row by row.
    kind       TEXT        NOT NULL,
    item_id    UUID        NOT NULL,
    family_id  UUID,
    user_id    UUID,
    changed_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
-- One row per item per transaction, however many statements touched it.
CREATE UNIQUE INDEX changes_once_idx ON changes (xid, kind, item_id, COALESCE(user_id, '00000000-0000-0000-0000-000000000000'));
CREATE INDEX changes_xid_idx ON changes (xid);
CREATE INDEX changes_changed_at_idx ON changes (changed_at);

CREATE FUNCTION change_log(p_kind TEXT, p_item UUID, p_family UUID, p_user UUID) RETURNS void
LANGUAGE sql AS $$
    INSERT INTO changes (kind, item_id, family_id, user_id)
    SELECT p_kind, p_item, p_family, p_user
     WHERE p_item IS NOT NULL AND (p_family IS NOT NULL OR p_user IS NOT NULL)
       AND current_setting('own_audio.quiet', true) IS DISTINCT FROM 'on'
    ON CONFLICT DO NOTHING
$$;

-- ── The catalogue ────────────────────────────────────────────────────────

-- Entities that carry their household.
CREATE FUNCTION change_log_entity() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log(TG_ARGV[0], r.id, r.family_id, NULL);
    RETURN NULL;
END $$;

CREATE TRIGGER contributors_change AFTER INSERT OR UPDATE OR DELETE ON contributors
    FOR EACH ROW EXECUTE FUNCTION change_log_entity('contributor');
CREATE TRIGGER albums_change AFTER INSERT OR UPDATE OR DELETE ON albums
    FOR EACH ROW EXECUTE FUNCTION change_log_entity('album');
CREATE TRIGGER release_groups_change AFTER INSERT OR UPDATE OR DELETE ON release_groups
    FOR EACH ROW EXECUTE FUNCTION change_log_entity('release_group');
CREATE TRIGGER recordings_change AFTER INSERT OR UPDATE OR DELETE ON recordings
    FOR EACH ROW EXECUTE FUNCTION change_log_entity('recording');
CREATE TRIGGER genres_change AFTER INSERT OR UPDATE OR DELETE ON genres
    FOR EACH ROW EXECUTE FUNCTION change_log_entity('genre');

-- Items, which belong to their owner's household.
CREATE FUNCTION change_log_owned() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log(TG_ARGV[0], r.id, music_owner_family(r.user_id), NULL);
    RETURN NULL;
END $$;

CREATE TRIGGER music_tracks_change AFTER INSERT OR UPDATE OR DELETE ON music_tracks_all
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('track');
CREATE TRIGGER audiobook_books_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_books_all
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('book');
CREATE TRIGGER podcast_feeds_change AFTER INSERT OR UPDATE OR DELETE ON podcast_feeds
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('podcast');

-- Rows that are part of another item: the item changed.
CREATE FUNCTION change_log_part() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
    v_item UUID;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    CASE TG_ARGV[0]
        WHEN 'track_genre' THEN
            PERFORM change_log('track', r.track_id, music_owner_family(t.user_id), NULL) FROM music_tracks_all t WHERE t.id = r.track_id;
        WHEN 'book_part' THEN
            PERFORM change_log('book', r.book_id, music_owner_family(b.user_id), NULL) FROM audiobook_books_all b WHERE b.id = r.book_id;
        WHEN 'episode' THEN
            PERFORM change_log('episode', r.id, music_owner_family(f.user_id), NULL) FROM podcast_feeds f WHERE f.id = r.feed_id;
        WHEN 'credit' THEN
            PERFORM change_log(r.item_kind, r.item_id, k.family_id, NULL) FROM contributors k WHERE k.id = r.contributor_id;
    END CASE;
    RETURN NULL;
END $$;

CREATE TRIGGER track_genres_change AFTER INSERT OR UPDATE OR DELETE ON track_genres
    FOR EACH ROW EXECUTE FUNCTION change_log_part('track_genre');
CREATE TRIGGER audiobook_files_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_files
    FOR EACH ROW EXECUTE FUNCTION change_log_part('book_part');
CREATE TRIGGER audiobook_chapters_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_chapters
    FOR EACH ROW EXECUTE FUNCTION change_log_part('book_part');
CREATE TRIGGER podcast_episodes_change AFTER INSERT OR UPDATE OR DELETE ON podcast_episodes
    FOR EACH ROW EXECUTE FUNCTION change_log_part('episode');
CREATE TRIGGER credits_change AFTER INSERT OR UPDATE OR DELETE ON credits
    FOR EACH ROW EXECUTE FUNCTION change_log_part('credit');

-- ── A listener's own state ───────────────────────────────────────────────

-- TG_ARGV: the state (`favourite`, `rating`, `progress`), then the column of
-- each kind it can be about, as `kind:column` pairs.
CREATE FUNCTION change_log_state() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r    JSONB;
    pair TEXT;
    i    INT;
BEGIN
    IF TG_OP = 'DELETE' THEN r := to_jsonb(OLD); ELSE r := to_jsonb(NEW); END IF;
    FOR i IN 1 .. TG_NARGS - 1 LOOP
        pair := TG_ARGV[i];
        IF r ->> split_part(pair, ':', 2) IS NOT NULL THEN
            PERFORM change_log(TG_ARGV[0] || '.' || split_part(pair, ':', 1),
                               (r ->> split_part(pair, ':', 2))::uuid, NULL, (r ->> 'user_id')::uuid);
        END IF;
    END LOOP;
    RETURN NULL;
END $$;

CREATE TRIGGER music_track_stars_change AFTER INSERT OR UPDATE OR DELETE ON music_track_stars
    FOR EACH ROW EXECUTE FUNCTION change_log_state('favourite', 'track:track_id');
CREATE TRIGGER music_stars_change AFTER INSERT OR UPDATE OR DELETE ON music_stars
    FOR EACH ROW EXECUTE FUNCTION change_log_state('favourite', 'album:album_id', 'contributor:contributor_id');
CREATE TRIGGER audiobook_favorites_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_favorites
    FOR EACH ROW EXECUTE FUNCTION change_log_state('favourite', 'book:book_id');
CREATE TRIGGER music_track_ratings_change AFTER INSERT OR UPDATE OR DELETE ON music_track_ratings
    FOR EACH ROW EXECUTE FUNCTION change_log_state('rating', 'track:track_id');
CREATE TRIGGER music_ratings_change AFTER INSERT OR UPDATE OR DELETE ON music_ratings
    FOR EACH ROW EXECUTE FUNCTION change_log_state('rating', 'album:album_id', 'contributor:contributor_id');
CREATE TRIGGER music_progress_change AFTER INSERT OR UPDATE OR DELETE ON music_progress
    FOR EACH ROW EXECUTE FUNCTION change_log_state('progress', 'track:track_id');
CREATE TRIGGER audiobook_progress_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_progress
    FOR EACH ROW EXECUTE FUNCTION change_log_state('progress', 'book:book_id');
CREATE TRIGGER podcast_progress_change AFTER INSERT OR UPDATE OR DELETE ON podcast_progress
    FOR EACH ROW EXECUTE FUNCTION change_log_state('progress', 'episode:episode_id');
-- A device's last position is part of the item's progress.
CREATE FUNCTION change_log_device_position() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log('progress.' || r.item_kind, r.item_id, NULL, r.user_id);
    RETURN NULL;
END $$;
CREATE TRIGGER progress_devices_change AFTER INSERT OR UPDATE OR DELETE ON progress_devices
    FOR EACH ROW EXECUTE FUNCTION change_log_device_position();
CREATE TRIGGER bookmarks_change AFTER INSERT OR UPDATE OR DELETE ON bookmarks
    FOR EACH ROW EXECUTE FUNCTION change_log_state('bookmark', 'bookmark:id');

-- ── A household move, a relink: one reset ─────────────────────────────────

-- A user moving household relinks their whole library set-based and quietly
-- (music_link_tracks); both households' readers start over from a snapshot.
CREATE OR REPLACE FUNCTION music_relink_member() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE credits c SET contributor_id = music_contributor(NEW.family_id, k.name, k.musicbrainz_artist_id)
      FROM contributors k
     WHERE c.source = 'user' AND k.id = c.contributor_id AND k.family_id <> NEW.family_id
       AND ((c.item_kind = 'track' AND c.item_id IN (SELECT id FROM music_tracks_all WHERE user_id = NEW.user_id))
         OR (c.item_kind = 'book' AND c.item_id IN (SELECT id FROM audiobook_books_all WHERE user_id = NEW.user_id))
         OR (c.item_kind = 'podcast' AND c.item_id IN (SELECT id FROM podcast_feeds WHERE user_id = NEW.user_id)));
    PERFORM music_link_tracks(NEW.user_id);
    UPDATE audiobook_books_all SET author = author WHERE user_id = NEW.user_id;
    UPDATE podcast_feeds SET author = author WHERE user_id = NEW.user_id;
    PERFORM change_log('reset', NEW.family_id, NEW.family_id, NULL);
    IF TG_OP = 'UPDATE' THEN
        PERFORM change_log('reset', OLD.family_id, OLD.family_id, NULL);
    END IF;
    RETURN NULL;
END $$;
