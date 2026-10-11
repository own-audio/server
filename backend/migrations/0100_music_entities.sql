-- Music entities (docs/data-model-plan.md, E1): contributors, credits,
-- release groups, albums, recordings and genres, following MusicBrainz.
-- Until now artists and albums were GROUP BYs over tag strings; from here
-- they are rows a household owns, that stars, ratings, images and API ids
-- hang off, and that survive a retag.
--
-- Entities belong to the owner's household (family_members), not to the
-- track's family_id — that column means "shared", and a private track still
-- belongs to its household's album and artists. Who may see an entity is
-- derived from the items credited to it; nothing about visibility is stored
-- here.
--
-- Triggers keep them in step with the tags (decision 2 of the plan): every
-- path that writes a track — upload, scan, identify, edits, the hosted
-- edition — is covered without remembering to call anything.

-- ── Tables ───────────────────────────────────────────────────────────────

CREATE TABLE contributors (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id             UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    name                  TEXT        NOT NULL,
    sort_name             TEXT,
    kind                  TEXT        NOT NULL DEFAULT 'unknown'
                          CHECK (kind IN ('unknown', 'person', 'group', 'orchestra', 'choir', 'organisation')),
    musicbrainz_artist_id TEXT,
    image_object_id       UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    biography             TEXT,
    -- Set when someone corrected the entity by hand; matching never overwrites it.
    edited_at             TIMESTAMPTZ,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX contributors_family_name_idx ON contributors (family_id, lower(name));
CREATE UNIQUE INDEX contributors_family_mbid_uq ON contributors (family_id, musicbrainz_artist_id)
    WHERE musicbrainz_artist_id IS NOT NULL;

-- How tags find a contributor: 'name:<lower(trim(name))>' or 'mb:<artist id>'.
-- Kept apart from the display name so a corrected name still matches the
-- next scan, and a merge can point several keys at one contributor. A split
-- points one key at several, in order ("A & B" → A, B; 0104).
CREATE TABLE contributor_keys (
    family_id      UUID     NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    key            TEXT     NOT NULL,
    position       SMALLINT NOT NULL DEFAULT 0,
    contributor_id UUID     NOT NULL REFERENCES contributors (id) ON DELETE CASCADE,
    PRIMARY KEY (family_id, key, position),
    UNIQUE (family_id, key, contributor_id)
);
CREATE INDEX contributor_keys_contributor_idx ON contributor_keys (contributor_id);

-- Who did what on an item: MusicBrainz's artist credit, also serving books
-- (ONIX contributor roles) and podcasts (<podcast:person>). Polymorphic on
-- (item_kind, item_id); triggers remove a deleted item's credits.
CREATE TABLE credits (
    id             UUID     PRIMARY KEY DEFAULT gen_random_uuid(),
    contributor_id UUID     NOT NULL REFERENCES contributors (id) ON DELETE CASCADE,
    item_kind      TEXT     NOT NULL CHECK (item_kind IN
                   ('track', 'album', 'release_group', 'recording', 'book', 'podcast', 'episode')),
    item_id        UUID     NOT NULL,
    role           TEXT     NOT NULL,
    position       SMALLINT NOT NULL DEFAULT 0,
    -- The name as printed on this item, when it differs from the contributor's.
    credited_as    TEXT,
    -- What follows this name in the full credit: ' feat. ', ' & ', ', '.
    join_phrase    TEXT     NOT NULL DEFAULT '',
    -- 'tag' rows are rewritten from the file on every relink; 'user' rows are corrections.
    source         TEXT     NOT NULL DEFAULT 'tag' CHECK (source IN ('tag', 'user')),
    UNIQUE (item_kind, item_id, role, position)
);
CREATE INDEX credits_item_idx ON credits (item_kind, item_id);
CREATE INDEX credits_contributor_idx ON credits (contributor_id, item_kind);

CREATE TABLE release_groups (
    id                           UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id                    UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    title                        TEXT        NOT NULL,
    kind                         TEXT        NOT NULL DEFAULT 'album',
    first_release_date           TEXT,
    musicbrainz_release_group_id TEXT,
    -- 'mb:<release group id>', or 'album:<album id>' for one made per album.
    match_key                    TEXT        NOT NULL,
    edited_at                    TIMESTAMPTZ,
    created_at                   TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (family_id, match_key)
);

CREATE TABLE albums (
    id                     UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id              UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    release_group_id       UUID        REFERENCES release_groups (id) ON DELETE SET NULL,
    title                  TEXT        NOT NULL,
    -- The album artist as one line, for lists; the credits are the structure.
    artist_display         TEXT        NOT NULL,
    release_date           TEXT,
    country                TEXT,
    label                  TEXT,
    barcode                TEXT,
    is_compilation         BOOLEAN     NOT NULL DEFAULT FALSE,
    musicbrainz_release_id TEXT,
    edited_at              TIMESTAMPTZ,
    -- Set when someone put the album in a release group by hand; tags no longer move it.
    release_group_set_at   TIMESTAMPTZ,
    created_at             TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at             TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX albums_family_title_idx ON albums (family_id, lower(title));
CREATE INDEX albums_release_group_idx ON albums (release_group_id);

-- How tags find an album: 'mb:<release id>', or 'tag:<album artists' contributor
-- ids, ';'>␟<lower album title>' — by id, so a corrected artist keeps its
-- albums. Several keys lead to one album once albums are merged, so the merge
-- holds at the next scan.
CREATE TABLE album_keys (
    family_id UUID NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    key       TEXT NOT NULL,
    album_id  UUID NOT NULL REFERENCES albums (id) ON DELETE CASCADE,
    PRIMARY KEY (family_id, key)
);
CREATE INDEX album_keys_album_idx ON album_keys (album_id);

-- The performance; several tracks on several albums can be one recording.
-- Only made from a MusicBrainz recording id: two songs sharing a title are
-- not one recording.
CREATE TABLE recordings (
    id                       UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id                UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    title                    TEXT        NOT NULL,
    musicbrainz_recording_id TEXT        NOT NULL,
    isrc                     TEXT,
    created_at               TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (family_id, musicbrainz_recording_id)
);

CREATE TABLE genres (
    id        UUID NOT NULL PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id UUID NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    name      TEXT NOT NULL,
    key       TEXT NOT NULL,
    UNIQUE (family_id, key)
);

CREATE TABLE track_genres (
    track_id UUID     NOT NULL REFERENCES music_tracks_all (id) ON DELETE CASCADE,
    genre_id UUID     NOT NULL REFERENCES genres (id) ON DELETE CASCADE,
    position SMALLINT NOT NULL DEFAULT 0,
    PRIMARY KEY (track_id, genre_id)
);
CREATE INDEX track_genres_genre_idx ON track_genres (genre_id);

ALTER TABLE music_tracks_all
    ADD COLUMN album_id     UUID REFERENCES albums (id) ON DELETE SET NULL,
    ADD COLUMN recording_id UUID REFERENCES recordings (id) ON DELETE SET NULL;
-- Not partial: the foreign key's ON DELETE SET NULL looks tracks up by album_id
-- when an album is removed, and only a full index serves that lookup.
CREATE INDEX music_tracks_album_idx ON music_tracks_all (album_id);
CREATE INDEX music_tracks_recording_idx ON music_tracks_all (recording_id) WHERE recording_id IS NOT NULL;

-- The view is `SELECT *`, fixed at creation: recreate it so it carries the new columns.
CREATE OR REPLACE VIEW music_tracks AS
    SELECT * FROM music_tracks_all WHERE trashed_at IS NULL AND missing_at IS NULL;

-- ── Tag parsing (the SQL twins of the 0089 generated keys) ───────────────

-- The guest markers the generated keys strip: " feat. ", " ft. ", " featuring ",
-- optionally opened by a bracket.
CREATE FUNCTION music_primary_artist(p_artist TEXT) RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(
        NULLIF(regexp_replace(trim(p_artist), '\s+[(\[]?(feat\.?|ft\.?|featuring)\s.*$', '', 'i'), ''),
        'Unknown Artist')
$$;

CREATE FUNCTION music_album_artist(p_album_artist TEXT, p_artist TEXT) RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(NULLIF(trim(p_album_artist), ''), music_primary_artist(p_artist))
$$;

CREATE FUNCTION music_album_title(p_album TEXT) RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT COALESCE(NULLIF(trim(p_album), ''), 'Unknown Album')
$$;

-- The guests after a marker, closing bracket dropped: "A feat. B; C)" → "B; C".
CREATE FUNCTION music_guest_artists(p_artist TEXT) RETURNS TEXT
LANGUAGE sql IMMUTABLE AS $$
    SELECT NULLIF(trim(regexp_replace(
        substring(trim(p_artist) FROM '(?i)\s[(\[]?(?:feat\.?|ft\.?|featuring)\s+(.*)$'),
        '[)\]]\s*$', '')), '')
$$;

-- ── Matching ─────────────────────────────────────────────────────────────

-- The household's contributor for a name (and MusicBrainz id, when the tag has
-- one); created on first sight. A known id wins over the name, and the first
-- time an id arrives for a name already seen, the two are joined.
CREATE FUNCTION music_contributor(p_family UUID, p_name TEXT, p_mbid TEXT) RETURNS UUID
LANGUAGE plpgsql AS $$
DECLARE
    v_name  TEXT := trim(p_name);
    v_mbid  TEXT := NULLIF(trim(p_mbid), '');
    v_nkey  TEXT := 'name:' || lower(trim(p_name));
    v_id    UUID;
BEGIN
    IF v_mbid IS NOT NULL THEN
        SELECT contributor_id INTO v_id FROM contributor_keys WHERE family_id = p_family AND key = 'mb:' || v_mbid;
        IF v_id IS NOT NULL THEN
            INSERT INTO contributor_keys (family_id, key, contributor_id) VALUES (p_family, v_nkey, v_id)
                ON CONFLICT DO NOTHING;
            RETURN v_id;
        END IF;
    END IF;
    SELECT contributor_id INTO v_id FROM contributor_keys WHERE family_id = p_family AND key = v_nkey
     ORDER BY position LIMIT 1;
    IF v_id IS NULL THEN
        INSERT INTO contributors (family_id, name) VALUES (p_family, v_name) RETURNING id INTO v_id;
        INSERT INTO contributor_keys (family_id, key, contributor_id) VALUES (p_family, v_nkey, v_id)
            ON CONFLICT DO NOTHING;
        IF NOT FOUND THEN
            -- Another transaction made it first.
            DELETE FROM contributors WHERE id = v_id;
            SELECT contributor_id INTO v_id FROM contributor_keys WHERE family_id = p_family AND key = v_nkey
             ORDER BY position LIMIT 1;
        END IF;
    END IF;
    IF v_mbid IS NOT NULL THEN
        -- Join the id to the name's contributor, unless that one already has another id.
        UPDATE contributors SET musicbrainz_artist_id = v_mbid, updated_at = now()
         WHERE id = v_id AND musicbrainz_artist_id IS NULL
           AND NOT EXISTS (SELECT 1 FROM contributors c WHERE c.family_id = p_family AND c.musicbrainz_artist_id = v_mbid);
        INSERT INTO contributor_keys (family_id, key, contributor_id) VALUES (p_family, 'mb:' || v_mbid, v_id)
            ON CONFLICT DO NOTHING;
    END IF;
    RETURN v_id;
END $$;

-- Every contributor a credited name stands for: the one music_contributor
-- finds, or the parts of a split name in order.
CREATE FUNCTION music_contributors(p_family UUID, p_name TEXT, p_mbid TEXT)
RETURNS TABLE (contributor_id UUID, part INT)
LANGUAGE plpgsql AS $$
#variable_conflict use_column
BEGIN
    RETURN QUERY
    SELECT k.contributor_id, (row_number() OVER (ORDER BY k.position))::int
      FROM contributor_keys k
     WHERE k.family_id = p_family AND k.key = 'name:' || lower(trim(p_name))
       AND EXISTS (SELECT 1 FROM contributor_keys o
                    WHERE o.family_id = k.family_id AND o.key = k.key AND o.position > 0);
    IF NOT FOUND THEN
        RETURN QUERY SELECT music_contributor(p_family, p_name, p_mbid), 1;
    END IF;
END $$;

-- The household of a track's owner; NULL for a user in none (then nothing is linked).
CREATE FUNCTION music_owner_family(p_user UUID) RETURNS UUID
LANGUAGE sql STABLE AS $$
    SELECT family_id FROM family_members WHERE user_id = p_user
$$;

-- Whether someone corrected this role on the item by hand (credits with
-- source 'user'): then the tags no longer write it (docs/api-v2-plan.md, V3c).
CREATE FUNCTION music_role_corrected(p_kind TEXT, p_item UUID, p_role TEXT) RETURNS BOOLEAN
LANGUAGE sql STABLE AS $$
    SELECT EXISTS (SELECT 1 FROM credits WHERE item_kind = p_kind AND item_id = p_item AND role = p_role AND source = 'user')
$$;

-- Join phrases from the final list, per role: '; ' between names in one role,
-- ' feat. ' after a track's last artist when guests follow, nothing after
-- the last name of a role. Computed after duplicates are dropped, so no
-- credit ends in a dangling separator.
CREATE FUNCTION music_credit_joins(p_kind TEXT, p_item UUID) RETURNS void
LANGUAGE sql AS $$
    UPDATE credits c SET join_phrase = j.phrase
      FROM (SELECT id, CASE WHEN lead(role) OVER w IS NOT NULL THEN '; '
                            WHEN role = 'artist' AND bool_or(role = 'featured') OVER () THEN ' feat. '
                            ELSE '' END AS phrase
              FROM credits WHERE item_kind = p_kind AND item_id = p_item
            WINDOW w AS (PARTITION BY role ORDER BY position)) j
     WHERE c.id = j.id AND c.join_phrase IS DISTINCT FROM j.phrase
$$;

-- BEFORE INSERT/UPDATE: the track's album (and its release group) and recording.
CREATE FUNCTION music_link_track_before() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_family    UUID := music_owner_family(NEW.user_id);
    v_aa        TEXT;
    v_title     TEXT;
    v_key       TEXT;
    v_album     UUID;
    v_rg        UUID;
    v_rg_key    TEXT;
    v_rg_set    BOOLEAN;
    v_aa_ids    UUID[];
    v_mb_rel    TEXT := NULLIF(trim(NEW.musicbrainz_release_id), '');
    v_mb_rg     TEXT := NULLIF(trim(NEW.musicbrainz_release_group_id), '');
    v_mb_rec    TEXT := NULLIF(trim(NEW.musicbrainz_recording_id), '');
BEGIN
    IF current_setting('own_audio.bulk_link', true) = 'on' THEN
        RETURN NEW;
    END IF;
    IF v_family IS NULL THEN
        NEW.album_id := NULL;
        NEW.recording_id := NULL;
        RETURN NEW;
    END IF;

    v_aa := music_album_artist(NEW.album_artist, NEW.artist);
    v_title := music_album_title(NEW.album);
    -- The album artists as contributors, in order, each once: the album's
    -- credits and, without a MusicBrainz release id, its key — by id, so an
    -- artist renamed, merged or split keeps its albums (0104).
    SELECT array_agg(d.contributor_id ORDER BY d.ord) INTO v_aa_ids
      FROM (SELECT DISTINCT ON (c.contributor_id) c.contributor_id, p.ord * 100 + c.part AS ord
              FROM (SELECT trim(x) AS name, ord
                      FROM unnest(string_to_array(v_aa, ';')) WITH ORDINALITY AS u(x, ord) WHERE trim(x) <> '') p
             CROSS JOIN LATERAL music_contributors(v_family, p.name,
                   CASE WHEN p.ord = 1 AND NULLIF(trim(NEW.album_artist), '') IS NULL THEN NEW.musicbrainz_artist_id END) c
             ORDER BY c.contributor_id, p.ord, c.part) d;
    v_key := CASE WHEN v_mb_rel IS NOT NULL THEN 'mb:' || v_mb_rel
                  ELSE 'tag:' || array_to_string(v_aa_ids, ';') || chr(31) || lower(v_title) END;
    SELECT album_id INTO v_album FROM album_keys WHERE family_id = v_family AND key = v_key;
    IF v_album IS NULL THEN
        INSERT INTO albums (family_id, title, artist_display, is_compilation, musicbrainz_release_id)
        VALUES (v_family, v_title, v_aa, COALESCE(NEW.is_compilation, FALSE), v_mb_rel)
        RETURNING id INTO v_album;
        INSERT INTO album_keys (family_id, key, album_id) VALUES (v_family, v_key, v_album)
            ON CONFLICT DO NOTHING;
        IF NOT FOUND THEN
            -- Another transaction made it first.
            DELETE FROM albums WHERE id = v_album;
            SELECT album_id INTO v_album FROM album_keys WHERE family_id = v_family AND key = v_key;
        ELSE
            -- The album artist is the album's credit (split on ';' like a track's),
            -- set once; corrections stay.
            INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
            SELECT u.id, 'album', v_album, 'artist', (u.ord - 1)::smallint
              FROM unnest(v_aa_ids) WITH ORDINALITY AS u(id, ord)
            ON CONFLICT DO NOTHING;
            PERFORM music_credit_joins('album', v_album);
            -- The line from the credits, so a corrected (renamed, split) artist
            -- reads the same on a new album as on the old ones.
            UPDATE albums a SET artist_display = l.line
              FROM (SELECT string_agg(k.name || c.join_phrase, '' ORDER BY c.position) AS line
                      FROM credits c JOIN contributors k ON k.id = c.contributor_id
                     WHERE c.item_kind = 'album' AND c.item_id = v_album AND c.role = 'artist') l
             WHERE a.id = v_album AND l.line IS NOT NULL AND a.artist_display IS DISTINCT FROM l.line;
        END IF;
    END IF;

    -- Release group: by its MusicBrainz id, or one per album; not when set by hand.
    v_rg_key := CASE WHEN v_mb_rg IS NOT NULL THEN 'mb:' || v_mb_rg ELSE 'album:' || v_album END;
    SELECT release_group_id, release_group_set_at IS NOT NULL INTO v_rg, v_rg_set FROM albums WHERE id = v_album;
    IF NOT v_rg_set AND (v_rg IS NULL OR v_mb_rg IS NOT NULL) THEN
        INSERT INTO release_groups (family_id, title, musicbrainz_release_group_id, match_key)
        VALUES (v_family, v_title, v_mb_rg, v_rg_key)
        ON CONFLICT (family_id, match_key) DO UPDATE SET match_key = EXCLUDED.match_key
        RETURNING id INTO v_rg;
        UPDATE albums SET release_group_id = v_rg WHERE id = v_album AND release_group_id IS DISTINCT FROM v_rg;
    END IF;
    NEW.album_id := v_album;

    IF v_mb_rec IS NOT NULL THEN
        INSERT INTO recordings (family_id, title, musicbrainz_recording_id)
        VALUES (v_family, NEW.title, v_mb_rec)
        ON CONFLICT (family_id, musicbrainz_recording_id) DO UPDATE SET musicbrainz_recording_id = EXCLUDED.musicbrainz_recording_id
        RETURNING id INTO NEW.recording_id;
    ELSE
        NEW.recording_id := NULL;
    END IF;
    RETURN NEW;
END $$;

-- AFTER INSERT/UPDATE: the track's credits and genres, rewritten from the tags.
-- Corrections (source = 'user') are left alone.
CREATE FUNCTION music_link_track_after() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_family  UUID := music_owner_family(NEW.user_id);
    v_primary TEXT;
    v_guests  TEXT;
    v_name    TEXT;
    v_pos     SMALLINT := 0;
    v_n       INT;
    v_i       INT;
    v_parts   TEXT[];
    v_genre   TEXT;
    v_gid     UUID;
    v_cid     UUID;
BEGIN
    IF current_setting('own_audio.bulk_link', true) = 'on' THEN
        RETURN NULL;
    END IF;
    DELETE FROM credits WHERE item_kind = 'track' AND item_id = NEW.id AND source = 'tag';
    DELETE FROM track_genres WHERE track_id = NEW.id;
    IF v_family IS NULL THEN
        RETURN NULL;
    END IF;

    -- The track's own artists: split on ';' only — '&' and ',' are part of names.
    v_primary := music_primary_artist(NEW.artist);
    v_parts := array_remove(array(SELECT NULLIF(trim(x), '') FROM unnest(string_to_array(v_primary, ';')) x), NULL);
    v_n := COALESCE(array_length(v_parts, 1), 0);
    v_guests := music_guest_artists(NEW.artist);
    -- A role someone corrected by hand is theirs: the tags no longer write it.
    IF music_role_corrected('track', NEW.id, 'artist') THEN
        v_n := 0;
    END IF;
    FOR v_i IN 1 .. v_n LOOP
        v_name := v_parts[v_i];
        FOR v_cid IN SELECT c.contributor_id FROM music_contributors(v_family, v_name,
                         CASE WHEN v_i = 1 THEN NEW.musicbrainz_artist_id END) c ORDER BY c.part LOOP
            CONTINUE WHEN EXISTS (SELECT 1 FROM credits WHERE item_kind = 'track' AND item_id = NEW.id
                                      AND role = 'artist' AND contributor_id = v_cid);
            INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
            VALUES (v_cid, 'track', NEW.id, 'artist', v_pos)
            ON CONFLICT DO NOTHING;
            v_pos := v_pos + 1;
        END LOOP;
    END LOOP;

    IF v_guests IS NOT NULL AND NOT music_role_corrected('track', NEW.id, 'featured') THEN
        v_parts := array_remove(array(SELECT NULLIF(trim(x), '') FROM unnest(string_to_array(v_guests, ';')) x), NULL);
        v_n := COALESCE(array_length(v_parts, 1), 0);
        FOR v_i IN 1 .. v_n LOOP
            FOR v_cid IN SELECT c.contributor_id FROM music_contributors(v_family, v_parts[v_i], NULL) c
                          ORDER BY c.part LOOP
                CONTINUE WHEN EXISTS (SELECT 1 FROM credits WHERE item_kind = 'track' AND item_id = NEW.id
                                          AND role = 'featured' AND contributor_id = v_cid);
                INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
                VALUES (v_cid, 'track', NEW.id, 'featured', v_pos)
                ON CONFLICT DO NOTHING;
                v_pos := v_pos + 1;
            END LOOP;
        END LOOP;
    END IF;

    PERFORM music_credit_joins('track', NEW.id);

    -- Genres: several per track, separated by ';' or ','.
    v_pos := 0;
    FOR v_genre IN
        SELECT name FROM (
            SELECT DISTINCT ON (lower(trim(g))) trim(g) AS name, o
              FROM unnest(regexp_split_to_array(COALESCE(NEW.genre, ''), '\s*[;,]\s*')) WITH ORDINALITY AS u(g, o)
             WHERE trim(g) <> ''
             ORDER BY lower(trim(g)), o
        ) first_seen ORDER BY o
    LOOP
        INSERT INTO genres (family_id, name, key) VALUES (v_family, v_genre, lower(v_genre))
            ON CONFLICT (family_id, key) DO UPDATE SET key = EXCLUDED.key
            RETURNING id INTO v_gid;
        INSERT INTO track_genres (track_id, genre_id, position) VALUES (NEW.id, v_gid, v_pos) ON CONFLICT DO NOTHING;
        v_pos := v_pos + 1;
    END LOOP;
    RETURN NULL;
END $$;

-- A deleted track takes its credits with it (track_genres cascades).
CREATE FUNCTION music_track_unlink() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM credits WHERE item_kind = 'track' AND item_id = OLD.id;
    RETURN NULL;
END $$;
CREATE TRIGGER music_tracks_unlink AFTER DELETE ON music_tracks_all
    FOR EACH ROW EXECUTE FUNCTION music_track_unlink();

-- A user moving household: their tracks join the new household's entities,
-- set-based (music_link_tracks), not through the row triggers.
CREATE FUNCTION music_relink_member() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM music_link_tracks(NEW.user_id);
    RETURN NULL;
END $$;
CREATE TRIGGER family_members_relink_music
    AFTER INSERT OR UPDATE OF family_id ON family_members
    FOR EACH ROW EXECUTE FUNCTION music_relink_member();

-- ── Linking many tracks at once ──────────────────────────────────────────
--
-- The same rules as the row triggers, set-based: the migration links every
-- track with it, and a user moving household relinks their whole library
-- with it. Running the row triggers instead took more than ten minutes for
-- 600,000 tracks; this takes about one. It reuses what exists (contributors,
-- albums, keys) and only adds what is missing. While it runs, the row
-- triggers stand aside (own_audio.bulk_link).

CREATE FUNCTION music_link_tracks(p_user UUID) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM set_config('own_audio.bulk_link', 'on', true);
    -- The change log (0109) takes a relink of many tracks as one reset, not row by row.
    PERFORM set_config('own_audio.quiet', 'on', true);
    DROP TABLE IF EXISTS link_src, link_names;

    CREATE TEMP TABLE link_src ON COMMIT DROP AS
    SELECT t.id, fm.family_id AS family,
           music_album_artist(t.album_artist, t.artist) AS aa,
           music_album_title(t.album) AS title,
           music_primary_artist(t.artist) AS primary_artist,
           music_guest_artists(t.artist) AS guests,
           NULLIF(trim(t.musicbrainz_artist_id), '') AS mb_artist,
           NULLIF(trim(t.musicbrainz_release_id), '') AS mb_rel,
           NULLIF(trim(t.musicbrainz_release_group_id), '') AS mb_rg,
           NULLIF(trim(t.musicbrainz_recording_id), '') AS mb_rec,
           t.is_compilation, t.title AS track_title, t.genre, t.created_at,
           NULL::TEXT AS album_key
      FROM music_tracks_all t
      JOIN family_members fm ON fm.user_id = t.user_id
     WHERE p_user IS NULL OR t.user_id = p_user;
    -- Keyed by release id here; by album artists and title below, once the
    -- artists are contributors.
    UPDATE link_src SET album_key = 'mb:' || mb_rel WHERE mb_rel IS NOT NULL;
    -- Temp tables have no statistics until analysed; without them the planner
    -- joins 600,000 rows by nested loop (the first try ran for an hour).
    ANALYZE link_src;

    -- What the tags said before is rewritten; corrections (source 'user') stay.
    -- Joins, not `IN (subquery)`: a subquery of 600,000 ids does not fit in
    -- work_mem, and PostgreSQL then re-scans it for every row (quadratic).
    DELETE FROM credits c USING music_tracks_all t
     WHERE c.item_kind = 'track' AND c.source = 'tag' AND c.item_id = t.id
       AND (p_user IS NULL OR t.user_id = p_user);
    DELETE FROM track_genres tg USING music_tracks_all t
     WHERE tg.track_id = t.id AND (p_user IS NULL OR t.user_id = p_user);

    -- Every credited name: the track's own artists (split on ';'), its guests.
    CREATE TEMP TABLE link_names ON COMMIT DROP AS
    SELECT s.id, s.family, trim(p.name) AS name, 'artist' AS role, (p.ord - 1)::int AS ord,
           p.ord = 1 AS first_artist, s.guests
      FROM link_src s, unnest(string_to_array(s.primary_artist, ';')) WITH ORDINALITY AS p(name, ord)
     WHERE trim(p.name) <> ''
    UNION ALL
    SELECT s.id, s.family, trim(g.name), 'featured', (1000 + g.ord)::int, FALSE, s.guests
      FROM link_src s, unnest(string_to_array(s.guests, ';')) WITH ORDINALITY AS g(name, ord)
     WHERE s.guests IS NOT NULL AND trim(g.name) <> '';
    ANALYZE link_names;

    -- New contributors: names (and album artists) no key knows yet.
    WITH wanted AS (
        SELECT DISTINCT ON (family, lower(name)) family, name
          FROM (SELECT family, name FROM link_names
                UNION ALL
                SELECT family, trim(x) FROM link_src, unnest(string_to_array(aa, ';')) x WHERE trim(x) <> '') n
         WHERE NOT EXISTS (SELECT 1 FROM contributor_keys k WHERE k.family_id = n.family AND k.key = 'name:' || lower(n.name))
         ORDER BY family, lower(name), name
    ), made AS (
        INSERT INTO contributors (family_id, name) SELECT family, name FROM wanted RETURNING id, family_id, name
    )
    INSERT INTO contributor_keys (family_id, key, contributor_id)
    SELECT family_id, 'name:' || lower(trim(name)), id FROM made
    ON CONFLICT DO NOTHING;

    -- A MusicBrainz artist id on a track belongs to its first artist: given to a
    -- contributor that has none yet, when no other contributor of the household holds it.
    WITH ids AS (
        SELECT DISTINCT ON (k.contributor_id) k.contributor_id, n.family, s.mb_artist AS mb
          FROM link_names n JOIN link_src s ON s.id = n.id
          JOIN contributor_keys k ON k.family_id = n.family AND k.key = 'name:' || lower(n.name)
         WHERE n.first_artist AND s.mb_artist IS NOT NULL
           -- not the parts of a split name: the id was the combined credit's
           AND NOT EXISTS (SELECT 1 FROM contributor_keys o
                            WHERE o.family_id = k.family_id AND o.key = k.key AND o.position > 0)
         ORDER BY k.contributor_id, s.mb_artist
    ), unique_ids AS (
        SELECT DISTINCT ON (family, mb) contributor_id, family, mb FROM ids ORDER BY family, mb, contributor_id
    )
    UPDATE contributors c SET musicbrainz_artist_id = u.mb, updated_at = now()
      FROM unique_ids u
     WHERE c.id = u.contributor_id AND c.musicbrainz_artist_id IS NULL
       AND NOT EXISTS (SELECT 1 FROM contributors o WHERE o.family_id = u.family AND o.musicbrainz_artist_id = u.mb);
    INSERT INTO contributor_keys (family_id, key, contributor_id)
    SELECT c.family_id, 'mb:' || c.musicbrainz_artist_id, c.id
      FROM contributors c
     WHERE c.musicbrainz_artist_id IS NOT NULL AND c.family_id IN (SELECT DISTINCT family FROM link_src)
    ON CONFLICT DO NOTHING;

    -- The other albums' keys: their album artists by contributor id, in order
    -- and each once, as the row trigger makes them.
    UPDATE link_src s SET album_key = 'tag:' || a.ids || chr(31) || lower(s.title)
      FROM (SELECT x.family, x.aa, string_agg(x.contributor_id::text, ';' ORDER BY x.ord) AS ids
              FROM (SELECT DISTINCT ON (d.family, d.aa, k.contributor_id) d.family, d.aa, k.contributor_id,
                           p.ord * 100 + k.position AS ord
                      FROM (SELECT DISTINCT family, aa FROM link_src WHERE mb_rel IS NULL) d
                     CROSS JOIN LATERAL unnest(string_to_array(d.aa, ';')) WITH ORDINALITY AS p(name, ord)
                      JOIN contributor_keys k ON k.family_id = d.family AND k.key = 'name:' || lower(trim(p.name))
                     WHERE trim(p.name) <> ''
                     ORDER BY d.family, d.aa, k.contributor_id, p.ord, k.position) x
             GROUP BY x.family, x.aa) a
     WHERE s.mb_rel IS NULL AND a.family = s.family AND a.aa = s.aa;
    ANALYZE link_src;

    -- Albums no key knows yet, with their artist credits.
    DROP TABLE IF EXISTS link_new_albums;
    CREATE TEMP TABLE link_new_albums ON COMMIT DROP AS
    SELECT DISTINCT ON (family, album_key) gen_random_uuid() AS id, family, album_key, title, aa,
           COALESCE(is_compilation, FALSE) AS is_compilation, mb_rel
      FROM link_src s
     WHERE NOT EXISTS (SELECT 1 FROM album_keys k WHERE k.family_id = s.family AND k.key = s.album_key)
     ORDER BY family, album_key, created_at, id;
    INSERT INTO albums (id, family_id, title, artist_display, is_compilation, musicbrainz_release_id)
    SELECT id, family, title, aa, is_compilation, mb_rel FROM link_new_albums;
    INSERT INTO album_keys (family_id, key, album_id) SELECT family, album_key, id FROM link_new_albums;
    INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
    SELECT contributor_id, 'album', id, 'artist', (row_number() OVER (PARTITION BY id ORDER BY ord) - 1)::smallint
      FROM (SELECT DISTINCT ON (m.id, k.contributor_id) k.contributor_id, m.id, p.ord * 100 + k.position AS ord
              FROM link_new_albums m
             CROSS JOIN LATERAL (SELECT trim(x) AS name, ord FROM unnest(string_to_array(m.aa, ';')) WITH ORDINALITY AS u(x, ord)
                                  WHERE trim(x) <> '') p
              JOIN contributor_keys k ON k.family_id = m.family AND k.key = 'name:' || lower(p.name)
             ORDER BY m.id, k.contributor_id, p.ord, k.position) d
    ON CONFLICT DO NOTHING;

    -- Release groups: by MusicBrainz id, else one per album.
    INSERT INTO release_groups (family_id, title, musicbrainz_release_group_id, match_key)
    SELECT DISTINCT ON (family, mb_rg) family, title, mb_rg, 'mb:' || mb_rg
      FROM link_src WHERE mb_rg IS NOT NULL ORDER BY family, mb_rg, created_at, id
    ON CONFLICT (family_id, match_key) DO NOTHING;
    UPDATE albums a SET release_group_id = rg.id
      FROM (SELECT DISTINCT ON (family, album_key) family, album_key, mb_rg FROM link_src
             WHERE mb_rg IS NOT NULL ORDER BY family, album_key, created_at DESC, id DESC) s
      JOIN album_keys ak ON ak.family_id = s.family AND ak.key = s.album_key
      JOIN release_groups rg ON rg.family_id = s.family AND rg.match_key = 'mb:' || s.mb_rg
     WHERE a.id = ak.album_id AND a.release_group_set_at IS NULL AND a.release_group_id IS DISTINCT FROM rg.id;
    -- One per album for the rest. Matched back through the unique index on
    -- (family_id, match_key): joining to the inserted rows themselves made the
    -- planner compare every album with every new group (an hour at 100,000).
    INSERT INTO release_groups (family_id, title, match_key)
    SELECT a.family_id, a.title, 'album:' || a.id FROM albums a
     WHERE a.release_group_id IS NULL AND a.family_id IN (SELECT DISTINCT family FROM link_src)
    ON CONFLICT (family_id, match_key) DO NOTHING;
    UPDATE albums a SET release_group_id = rg.id
      FROM release_groups rg
     WHERE a.release_group_id IS NULL AND a.family_id IN (SELECT DISTINCT family FROM link_src)
       AND rg.family_id = a.family_id AND rg.match_key = 'album:' || a.id;

    INSERT INTO recordings (family_id, title, musicbrainz_recording_id)
    SELECT DISTINCT ON (family, mb_rec) family, track_title, mb_rec
      FROM link_src WHERE mb_rec IS NOT NULL ORDER BY family, mb_rec, created_at, id
    ON CONFLICT (family_id, musicbrainz_recording_id) DO NOTHING;

    UPDATE music_tracks_all t SET album_id = a.album_id, recording_id = r.id
      FROM link_src s
      JOIN album_keys a ON a.family_id = s.family AND a.key = s.album_key
      LEFT JOIN recordings r ON r.family_id = s.family AND r.musicbrainz_recording_id = s.mb_rec
     WHERE t.id = s.id AND (t.album_id IS DISTINCT FROM a.album_id OR t.recording_id IS DISTINCT FROM r.id);
    -- A user's tracks outside any household are linked to nothing.
    UPDATE music_tracks_all SET album_id = NULL, recording_id = NULL
     WHERE (p_user IS NULL OR user_id = p_user) AND album_id IS NOT NULL
       AND NOT EXISTS (SELECT 1 FROM family_members fm WHERE fm.user_id = music_tracks_all.user_id);

    INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
    SELECT contributor_id, 'track', id, role, (row_number() OVER (PARTITION BY id ORDER BY ord) - 1)::smallint
      FROM (SELECT DISTINCT ON (n.id, n.role, k.contributor_id) k.contributor_id, n.id, n.role,
                   n.ord * 100 + k.position AS ord
              FROM link_names n
              JOIN contributor_keys k ON k.family_id = n.family AND k.key = 'name:' || lower(n.name)
              -- not a role corrected by hand
             WHERE NOT EXISTS (SELECT 1 FROM credits u WHERE u.item_kind = 'track' AND u.item_id = n.id
                                  AND u.role = n.role AND u.source = 'user')
             ORDER BY n.id, n.role, k.contributor_id, n.ord, k.position) d
    ON CONFLICT DO NOTHING;

    -- Join phrases from the final lists (see music_credit_joins), for the
    -- linked tracks and their albums.
    DROP TABLE IF EXISTS link_items;
    CREATE TEMP TABLE link_items ON COMMIT DROP AS
    SELECT 'track'::text AS kind, id FROM link_src
    UNION
    SELECT 'album', t.album_id FROM music_tracks_all t JOIN link_src s ON s.id = t.id WHERE t.album_id IS NOT NULL;
    ANALYZE link_items;
    UPDATE credits c SET join_phrase = j.phrase
      FROM (SELECT cr.id, CASE WHEN lead(cr.role) OVER w IS NOT NULL THEN '; '
                               WHEN cr.role = 'artist' AND bool_or(cr.role = 'featured') OVER item THEN ' feat. '
                               ELSE '' END AS phrase
              FROM credits cr JOIN link_items li ON li.kind = cr.item_kind AND li.id = cr.item_id
            WINDOW w AS (PARTITION BY cr.item_kind, cr.item_id, cr.role ORDER BY cr.position),
                   item AS (PARTITION BY cr.item_kind, cr.item_id)) j
     WHERE c.id = j.id AND c.join_phrase IS DISTINCT FROM j.phrase;
    -- New albums' artist line from their credits, as the row trigger does.
    UPDATE albums a SET artist_display = l.line
      FROM (SELECT c.item_id, string_agg(k.name || c.join_phrase, '' ORDER BY c.position) AS line
              FROM link_new_albums m
              JOIN credits c ON c.item_kind = 'album' AND c.item_id = m.id AND c.role = 'artist'
              JOIN contributors k ON k.id = c.contributor_id
             GROUP BY c.item_id) l
     WHERE a.id = l.item_id AND a.artist_display IS DISTINCT FROM l.line;

    INSERT INTO genres (family_id, name, key)
    SELECT DISTINCT ON (s.family, lower(trim(g))) s.family, trim(g), lower(trim(g))
      FROM link_src s, unnest(regexp_split_to_array(COALESCE(s.genre, ''), '\s*[;,]\s*')) g
     WHERE trim(g) <> ''
     ORDER BY s.family, lower(trim(g)), trim(g)
    ON CONFLICT DO NOTHING;
    INSERT INTO track_genres (track_id, genre_id, position)
    SELECT s.id, gr.id, (min(g.ord) - 1)::smallint
      FROM link_src s
     CROSS JOIN LATERAL unnest(regexp_split_to_array(COALESCE(s.genre, ''), '\s*[;,]\s*')) WITH ORDINALITY AS g(name, ord)
      JOIN genres gr ON gr.family_id = s.family AND gr.key = lower(trim(g.name))
     WHERE trim(g.name) <> ''
     GROUP BY s.id, gr.id
    ON CONFLICT DO NOTHING;

    PERFORM set_config('own_audio.bulk_link', 'off', true);
    PERFORM set_config('own_audio.quiet', 'off', true);
END $$;

-- ── Link every existing track, then keep them linked ─────────────────────

SELECT music_link_tracks(NULL);


CREATE TRIGGER music_tracks_link_before
    BEFORE INSERT OR UPDATE OF artist, album, album_artist, genre, is_compilation, user_id, album_id,
        musicbrainz_recording_id, musicbrainz_release_id, musicbrainz_artist_id, musicbrainz_release_group_id
    ON music_tracks_all FOR EACH ROW EXECUTE FUNCTION music_link_track_before();

CREATE TRIGGER music_tracks_link_after
    AFTER INSERT OR UPDATE OF artist, album, album_artist, genre, is_compilation, user_id, album_id,
        musicbrainz_recording_id, musicbrainz_release_id, musicbrainz_artist_id, musicbrainz_release_group_id
    ON music_tracks_all FOR EACH ROW EXECUTE FUNCTION music_link_track_after();

