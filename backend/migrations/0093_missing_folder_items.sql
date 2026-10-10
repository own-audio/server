-- A library-folder file that disappears hides its item until it is back
-- (issue #1): the item keeps its stars, playlists and history, but no client
-- lists it or tries to play a file that isn't there. Set and cleared by the
-- scanner (library_folders::scan_folder); a book is hidden when none of its
-- files are left.
ALTER TABLE music_tracks_all ADD COLUMN missing_at TIMESTAMPTZ;
ALTER TABLE audiobook_books_all ADD COLUMN missing_at TIMESTAMPTZ;

CREATE OR REPLACE VIEW music_tracks AS
    SELECT * FROM music_tracks_all WHERE trashed_at IS NULL AND missing_at IS NULL;
CREATE OR REPLACE VIEW audiobook_books AS
    SELECT * FROM audiobook_books_all WHERE trashed_at IS NULL AND missing_at IS NULL;

-- The partial indexes behind the views carry the same condition, so browsing
-- stays an index-only scan.
DROP INDEX music_tracks_album_group_idx;
CREATE INDEX music_tracks_album_group_idx
    ON music_tracks_all (family_id, album_artist_key, album_key)
    INCLUDE (user_id, duration_secs, cover_object_id, created_at)
    WHERE trashed_at IS NULL AND missing_at IS NULL;
DROP INDEX music_tracks_primary_artist_idx;
CREATE INDEX music_tracks_primary_artist_idx
    ON music_tracks_all (family_id, primary_artist_key)
    WHERE trashed_at IS NULL AND missing_at IS NULL;
DROP INDEX music_tracks_subsonic_group_idx;
CREATE INDEX music_tracks_subsonic_group_idx ON music_tracks_all (
    (COALESCE(NULLIF(trim(artist), ''), 'Unknown Artist')),
    (COALESCE(NULLIF(trim(album), ''), 'Unknown Album'))
) WHERE trashed_at IS NULL AND missing_at IS NULL;

-- Files already marked missing before this column existed.
UPDATE music_tracks_all t SET missing_at = lf.last_seen_at
FROM library_files lf
WHERE lf.item_kind = 'track' AND lf.item_id = t.id AND lf.missing;
UPDATE audiobook_books_all b SET missing_at = now()
WHERE b.source = 'folder' AND NOT EXISTS (
    SELECT 1 FROM library_files lf JOIN audiobook_files f ON f.id = lf.item_id
    WHERE lf.item_kind = 'book_file' AND f.book_id = b.id AND NOT lf.missing);
