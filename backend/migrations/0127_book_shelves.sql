-- Books' series, collections and tags in the change feed (docs/api-v2-completion-plan.md,
-- phase 5). Series and collections become kinds of their own (`series`, `collection`);
-- a book's tags and its series are part of the book's record, so a change to either is a
-- change of the book.
--
-- 1. A series: its own row, and the books it names (each book's record carries the
--    series' name and the book's place in it, as far as the reader may see the series).
-- 2. A book's place in a series: the series and the book.
-- 3. A collection and what it holds: the collection (its record counts the books).
-- 4. A book's tags: the book.
-- 5. What a book makes visible (0126's `change_log_item`): the series and collections it
--    is in, whose records list or count only the books the reader can see.

-- ── 1. Series ────────────────────────────────────────────────────────────

-- Series and collections are their owner's, private or shared with the household, like
-- the books they hold; readers decide visibility when they read.
CREATE FUNCTION change_log_series() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    PERFORM change_log('series', r.id, music_owner_family(r.user_id), NULL);
    -- Renamed, shared, unshared or handed to someone else: every book shows it differently.
    -- (A deleted series takes its places with it; their own trigger logs the books.)
    IF TG_OP = 'UPDATE' THEN
        PERFORM change_log('book', sb.book_id, music_owner_family(b.user_id), NULL)
           FROM audiobook_series_books sb JOIN audiobook_books_all b ON b.id = sb.book_id
          WHERE sb.series_id = r.id;
    END IF;
    RETURN NULL;
END $$;

CREATE TRIGGER audiobook_series_change AFTER INSERT OR DELETE ON audiobook_series
    FOR EACH ROW EXECUTE FUNCTION change_log_series();
CREATE TRIGGER audiobook_series_update
    AFTER UPDATE OF name, description, user_id, family_id ON audiobook_series
    FOR EACH ROW WHEN (OLD.name IS DISTINCT FROM NEW.name OR OLD.description IS DISTINCT FROM NEW.description
                       OR OLD.user_id IS DISTINCT FROM NEW.user_id OR OLD.family_id IS DISTINCT FROM NEW.family_id)
    EXECUTE FUNCTION change_log_series();

-- ── 2, 3, 4. Places in a series, entries of a collection, a book's tags ──

CREATE FUNCTION change_log_shelf_part() RETURNS trigger
LANGUAGE plpgsql AS $$
DECLARE
    r RECORD;
BEGIN
    IF TG_OP = 'DELETE' THEN r := OLD; ELSE r := NEW; END IF;
    CASE TG_ARGV[0]
        WHEN 'series_book' THEN
            -- When the series itself is being deleted it is gone by now; its own row says so.
            PERFORM change_log('series', s.id, music_owner_family(s.user_id), NULL)
               FROM audiobook_series s WHERE s.id = r.series_id;
            PERFORM change_log('book', b.id, music_owner_family(b.user_id), NULL)
               FROM audiobook_books_all b WHERE b.id = r.book_id;
        WHEN 'collection_book' THEN
            PERFORM change_log('collection', c.id, music_owner_family(c.user_id), NULL)
               FROM audiobook_collections c WHERE c.id = r.collection_id;
        WHEN 'book_tag' THEN
            PERFORM change_log('book', b.id, music_owner_family(b.user_id), NULL)
               FROM audiobook_books_all b WHERE b.id = r.book_id;
    END CASE;
    RETURN NULL;
END $$;

CREATE TRIGGER audiobook_series_books_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_series_books
    FOR EACH ROW EXECUTE FUNCTION change_log_shelf_part('series_book');
CREATE TRIGGER audiobook_collection_books_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_collection_books
    FOR EACH ROW EXECUTE FUNCTION change_log_shelf_part('collection_book');
CREATE TRIGGER audiobook_book_tags_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_book_tags
    FOR EACH ROW EXECUTE FUNCTION change_log_shelf_part('book_tag');

CREATE TRIGGER audiobook_collections_change AFTER INSERT OR UPDATE OR DELETE ON audiobook_collections
    FOR EACH ROW EXECUTE FUNCTION change_log_owned('collection');

-- A renamed tag renames it on every book that has it.
CREATE FUNCTION change_log_tag() RETURNS trigger
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log('book', b.id, music_owner_family(b.user_id), NULL)
       FROM audiobook_book_tags bt JOIN audiobook_books_all b ON b.id = bt.book_id
      WHERE bt.tag_id = NEW.id;
    RETURN NULL;
END $$;

CREATE TRIGGER audiobook_tags_change AFTER UPDATE OF name ON audiobook_tags
    FOR EACH ROW WHEN (OLD.name IS DISTINCT FROM NEW.name)
    EXECUTE FUNCTION change_log_tag();

-- From a book to the series and collections that hold it, without a scan: the primary keys
-- start with the series and the collection.
CREATE INDEX audiobook_series_books_book_idx ON audiobook_series_books (book_id);
CREATE INDEX audiobook_collection_books_book_idx ON audiobook_collection_books (book_id);

-- ── 5. What a book makes visible ─────────────────────────────────────────

-- 0126's function, with the series and collections a book is in: a book that appears or goes
-- for someone changes what those show them.
CREATE OR REPLACE FUNCTION change_log_item(p_kind TEXT, p_item UUID, p_family UUID, p_user UUID) RETURNS void
LANGUAGE plpgsql AS $$
BEGIN
    PERFORM change_log(p_kind, p_item, p_family, p_user);
    IF p_kind = 'track' THEN
        PERFORM change_log_track_refs(t.album_id, t.recording_id, p_item, p_family, p_user)
           FROM music_tracks_all t WHERE t.id = p_item;
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
