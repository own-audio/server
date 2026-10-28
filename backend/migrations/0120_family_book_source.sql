-- Where "identify this audiobook" looks, per family, chosen by a family admin
-- in the open-source edition (`Hooks::book_source_choice`). Open Library by
-- default: keyless Google Books is turned away nearly everywhere.
--
-- `google_books_api_key` is the family's own key for the Google source,
-- encrypted with a key derived from AUTH__SESSION_SECRET (`auth::at_rest`).
-- It is never sent back to a client.
ALTER TABLE families
    ADD COLUMN book_source          TEXT NOT NULL DEFAULT 'openlibrary'
        CHECK (book_source IN ('openlibrary', 'apple', 'google')),
    ADD COLUMN google_books_api_key TEXT;
