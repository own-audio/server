-- Playlists, smart playlists and track feedback in the change feed
-- (docs/api-v2-completion-plan.md, phase 6). Playlists and smart playlists become kinds of
-- their own (`playlist`, `smart_playlist`); a listener's dislikes and bans are their state,
-- `feedback.track`, like favourites.
--
-- 1. A playlist: its own row, the trash included (a deleted, restored or purged playlist is
--    an update or a delete of the row).
-- 2. Its entries: the playlist (its record counts them and carries their length and cover).
-- 3. A track that appears or goes for someone (shared, unshared, trashed, quarantined,
--    missing) or changes its length or cover: the playlists that hold it, whose records count
--    only the tracks a reader can see.
-- 4. A grant on a track or a playlist (0126's `change_log_item`): the playlists it shows.
-- 5. A smart playlist: its own row.
-- 6. Track feedback: `feedback.track` for its listener.

-- ── 1. Playlists ─────────────────────────────────────────────────────────

CREATE TRIGGER music_playlists_change AFTER INSERT OR UPDATE OR DELETE ON music_playlists_all
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('playlist');

-- ── 2. Entries ───────────────────────────────────────────────────────────

CREATE FUNCTION change_log_playlist_entry() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    -- A playlist being purged is gone by now; its own row says so.
    PERFORM change_log('playlist', p.id, music_owner_family(p.user_id), NULL)
       FROM music_playlists_all p WHERE p.id = r.playlist_id;
    IF TG_OP = 'UPDATE' AND OLD.playlist_id <> NEW.playlist_id THEN
        PERFORM change_log('playlist', p.id, music_owner_family(p.user_id), NULL)
           FROM music_playlists_all p WHERE p.id = OLD.playlist_id;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER music_playlist_tracks_change AFTER INSERT OR UPDATE OR DELETE ON music_playlist_tracks
    FOR EACH ROW EXECUTE FUNCTION change_log_playlist_entry();

-- From a track to the playlists that hold it without a scan: the trigger below, and the
-- cascade when a track is purged (it scanned every entry before).
CREATE INDEX music_playlist_tracks_track_idx ON music_playlist_tracks (track_id);

-- ── 3. A track seen differently ──────────────────────────────────────────

CREATE FUNCTION change_log_track_playlists() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('playlist', p.id, music_owner_family(p.user_id), NULL)
       FROM music_playlist_tracks e JOIN music_playlists_all p ON p.id = e.playlist_id
      WHERE e.track_id = NEW.id;
    RETURN NULL;
END $$;

CREATE TRIGGER music_tracks_playlists_change
    AFTER UPDATE OF user_id, family_id, trashed_at, missing_at, quarantined_at, duration_secs, cover_object_id ON music_tracks_all
    FOR EACH ROW WHEN (OLD.user_id IS DISTINCT FROM NEW.user_id OR OLD.family_id IS DISTINCT FROM NEW.family_id
                       OR OLD.trashed_at IS DISTINCT FROM NEW.trashed_at OR OLD.missing_at IS DISTINCT FROM NEW.missing_at
                       OR OLD.quarantined_at IS DISTINCT FROM NEW.quarantined_at
                       OR OLD.duration_secs IS DISTINCT FROM NEW.duration_secs
                       OR OLD.cover_object_id IS DISTINCT FROM NEW.cover_object_id)
    EXECUTE FUNCTION change_log_track_playlists();

-- ── 4. What a grant makes visible ────────────────────────────────────────

-- 0127's function, with playlists: a member granted a track sees it in the playlists that
-- hold it, and a playlist granted to them (media kind `music`, as v1 grants it) is theirs to
-- read. Readers decide visibility when they read.
CREATE OR REPLACE FUNCTION change_log_item(p_kind TEXT, p_item UUID, p_family UUID, p_user UUID) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log(p_kind, p_item, p_family, p_user);
    IF p_kind = 'track' THEN
        PERFORM change_log_track_refs(t.album_id, t.recording_id, p_item, p_family, p_user)
           FROM music_tracks_all t WHERE t.id = p_item;
        PERFORM change_log('playlist', e.playlist_id, p_family, p_user)
           FROM music_playlist_tracks e WHERE e.track_id = p_item;
        PERFORM change_log('playlist', p.id, p_family, p_user)
           FROM music_playlists_all p WHERE p.id = p_item;
    ELSIF p_kind = 'book' THEN
        PERFORM change_log('contributor', c.contributor_id, p_family, p_user)
           FROM credits c WHERE c.item_kind = 'book' AND c.item_id = p_item;
        PERFORM change_log('series', sb.series_id, p_family, p_user)
           FROM audiobook_series_books sb WHERE sb.book_id = p_item;
        PERFORM change_log('collection', cb.collection_id, p_family, p_user)
           FROM audiobook_collection_books cb WHERE cb.book_id = p_item;
    ELSIF p_kind = 'podcast' THEN
        PERFORM change_log('contributor', c.contributor_id, p_family, p_user)
           FROM credits c WHERE c.item_kind = 'podcast' AND c.item_id = p_item;
        PERFORM change_log('episode', e.id, p_family, p_user)
           FROM podcast_episodes e WHERE e.feed_id = p_item;
    END IF;
END $$;

-- ── 5. Smart playlists ───────────────────────────────────────────────────

CREATE TRIGGER music_smart_playlists_change AFTER INSERT OR UPDATE OR DELETE ON music_smart_playlists
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('smart_playlist');

-- ── 6. Track feedback ────────────────────────────────────────────────────

CREATE TRIGGER music_track_feedback_change AFTER INSERT OR UPDATE OR DELETE ON music_track_feedback
    FOR EACH ROW EXECUTE FUNCTION change_log_state('feedback', 'track:track_id');
