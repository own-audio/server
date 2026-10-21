-- When a library-folder book's chapter marks were last read from its files
-- (M4B, M4A and MP3 can carry them). NULL: not yet, or a file changed since;
-- the scanner reads those books' chapters and sets it, so a book is probed once,
-- not on every scan. Books already indexed are read at the next scan.
ALTER TABLE audiobook_books_all ADD COLUMN chapters_read_at TIMESTAMPTZ;
