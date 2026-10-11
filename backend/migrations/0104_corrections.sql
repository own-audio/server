-- Corrections on the entities (docs/data-model-plan.md, E4): merge and split
-- contributors, merge albums. A wrong match is fixed once, on the entity, and
-- holds: the keys tags are matched by move with the correction, so the next
-- scan of the same files lands where the correction put them. Renaming, and
-- moving an album to another release group, are plain updates (db::entities).

-- The album artist line shown in lists, rebuilt from the album's credits after
-- a correction renamed, merged or split one of them.
CREATE FUNCTION music_refresh_album_lines(p_contributors UUID[]) RETURNS void
LANGUAGE sql AS $$
    UPDATE albums a SET artist_display = l.line, updated_at = now()
      FROM (SELECT c.item_id, string_agg(k.name || c.join_phrase, '' ORDER BY c.position) AS line
              FROM credits c JOIN contributors k ON k.id = c.contributor_id
             WHERE c.item_kind = 'album' AND c.role = 'artist'
               AND c.item_id IN (SELECT x.item_id FROM credits x
                                  WHERE x.item_kind = 'album' AND x.contributor_id = ANY (p_contributors))
             GROUP BY c.item_id) l
     WHERE a.id = l.item_id AND a.artist_display IS DISTINCT FROM l.line
$$;

-- An album key with one album artist replaced: by another (a merge) or by
-- several (a split), each id once, in the order the row trigger keeps.
CREATE FUNCTION album_key_swap(p_key TEXT, p_old TEXT, p_new TEXT[]) RETURNS TEXT
LANGUAGE plpgsql IMMUTABLE AS $$
DECLARE
    v_head TEXT := split_part(substr(p_key, 5), chr(31), 1);
    v_out  TEXT[] := '{}';
    v_id   TEXT;
    v_new  TEXT;
BEGIN
    FOREACH v_id IN ARRAY string_to_array(v_head, ';') LOOP
        FOREACH v_new IN ARRAY CASE WHEN v_id = p_old THEN p_new ELSE ARRAY[v_id] END LOOP
            IF NOT v_new = ANY (v_out) THEN
                v_out := v_out || v_new;
            END IF;
        END LOOP;
    END LOOP;
    RETURN 'tag:' || array_to_string(v_out, ';') || chr(31) || substr(p_key, 5 + length(v_head) + 1);
END $$;

-- Album keys name their album artists by contributor id. When a correction
-- replaces one, the keys follow, and two albums whose keys then agree — the
-- same title by the same artists — are one album.
CREATE FUNCTION album_keys_replace(p_family UUID, p_old UUID, p_new UUID[]) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_key   TEXT;
    v_new   TEXT;
    v_album UUID;
    v_other UUID;
BEGIN
    FOR v_key IN SELECT key FROM album_keys
                  WHERE family_id = p_family AND key LIKE 'tag:%' AND strpos(key, p_old::text) > 0 LOOP
        v_new := album_key_swap(v_key, p_old::text, p_new::text[]);
        -- Read now, not from the loop's query: a merge further up may have moved it.
        SELECT album_id INTO v_album FROM album_keys WHERE family_id = p_family AND key = v_key;
        SELECT album_id INTO v_other FROM album_keys WHERE family_id = p_family AND key = v_new;
        IF v_other IS NULL THEN
            UPDATE album_keys SET key = v_new WHERE family_id = p_family AND key = v_key;
        ELSE
            -- The older album stays: it is the one people have seen longest.
            IF v_other <> v_album THEN
                IF (SELECT (created_at, id) < (SELECT created_at, id FROM albums WHERE id = v_other)
                      FROM albums WHERE id = v_album) THEN
                    PERFORM album_merge(v_other, v_album);
                    UPDATE album_keys SET key = v_new WHERE family_id = p_family AND key = v_key;
                    CONTINUE;
                END IF;
                PERFORM album_merge(v_album, v_other);
            END IF;
            DELETE FROM album_keys WHERE family_id = p_family AND key = v_key;
        END IF;
    END LOOP;
END $$;

-- Two contributors that are one: everything `p_from` has goes to `p_into` —
-- credits, keys (so tags naming either find `p_into`), stars, ratings, and
-- whatever `p_into` lacks of MusicBrainz id, photo, biography and sort name.
CREATE FUNCTION contributor_merge(p_from UUID, p_into UUID) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_from contributors%ROWTYPE;
    v_item RECORD;
BEGIN
    SELECT * INTO v_from FROM contributors WHERE id = p_from FOR UPDATE;
    PERFORM 1 FROM contributors WHERE id = p_into FOR UPDATE;

    -- Both credited in one role on one item: the target's credit stays.
    FOR v_item IN
        DELETE FROM credits f USING credits i
         WHERE f.contributor_id = p_from AND i.contributor_id = p_into
           AND i.item_kind = f.item_kind AND i.item_id = f.item_id AND i.role = f.role
        RETURNING f.item_kind, f.item_id
    LOOP
        PERFORM music_credit_joins(v_item.item_kind, v_item.item_id);
    END LOOP;
    UPDATE credits SET contributor_id = p_into WHERE contributor_id = p_from;

    DELETE FROM contributor_keys f USING contributor_keys i
     WHERE f.contributor_id = p_from AND i.contributor_id = p_into
       AND i.family_id = f.family_id AND i.key = f.key;
    UPDATE contributor_keys SET contributor_id = p_into WHERE contributor_id = p_from;

    DELETE FROM music_stars f USING music_stars i
     WHERE f.contributor_id = p_from AND i.contributor_id = p_into AND i.user_id = f.user_id;
    UPDATE music_stars SET contributor_id = p_into WHERE contributor_id = p_from;
    DELETE FROM music_ratings f USING music_ratings i
     WHERE f.contributor_id = p_from AND i.contributor_id = p_into AND i.user_id = f.user_id;
    UPDATE music_ratings SET contributor_id = p_into WHERE contributor_id = p_from;

    -- The id is unique per household: free it before the target takes it.
    UPDATE contributors SET musicbrainz_artist_id = NULL, image_object_id = NULL WHERE id = p_from;
    UPDATE contributors i SET
        musicbrainz_artist_id = COALESCE(i.musicbrainz_artist_id, v_from.musicbrainz_artist_id),
        sort_name             = COALESCE(i.sort_name, v_from.sort_name),
        biography             = COALESCE(i.biography, v_from.biography),
        kind                  = CASE WHEN i.kind = 'unknown' THEN v_from.kind ELSE i.kind END,
        image_object_id       = COALESCE(i.image_object_id, v_from.image_object_id),
        image_fetched_at      = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_fetched_at ELSE i.image_fetched_at END,
        image_is_user_set     = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_is_user_set ELSE i.image_is_user_set END,
        image_source          = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_source ELSE i.image_source END,
        image_author          = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_author ELSE i.image_author END,
        image_license         = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_license ELSE i.image_license END,
        image_license_url     = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_license_url ELSE i.image_license_url END,
        image_source_url      = CASE WHEN i.image_object_id IS NULL AND v_from.image_object_id IS NOT NULL
                                     THEN v_from.image_source_url ELSE i.image_source_url END,
        edited_at             = now(),
        updated_at            = now()
     WHERE i.id = p_into;
    PERFORM album_keys_replace(v_from.family_id, p_from, ARRAY[p_into]);
    DELETE FROM contributors WHERE id = p_from;
    PERFORM music_refresh_album_lines(ARRAY[p_into]);
END $$;

-- One contributor that is several ("A & B" read as one name when they are
-- two): every credit becomes one per part, in its place and in order, and the
-- names it was matched by lead to all the parts, so the next scan credits
-- them too. Each part is the household's contributor of that name, made if
-- new; stars and ratings go to every part. Returns the parts in order.
CREATE FUNCTION contributor_split(p_id UUID, p_names TEXT[]) RETURNS TABLE (contributor_id UUID)
LANGUAGE plpgsql AS $$
#variable_conflict use_column
DECLARE
    v_family UUID;
    v_parts  UUID[] := '{}';
    v_name   TEXT;
    v_cid    UUID;
    v_item   RECORD;
BEGIN
    SELECT c.family_id INTO v_family FROM contributors c WHERE c.id = p_id FOR UPDATE;

    -- Its names first leave it, so a part spelled like one of them is a new
    -- contributor rather than this one again.
    DROP TABLE IF EXISTS split_keys, split_rows;
    CREATE TEMP TABLE split_keys ON COMMIT DROP AS
    SELECT k.key FROM contributor_keys k WHERE k.contributor_id = p_id AND k.key LIKE 'name:%';
    DELETE FROM contributor_keys k WHERE k.contributor_id = p_id;

    FOREACH v_name IN ARRAY p_names LOOP
        CONTINUE WHEN trim(v_name) = '';
        v_cid := music_contributor(v_family, v_name, NULL);
        IF NOT v_cid = ANY (v_parts) THEN
            v_parts := v_parts || v_cid;
        END IF;
    END LOOP;

    INSERT INTO contributor_keys (family_id, key, position, contributor_id)
    SELECT v_family, k.key, (p.ord - 1)::smallint, p.cid
      FROM split_keys k, unnest(v_parts) WITH ORDINALITY AS p(cid, ord)
     WHERE NOT EXISTS (SELECT 1 FROM contributor_keys o WHERE o.family_id = v_family AND o.key = k.key);

    -- Every item it is credited on gets its list rebuilt: the credit replaced by
    -- the parts, a part already credited in that role kept once, then renumbered.
    CREATE TEMP TABLE split_rows ON COMMIT DROP AS
    SELECT DISTINCT ON (c.item_kind, c.item_id, c.role, COALESCE(p.cid, c.contributor_id))
           CASE WHEN p.ord IS NULL OR p.ord = 1 THEN c.id ELSE gen_random_uuid() END AS id,
           c.item_kind, c.item_id, c.role, c.position, COALESCE(p.ord, 0) AS sub, c.source,
           CASE WHEN p.cid IS NULL THEN c.credited_as END AS credited_as,
           COALESCE(p.cid, c.contributor_id) AS contributor_id
      FROM credits c
      JOIN (SELECT DISTINCT x.item_kind, x.item_id FROM credits x WHERE x.contributor_id = p_id) i
        ON i.item_kind = c.item_kind AND i.item_id = c.item_id
      LEFT JOIN unnest(v_parts) WITH ORDINALITY AS p(cid, ord) ON c.contributor_id = p_id
     ORDER BY c.item_kind, c.item_id, c.role, COALESCE(p.cid, c.contributor_id), c.position, COALESCE(p.ord, 0);
    DELETE FROM credits c USING (SELECT DISTINCT r.item_kind, r.item_id FROM split_rows r) i
     WHERE c.item_kind = i.item_kind AND c.item_id = i.item_id;
    INSERT INTO credits (id, contributor_id, item_kind, item_id, role, position, credited_as, source)
    SELECT r.id, r.contributor_id, r.item_kind, r.item_id, r.role,
           (row_number() OVER (PARTITION BY r.item_kind, r.item_id ORDER BY r.position, r.sub) - 1)::smallint,
           r.credited_as, r.source
      FROM split_rows r;
    FOR v_item IN SELECT DISTINCT r.item_kind, r.item_id FROM split_rows r LOOP
        PERFORM music_credit_joins(v_item.item_kind, v_item.item_id);
    END LOOP;

    INSERT INTO music_stars (user_id, contributor_id, created_at)
    SELECT s.user_id, p.cid, s.created_at FROM music_stars s, unnest(v_parts) AS p(cid)
     WHERE s.contributor_id = p_id
    ON CONFLICT DO NOTHING;
    INSERT INTO music_ratings (user_id, contributor_id, rating, updated_at)
    SELECT r.user_id, p.cid, r.rating, r.updated_at FROM music_ratings r, unnest(v_parts) AS p(cid)
     WHERE r.contributor_id = p_id
    ON CONFLICT DO NOTHING;

    -- Kept by the orphan sweep even when nothing credits them for a while, so
    -- the split holds.
    UPDATE contributors c SET edited_at = now(), updated_at = now() WHERE c.id = ANY (v_parts);
    PERFORM album_keys_replace(v_family, p_id, v_parts);
    DELETE FROM contributors c WHERE c.id = p_id;
    PERFORM music_refresh_album_lines(v_parts);

    RETURN QUERY SELECT p.cid FROM unnest(v_parts) WITH ORDINALITY AS p(cid, ord) ORDER BY p.ord;
END $$;

-- Two albums that are one (the same release tagged two ways): the tracks,
-- keys, stars and ratings of `p_from` go to `p_into`, which keeps its title,
-- credits and release group and takes what it lacks of the rest.
CREATE FUNCTION album_merge(p_from UUID, p_into UUID) RETURNS void
LANGUAGE plpgsql AS $$
DECLARE
    v_from albums%ROWTYPE;
    v_bulk TEXT;
BEGIN
    SELECT * INTO v_from FROM albums WHERE id = p_from FOR UPDATE;
    PERFORM 1 FROM albums WHERE id = p_into FOR UPDATE;
    IF v_from.id IS NULL OR NOT FOUND OR p_from = p_into THEN
        RAISE EXCEPTION 'album_merge: % into %', p_from, p_into;
    END IF;

    UPDATE album_keys SET album_id = p_into WHERE album_id = p_from;
    -- With the row triggers aside: the keys are already moved, and a merge
    -- inside a contributor merge runs while other keys are still being
    -- rewritten. `album_id` is not watched by the file-sync log.
    v_bulk := current_setting('own_audio.bulk_link', true);
    PERFORM set_config('own_audio.bulk_link', 'on', true);
    UPDATE music_tracks_all SET album_id = p_into WHERE album_id = p_from;
    PERFORM set_config('own_audio.bulk_link', COALESCE(v_bulk, 'off'), true);

    DELETE FROM music_stars f USING music_stars i
     WHERE f.album_id = p_from AND i.album_id = p_into AND i.user_id = f.user_id;
    UPDATE music_stars SET album_id = p_into WHERE album_id = p_from;
    DELETE FROM music_ratings f USING music_ratings i
     WHERE f.album_id = p_from AND i.album_id = p_into AND i.user_id = f.user_id;
    UPDATE music_ratings SET album_id = p_into WHERE album_id = p_from;

    UPDATE albums i SET
        musicbrainz_release_id = COALESCE(i.musicbrainz_release_id, v_from.musicbrainz_release_id),
        release_date           = COALESCE(i.release_date, v_from.release_date),
        country                = COALESCE(i.country, v_from.country),
        label                  = COALESCE(i.label, v_from.label),
        barcode                = COALESCE(i.barcode, v_from.barcode),
        edited_at              = now(),
        updated_at             = now()
     WHERE i.id = p_into;
    DELETE FROM credits WHERE item_kind = 'album' AND item_id = p_from;
    DELETE FROM albums WHERE id = p_from;
END $$;
