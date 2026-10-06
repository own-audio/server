-- "Identify this book" against Google Books needs somewhere to record what a
-- book was matched to, plus the print-edition facts the match brings back.
--
-- `google_books_volume_id` is the counterpart of `music_tracks`'
-- `musicbrainz_recording_id`: without it there is no way to tell a book that
-- was identified from one nobody has tried yet.
--
-- No narrator column here on purpose — Google Books describes the print
-- edition and has no narrator to give.
ALTER TABLE audiobook_books
    ADD COLUMN IF NOT EXISTS google_books_volume_id TEXT,
    ADD COLUMN IF NOT EXISTS isbn                   TEXT,
    ADD COLUMN IF NOT EXISTS publisher              TEXT,
    ADD COLUMN IF NOT EXISTS published_year         INTEGER;
