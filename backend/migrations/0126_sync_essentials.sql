-- Sync essentials for /api/v2 (docs/api-v2-completion-plan.md, phase 3): what the
-- change log of 0109 still missed for an offline copy of the library.
--
-- 1. A rename in the own.audio folder (`sync_paths`) is a change of its item.
-- 2. A per-item grant changes what one member sees: that item, for that member.
--    A changed media policy changes a whole kind: that member starts over.
-- 3. Albums, release groups, recordings, genres and contributors are visible
--    through the items that use them, so whatever makes an item appear or go
--    away for someone also re-sends what it refers to.
-- 4. A file's checksum, computed after the upload, reaches the item's record.
-- 5. The household's people as a kind of their own (`member`), former members
--    included, so a client can name every owner it is shown.

-- ── 0. One row per item per household per transaction ────────────────────

-- 0109 kept one row per item per transaction whatever the household, so a
-- member moving from one household to another in one transaction was logged
-- for the household they left only. Each household gets its row now.
DROP INDEX changes_once_idx;
CREATE UNIQUE INDEX changes_once_idx ON changes (xid, kind, item_id,
    COALESCE(user_id, '00000000-0000-0000-0000-000000000000'),
    COALESCE(family_id, '00000000-0000-0000-0000-000000000000'));

-- ── 1. Paths ─────────────────────────────────────────────────────────────

CREATE FUNCTION change_log_path() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log(CASE r.kind WHEN 'music_track' THEN 'track' WHEN 'audiobook' THEN 'book'
                                   WHEN 'podcast_episode' THEN 'episode' END,
                       r.item_id, music_owner_family(r.user_id), NULL)
     WHERE r.kind IN ('music_track', 'audiobook', 'podcast_episode');
    RETURN NULL;
END $$;

CREATE TRIGGER sync_paths_change AFTER INSERT OR UPDATE OR DELETE ON sync_paths
    FOR EACH ROW EXECUTE FUNCTION change_log_path();

-- ── 3. An item and what it refers to ─────────────────────────────────────

-- Log an item and the records seen through it, for a household (`p_family`) or
-- for one member (`p_user`). Readers decide visibility when they read.
CREATE FUNCTION change_log_item(p_kind TEXT, p_item UUID, p_family UUID, p_user UUID) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log(p_kind, p_item, p_family, p_user);
    IF p_kind = 'track' THEN
        PERFORM change_log_track_refs(t.album_id, t.recording_id, p_item, p_family, p_user)
           FROM music_tracks_all t WHERE t.id = p_item;
    ELSIF p_kind = 'book' THEN
        PERFORM change_log('contributor', c.contributor_id, p_family, p_user)
           FROM credits c WHERE c.item_kind = 'book' AND c.item_id = p_item;
    ELSIF p_kind = 'podcast' THEN
        PERFORM change_log('contributor', c.contributor_id, p_family, p_user)
           FROM credits c WHERE c.item_kind = 'podcast' AND c.item_id = p_item;
        PERFORM change_log('episode', e.id, p_family, p_user)
           FROM podcast_episodes e WHERE e.feed_id = p_item;
    END IF;
END $$;

CREATE FUNCTION change_log_track_refs(p_album UUID, p_recording UUID, p_track UUID, p_family UUID, p_user UUID) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('album', p_album, p_family, p_user);
    PERFORM change_log('recording', p_recording, p_family, p_user);
    PERFORM change_log('release_group', a.release_group_id, p_family, p_user) FROM albums a WHERE a.id = p_album;
    PERFORM change_log('genre', g.genre_id, p_family, p_user) FROM track_genres g WHERE g.track_id = p_track;
    PERFORM change_log('contributor', c.contributor_id, p_family, p_user)
       FROM credits c
      WHERE (c.item_kind = 'track' AND c.item_id = p_track) OR (c.item_kind = 'album' AND c.item_id = p_album);
END $$;

-- A track that appears, goes, is shared, unshared, trashed, quarantined, missing
-- from its library folder or moved to another album: its album, recording,
-- release group, genres and contributors too.
CREATE FUNCTION change_log_track_seen() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_family UUID;
BEGIN
    IF TG_OP = 'INSERT' THEN
        -- A new track's own credits and genres log their contributors and genres
        -- as they are written; what it joins is the album and what hangs off it.
        v_family := music_owner_family(NEW.user_id);
        PERFORM change_log('album', NEW.album_id, v_family, NULL);
        PERFORM change_log('recording', NEW.recording_id, v_family, NULL);
        PERFORM change_log('release_group', a.release_group_id, v_family, NULL) FROM albums a WHERE a.id = NEW.album_id;
        PERFORM change_log('contributor', c.contributor_id, v_family, NULL)
           FROM credits c WHERE c.item_kind = 'album' AND c.item_id = NEW.album_id;
    ELSIF TG_OP = 'UPDATE' THEN
        PERFORM change_log_track_refs(NEW.album_id, NEW.recording_id, NEW.id, music_owner_family(NEW.user_id), NULL);
    END IF;
    IF TG_OP IN ('UPDATE', 'DELETE') THEN
        PERFORM change_log_track_refs(OLD.album_id, OLD.recording_id, OLD.id, music_owner_family(OLD.user_id), NULL);
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER music_tracks_seen_change AFTER INSERT OR DELETE ON music_tracks_all
    FOR EACH ROW EXECUTE FUNCTION change_log_track_seen();
CREATE TRIGGER music_tracks_seen_update
    AFTER UPDATE OF user_id, family_id, trashed_at, missing_at, quarantined_at, album_id, recording_id ON music_tracks_all
    FOR EACH ROW WHEN (OLD.user_id IS DISTINCT FROM NEW.user_id OR OLD.family_id IS DISTINCT FROM NEW.family_id
                       OR OLD.trashed_at IS DISTINCT FROM NEW.trashed_at OR OLD.missing_at IS DISTINCT FROM NEW.missing_at
                       OR OLD.quarantined_at IS DISTINCT FROM NEW.quarantined_at
                       OR OLD.album_id IS DISTINCT FROM NEW.album_id OR OLD.recording_id IS DISTINCT FROM NEW.recording_id)
    EXECUTE FUNCTION change_log_track_seen();

CREATE FUNCTION change_log_shelf_seen() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log_item(TG_ARGV[0], r.id, music_owner_family(r.user_id), NULL);
    RETURN NULL;
END $$;

CREATE TRIGGER audiobook_books_seen_change
    AFTER UPDATE OF user_id, family_id, trashed_at, missing_at ON audiobook_books_all
    FOR EACH ROW WHEN (OLD.user_id IS DISTINCT FROM NEW.user_id OR OLD.family_id IS DISTINCT FROM NEW.family_id
                       OR OLD.trashed_at IS DISTINCT FROM NEW.trashed_at OR OLD.missing_at IS DISTINCT FROM NEW.missing_at)
    EXECUTE FUNCTION change_log_shelf_seen('book');
-- Sharing a show shares its episodes: each one is sent again.
CREATE TRIGGER podcast_feeds_seen_change
    AFTER UPDATE OF user_id, family_id ON podcast_feeds
    FOR EACH ROW WHEN (OLD.user_id IS DISTINCT FROM NEW.user_id OR OLD.family_id IS DISTINCT FROM NEW.family_id)
    EXECUTE FUNCTION change_log_shelf_seen('podcast');

-- A credit or a genre on an item can make a contributor or a genre visible.
CREATE OR REPLACE FUNCTION change_log_part() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    CASE TG_ARGV[0]
        WHEN 'track_genre' THEN
            PERFORM change_log('track', r.track_id, music_owner_family(t.user_id), NULL),
                    -- Other parts of a track (its chapters, 0118) use this arm too: only a genre has one.
                    change_log('genre', (to_jsonb(r) ->> 'genre_id')::uuid, music_owner_family(t.user_id), NULL)
               FROM music_tracks_all t WHERE t.id = r.track_id;
        WHEN 'book_part' THEN
            PERFORM change_log('book', r.book_id, music_owner_family(b.user_id), NULL) FROM audiobook_books_all b WHERE b.id = r.book_id;
        WHEN 'episode' THEN
            PERFORM change_log('episode', r.id, music_owner_family(f.user_id), NULL) FROM podcast_feeds f WHERE f.id = r.feed_id;
        WHEN 'credit' THEN
            PERFORM change_log(r.item_kind, r.item_id, k.family_id, NULL),
                    change_log('contributor', r.contributor_id, k.family_id, NULL)
               FROM contributors k WHERE k.id = r.contributor_id;
    END CASE;
    RETURN NULL;
END $$;

-- ── 2. Grants and policies ───────────────────────────────────────────────

CREATE FUNCTION change_log_grant() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    g RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN g := OLD; ELSE g := NEW; END IF;
    PERFORM change_log_item(CASE g.media_kind WHEN 'music' THEN 'track' WHEN 'audiobook' THEN 'book' ELSE 'podcast' END,
                            g.item_id, NULL, g.user_id);
    IF TG_OP = 'UPDATE' AND (OLD.user_id <> NEW.user_id OR OLD.item_id <> NEW.item_id) THEN
        PERFORM change_log_item(CASE OLD.media_kind WHEN 'music' THEN 'track' WHEN 'audiobook' THEN 'book' ELSE 'podcast' END,
                                OLD.item_id, NULL, OLD.user_id);
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER content_grants_change AFTER INSERT OR UPDATE OR DELETE ON content_grants
    FOR EACH ROW EXECUTE FUNCTION change_log_grant();

-- A whole kind shown or hidden: that member takes a new snapshot.
CREATE FUNCTION change_log_policy() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log('reset', r.user_id, NULL, r.user_id);
    RETURN NULL;
END $$;

CREATE TRIGGER member_media_policy_change AFTER INSERT OR UPDATE OR DELETE ON member_media_policy
    FOR EACH ROW EXECUTE FUNCTION change_log_policy();

-- ── 4. Checksums ─────────────────────────────────────────────────────────

-- The checksum job fills `sha256` after the upload; the items that play this
-- file are sent again with it. The indexes make that a lookup, not a scan.
CREATE INDEX music_tracks_audio_object_idx ON music_tracks_all (audio_object_id) WHERE audio_object_id IS NOT NULL;
CREATE INDEX audiobook_files_audio_object_idx ON audiobook_files (audio_object_id) WHERE audio_object_id IS NOT NULL;
CREATE INDEX podcast_episodes_audio_object_idx ON podcast_episodes (audio_object_id) WHERE audio_object_id IS NOT NULL;

CREATE FUNCTION change_log_checksum() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('track', t.id, music_owner_family(t.user_id), NULL)
       FROM music_tracks_all t WHERE t.audio_object_id = NEW.id;
    PERFORM change_log('book', f.book_id, music_owner_family(b.user_id), NULL)
       FROM audiobook_files f JOIN audiobook_books_all b ON b.id = f.book_id WHERE f.audio_object_id = NEW.id;
    PERFORM change_log('episode', e.id, music_owner_family(p.user_id), NULL)
       FROM podcast_episodes e JOIN podcast_feeds p ON p.id = e.feed_id WHERE e.audio_object_id = NEW.id;
    RETURN NULL;
END $$;

CREATE TRIGGER media_objects_checksum_change AFTER UPDATE OF sha256 ON media_objects
    FOR EACH ROW WHEN (OLD.sha256 IS DISTINCT FROM NEW.sha256)
    EXECUTE FUNCTION change_log_checksum();

-- ── 5. Members, and the people who left ──────────────────────────────────

-- Someone who left a household (moved to another, or deleted their account)
-- may still be named on what the household keeps: its edit history, and on a
-- server with several households, anything a client cached. Their name is kept
-- as the household knew it when they left; a deleted account keeps no name.
CREATE TABLE family_former_members (
    family_id    UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    -- No foreign key: the row outlives a deleted account.
    user_id      UUID        NOT NULL,
    display_name TEXT,
    label        TEXT,
    left_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (family_id, user_id)
);
CREATE INDEX family_former_members_user_idx ON family_former_members (user_id);

CREATE FUNCTION family_member_moved() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    IF TG_OP IN ('UPDATE', 'DELETE') AND (TG_OP = 'DELETE' OR OLD.family_id <> NEW.family_id) THEN
        -- A household being deleted takes its rows with it; nothing to keep.
        INSERT INTO family_former_members (family_id, user_id, display_name, label)
        SELECT OLD.family_id, OLD.user_id, u.display_name, OLD.display_label
          FROM families f LEFT JOIN users u ON u.id = OLD.user_id
         WHERE f.id = OLD.family_id
        ON CONFLICT (family_id, user_id) DO UPDATE
           SET display_name = EXCLUDED.display_name, label = EXCLUDED.label, left_at = now();
        -- A deleted account is gone by now: keep no name for it.
        UPDATE family_former_members SET display_name = NULL, label = NULL
         WHERE family_id = OLD.family_id AND user_id = OLD.user_id
           AND NOT EXISTS (SELECT 1 FROM users u WHERE u.id = OLD.user_id);
        PERFORM change_log('member', OLD.user_id, OLD.family_id, NULL);
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') THEN
        DELETE FROM family_former_members WHERE family_id = NEW.family_id AND user_id = NEW.user_id;
        PERFORM change_log('member', NEW.user_id, NEW.family_id, NULL);
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER family_members_change AFTER INSERT OR UPDATE OR DELETE ON family_members
    FOR EACH ROW EXECUTE FUNCTION family_member_moved();

-- A renamed or re-pictured account, in its household.
CREATE FUNCTION change_log_user() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('member', NEW.id, fm.family_id, NULL) FROM family_members fm WHERE fm.user_id = NEW.id;
    RETURN NULL;
END $$;

CREATE TRIGGER users_member_change AFTER UPDATE OF display_name, avatar_object_id ON users
    FOR EACH ROW WHEN (OLD.display_name IS DISTINCT FROM NEW.display_name OR OLD.avatar_object_id IS DISTINCT FROM NEW.avatar_object_id)
    EXECUTE FUNCTION change_log_user();

-- A deleted account leaves no name in the households it left before.
CREATE FUNCTION family_former_member_forget() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    UPDATE family_former_members SET display_name = NULL, label = NULL WHERE user_id = OLD.id;
    RETURN NULL;
END $$;

CREATE TRIGGER users_forget_former AFTER DELETE ON users
    FOR EACH ROW EXECUTE FUNCTION family_former_member_forget();

CREATE FUNCTION change_log_former_member() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log('member', r.user_id, r.family_id, NULL);
    RETURN NULL;
END $$;

CREATE TRIGGER family_former_members_change AFTER UPDATE ON family_former_members
    FOR EACH ROW EXECUTE FUNCTION change_log_former_member();

-- Joining a household: the one who joins starts over from a snapshot of it; the
-- others only when the newcomer brings a library along (0109 reset everyone, so
-- each new account made every device in the household download everything
-- again). Same relinking as 0109 otherwise.
CREATE OR REPLACE FUNCTION music_relink_member() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_library BOOLEAN;
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
    v_library := EXISTS (SELECT 1 FROM music_tracks_all WHERE user_id = NEW.user_id)
              OR EXISTS (SELECT 1 FROM audiobook_books_all WHERE user_id = NEW.user_id)
              OR EXISTS (SELECT 1 FROM podcast_feeds WHERE user_id = NEW.user_id);
    PERFORM change_log('reset', NEW.user_id, NULL, NEW.user_id);
    IF v_library THEN
        PERFORM change_log('reset', NEW.family_id, NEW.family_id, NULL);
        IF TG_OP = 'UPDATE' THEN
            PERFORM change_log('reset', OLD.family_id, OLD.family_id, NULL);
        END IF;
    END IF;
    RETURN NULL;
END $$;

-- Deleting an account that follows a podcast failed: the cascade removed the
-- feed, and the gPodder log (0115) then recorded the unsubscription for an
-- account no longer there. Nobody is left to read it, so nothing is recorded.
CREATE OR REPLACE FUNCTION gpodder_log_subscription() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    device TEXT := NULLIF(current_setting('own_audio.gpodder_device', true), '');
BEGIN
    IF TG_OP IN ('DELETE', 'UPDATE') AND OLD.source_type = 'rss' THEN
        IF TG_OP = 'DELETE' OR OLD.feed_url IS DISTINCT FROM NEW.feed_url OR OLD.user_id IS DISTINCT FROM NEW.user_id THEN
            INSERT INTO gpodder_subscription_changes (user_id, feed_url, added, device_id)
            SELECT OLD.user_id, OLD.feed_url, false, device WHERE EXISTS (SELECT 1 FROM users u WHERE u.id = OLD.user_id);
        END IF;
    END IF;
    IF TG_OP IN ('INSERT', 'UPDATE') AND NEW.source_type = 'rss' THEN
        IF TG_OP = 'INSERT' OR OLD.feed_url IS DISTINCT FROM NEW.feed_url OR OLD.user_id IS DISTINCT FROM NEW.user_id THEN
            INSERT INTO gpodder_subscription_changes (user_id, feed_url, added, device_id) VALUES (NEW.user_id, NEW.feed_url, true, device);
        END IF;
    END IF;
    RETURN NULL;
END $$;

-- ── Continue listening ───────────────────────────────────────────────────

-- `GET /me/progress` walks a listener's progress newest first, across kinds.
-- Without these a listener with 15,000 unfinished songs read the whole
-- catalog's visibility to find the newest 25 (33 ms on 600,000 tracks; 0.7 ms
-- with them). Books are a thousand at most and stay on their primary key.
CREATE INDEX music_progress_recent_idx ON music_progress (user_id, updated_at DESC, track_id DESC);
CREATE INDEX podcast_progress_recent_idx ON podcast_progress (user_id, updated_at DESC, episode_id DESC);
