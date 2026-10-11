-- Books join the contributors of 0100 (docs/data-model-plan.md, E3): an
-- author, narrator or translator is the same household contributor whether
-- they write, read or sing, credited on the book with a role. Replaces
-- audiobook_authors, which was one list for the whole server — every
-- household saw every other's authors — and audiobook_book_authors.
--
-- Two kinds of book credit, like a track's: 'tag' rows come from the book's
-- `author` / `narrator` text and are rewritten whenever it changes; 'user'
-- rows are links someone made by hand (the old audiobook_book_authors) and
-- are kept.

-- Several names in one field: ';' or ' & ' between them — "Terry Pratchett &
-- Neil Gaiman" is two authors on a book. (Music splits only on ';', because
-- bands are named "Simon & Garfunkel"; book credits are people.)
CREATE FUNCTION book_credit_names(p_text TEXT) RETURNS TABLE (name TEXT, ord INT)
LANGUAGE sql IMMUTABLE AS $$
    SELECT trim(x), o::int
      FROM unnest(regexp_split_to_array(COALESCE(p_text, ''), '\s*;\s*|\s+&\s+')) WITH ORDINALITY AS u(x, o)
     WHERE trim(x) <> ''
$$;

CREATE FUNCTION book_link_credits() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_family UUID := music_owner_family(NEW.user_id);
    v_role   TEXT;
    v_text   TEXT;
    v_pos    SMALLINT;
    v_rec    RECORD;
    v_cid    UUID;
BEGIN
    DELETE FROM credits WHERE item_kind = 'book' AND item_id = NEW.id AND source = 'tag';
    IF v_family IS NULL THEN
        RETURN NULL;
    END IF;
    FOREACH v_role IN ARRAY ARRAY['author', 'narrator'] LOOP
        v_text := CASE v_role WHEN 'author' THEN NEW.author ELSE NEW.narrator END;
        SELECT COALESCE(max(position) + 1, 0) INTO v_pos FROM credits
         WHERE item_kind = 'book' AND item_id = NEW.id AND role = v_role;
        FOR v_rec IN SELECT * FROM book_credit_names(v_text) ORDER BY ord LOOP
            v_cid := music_contributor(v_family, v_rec.name, NULL);
            CONTINUE WHEN EXISTS (SELECT 1 FROM credits WHERE item_kind = 'book' AND item_id = NEW.id
                                      AND role = v_role AND contributor_id = v_cid);
            INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
            VALUES (v_cid, 'book', NEW.id, v_role, v_pos)
            ON CONFLICT DO NOTHING;
            v_pos := v_pos + 1;
        END LOOP;
    END LOOP;
    PERFORM music_credit_joins('book', NEW.id);
    RETURN NULL;
END $$;

CREATE FUNCTION book_unlink() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM credits WHERE item_kind = 'book' AND item_id = OLD.id;
    RETURN NULL;
END $$;

-- ── Carry the hand-made links over ───────────────────────────────────────

-- Each linked author becomes a contributor in the household of the book's
-- owner; name, sort name, biography and photo come along when the
-- contributor has none yet (a singer of the same name keeps their own).
CREATE TEMP TABLE book_links ON COMMIT DROP AS
SELECT ba.book_id, ba.role, a.id AS author_id, a.name, fm.family_id AS family,
       row_number() OVER (PARTITION BY ba.book_id, ba.role ORDER BY a.name) - 1 AS pos
  FROM audiobook_book_authors ba
  JOIN audiobook_authors a ON a.id = ba.author_id
  JOIN audiobook_books_all b ON b.id = ba.book_id
  JOIN family_members fm ON fm.user_id = b.user_id;

INSERT INTO contributors (family_id, name)
SELECT DISTINCT ON (l.family, lower(trim(l.name))) l.family, trim(l.name)
  FROM book_links l
 WHERE NOT EXISTS (SELECT 1 FROM contributor_keys k WHERE k.family_id = l.family AND k.key = 'name:' || lower(trim(l.name)))
 ORDER BY l.family, lower(trim(l.name));
INSERT INTO contributor_keys (family_id, key, contributor_id)
SELECT family_id, 'name:' || lower(trim(name)), id FROM contributors
ON CONFLICT DO NOTHING;

UPDATE contributors c SET
    sort_name         = COALESCE(c.sort_name, a.sort_name),
    biography         = COALESCE(c.biography, a.bio),
    image_object_id   = COALESCE(c.image_object_id, a.image_object_id),
    image_fetched_at  = CASE WHEN c.image_object_id IS NULL THEN a.image_fetched_at ELSE c.image_fetched_at END,
    image_is_user_set = CASE WHEN c.image_object_id IS NULL THEN a.is_user_set ELSE c.image_is_user_set END,
    image_source      = CASE WHEN c.image_object_id IS NULL THEN a.image_source ELSE c.image_source END,
    image_author      = CASE WHEN c.image_object_id IS NULL THEN a.image_author ELSE c.image_author END,
    image_license     = CASE WHEN c.image_object_id IS NULL THEN a.image_license ELSE c.image_license END,
    image_license_url = CASE WHEN c.image_object_id IS NULL THEN a.image_license_url ELSE c.image_license_url END,
    image_source_url  = CASE WHEN c.image_object_id IS NULL THEN a.image_source_url ELSE c.image_source_url END
  FROM (SELECT DISTINCT ON (k.contributor_id) k.contributor_id, a.*
          FROM book_links l
          JOIN audiobook_authors a ON a.id = l.author_id
          JOIN contributor_keys k ON k.family_id = l.family AND k.key = 'name:' || lower(trim(l.name))
         ORDER BY k.contributor_id, (a.image_object_id IS NOT NULL) DESC, a.updated_at DESC) a
 WHERE c.id = a.contributor_id;

INSERT INTO credits (contributor_id, item_kind, item_id, role, position, source)
SELECT k.contributor_id, 'book', l.book_id, l.role, l.pos::smallint, 'user'
  FROM book_links l
  JOIN contributor_keys k ON k.family_id = l.family AND k.key = 'name:' || lower(trim(l.name))
ON CONFLICT DO NOTHING;

DROP TABLE audiobook_book_authors;
DROP TABLE audiobook_authors;

-- ── From here on, the text fields keep their credits ─────────────────────

CREATE TRIGGER audiobook_books_link
    AFTER INSERT OR UPDATE OF author, narrator, user_id ON audiobook_books_all
    FOR EACH ROW EXECUTE FUNCTION book_link_credits();
CREATE TRIGGER audiobook_books_unlink
    AFTER DELETE ON audiobook_books_all
    FOR EACH ROW EXECUTE FUNCTION book_unlink();

-- Every book once (a household holds hundreds, not hundreds of thousands, so
-- the row trigger is fast enough). `author` is not watched by the file-sync
-- log (0078), so this writes nothing there.
UPDATE audiobook_books_all SET author = author;

-- A user moving household: their books' hand-made credits follow them to the
-- new household's contributor of the same name, and the text credits are
-- rebuilt there. Music does the same (music_link_tracks).
CREATE OR REPLACE FUNCTION music_relink_member() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM music_link_tracks(NEW.user_id);
    UPDATE credits c SET contributor_id = music_contributor(NEW.family_id, k.name, k.musicbrainz_artist_id)
      FROM contributors k, audiobook_books_all b
     WHERE c.item_kind = 'book' AND c.source = 'user' AND c.item_id = b.id AND b.user_id = NEW.user_id
       AND k.id = c.contributor_id AND k.family_id <> NEW.family_id;
    UPDATE audiobook_books_all SET author = author WHERE user_id = NEW.user_id;
    RETURN NULL;
END $$;

-- The orphan sweep (db::entities) keeps contributors that something credits;
-- book credits count from here on.
