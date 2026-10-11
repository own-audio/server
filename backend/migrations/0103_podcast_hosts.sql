-- Podcasts credit their hosts as household contributors (docs/data-model-plan.md,
-- E3): the feed's <itunes:author> becomes 'host' credits, split like a book's
-- author line (';' or ' & '). A host who also narrates a book or sings is
-- the same contributor. Kept in step with the feed's `author` by a trigger;
-- a user moving household takes their feeds' hosts along.

CREATE FUNCTION podcast_link_hosts() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    v_family UUID := music_owner_family(NEW.user_id);
    v_rec    RECORD;
    v_pos    SMALLINT := 0;
    v_cid    UUID;
BEGIN
    -- Every refresh writes `author` back; unchanged, with its credits already in
    -- this household, there is nothing to do. (A household move changes the
    -- household, so it still relinks.)
    IF TG_OP = 'UPDATE' AND OLD.author IS NOT DISTINCT FROM NEW.author AND OLD.user_id = NEW.user_id
       AND NOT EXISTS (SELECT 1 FROM credits c JOIN contributors k ON k.id = c.contributor_id
                        WHERE c.item_kind = 'podcast' AND c.item_id = NEW.id
                          AND k.family_id IS DISTINCT FROM v_family)
       AND (NULLIF(trim(NEW.author), '') IS NULL
            OR EXISTS (SELECT 1 FROM credits c WHERE c.item_kind = 'podcast' AND c.item_id = NEW.id)) THEN
        RETURN NULL;
    END IF;
    DELETE FROM credits WHERE item_kind = 'podcast' AND item_id = NEW.id AND source = 'tag';
    IF v_family IS NULL THEN
        RETURN NULL;
    END IF;
    FOR v_rec IN SELECT * FROM book_credit_names(NEW.author) ORDER BY ord LOOP
        FOR v_cid IN SELECT c.contributor_id FROM music_contributors(v_family, v_rec.name, NULL) c ORDER BY c.part LOOP
            CONTINUE WHEN EXISTS (SELECT 1 FROM credits WHERE item_kind = 'podcast' AND item_id = NEW.id
                                      AND role = 'host' AND contributor_id = v_cid);
            INSERT INTO credits (contributor_id, item_kind, item_id, role, position)
            VALUES (v_cid, 'podcast', NEW.id, 'host', v_pos)
            ON CONFLICT DO NOTHING;
            v_pos := v_pos + 1;
        END LOOP;
    END LOOP;
    PERFORM music_credit_joins('podcast', NEW.id);
    RETURN NULL;
END $$;

CREATE FUNCTION podcast_unlink() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    DELETE FROM credits WHERE item_kind = 'podcast' AND item_id = OLD.id;
    RETURN NULL;
END $$;

CREATE TRIGGER podcast_feeds_link
    AFTER INSERT OR UPDATE OF author, user_id ON podcast_feeds
    FOR EACH ROW EXECUTE FUNCTION podcast_link_hosts();
CREATE TRIGGER podcast_feeds_unlink
    AFTER DELETE ON podcast_feeds
    FOR EACH ROW EXECUTE FUNCTION podcast_unlink();

UPDATE podcast_feeds SET author = author;

CREATE OR REPLACE FUNCTION music_relink_member() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM music_link_tracks(NEW.user_id);
    UPDATE credits c SET contributor_id = music_contributor(NEW.family_id, k.name, k.musicbrainz_artist_id)
      FROM contributors k, audiobook_books_all b
     WHERE c.item_kind = 'book' AND c.source = 'user' AND c.item_id = b.id AND b.user_id = NEW.user_id
       AND k.id = c.contributor_id AND k.family_id <> NEW.family_id;
    UPDATE audiobook_books_all SET author = author WHERE user_id = NEW.user_id;
    UPDATE podcast_feeds SET author = author WHERE user_id = NEW.user_id;
    RETURN NULL;
END $$;
