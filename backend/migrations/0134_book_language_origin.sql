-- A book's language and where it came from, so every client can mark a machine
-- translation without knowing who made it.
--
-- language: a BCP 47 tag as the book's owner or the server knows it (`en`,
-- `cs`, `pt-BR`); NULL when nobody said. The owner may change it.
--
-- origin: NULL for an ordinary upload or library-folder book. Otherwise an
-- object written only by the server, never by a client's PATCH:
--   {"kind": "narrated"}                         read aloud by a synthetic voice
--   {"kind": "machine_translated",               translated by machine, then read
--    "source_language": "cs", "translator": "google"}
-- plus "voice" (the reading voice's display name) on either kind. Clients
-- treat an unknown kind as an ordinary book.
ALTER TABLE audiobook_books_all
    ADD COLUMN language TEXT CHECK (language IS NULL OR length(language) BETWEEN 2 AND 35),
    ADD COLUMN origin JSONB CHECK (origin IS NULL OR coalesce(jsonb_typeof(origin -> 'kind') = 'string', false));

-- The view's column list was fixed when it was made (0093); chapters_read_at
-- (0117) joins it here too, at the end, which CREATE OR REPLACE allows.
CREATE OR REPLACE VIEW audiobook_books AS
    SELECT * FROM audiobook_books_all WHERE trashed_at IS NULL AND missing_at IS NULL;
