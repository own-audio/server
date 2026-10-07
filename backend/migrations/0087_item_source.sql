-- Where a track's or a book's audio comes from: an upload, or a read-only
-- library folder (`library_folders`). Clients hide actions that cannot work on
-- folder items (editing the file, deleting it from disk).
ALTER TABLE music_tracks_all
    ADD COLUMN source TEXT NOT NULL DEFAULT 'upload' CHECK (source IN ('upload', 'folder'));
ALTER TABLE audiobook_books_all
    ADD COLUMN source TEXT NOT NULL DEFAULT 'upload' CHECK (source IN ('upload', 'folder'));

CREATE OR REPLACE VIEW music_tracks AS
    SELECT * FROM music_tracks_all WHERE trashed_at IS NULL;
CREATE OR REPLACE VIEW audiobook_books AS
    SELECT * FROM audiobook_books_all WHERE trashed_at IS NULL;

-- Items a scan already created before this column existed.
UPDATE music_tracks_all t SET source = 'folder'
FROM media_objects mo WHERE mo.id = t.audio_object_id AND mo.object_key LIKE 'folder/%';
UPDATE audiobook_books_all b SET source = 'folder'
WHERE EXISTS (
    SELECT 1 FROM audiobook_files f JOIN media_objects mo ON mo.id = f.audio_object_id
    WHERE f.book_id = b.id AND mo.object_key LIKE 'folder/%');
