-- Migration 0005: audiobooks (books, files, chapters)
-- ─────────────────────────────────────────────────
-- An audiobook_book is owned by one user.
-- It groups one or more audiobook_files (ordered parts/discs).
-- audiobook_chapters define chapter navigation within a file.

-- ── audiobook_books ──────────────────────────────────────────────────────────
CREATE TABLE audiobook_books (
    id                   UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id              UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title                TEXT        NOT NULL,
    author               TEXT,
    narrator             TEXT,
    description          TEXT,
    cover_object_id      UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    -- Computed from sum of all file durations; updated by ingestion job.
    total_duration_secs  INTEGER,
    -- Set for URL / RSS-style ingestion; NULL for manual file uploads.
    source_url           TEXT,
    created_at           TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at           TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX audiobook_books_user_id_idx ON audiobook_books (user_id);

-- ── audiobook_files ──────────────────────────────────────────────────────────
-- Each row is one audio file that belongs to a book.
-- `position` determines playback order (1-based, gapless).
CREATE TABLE audiobook_files (
    id               UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    book_id          UUID    NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    position         INTEGER NOT NULL,
    title            TEXT,
    duration_secs    INTEGER,
    audio_object_id  UUID    NOT NULL REFERENCES media_objects (id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT audiobook_files_book_position_uq UNIQUE (book_id, position)
);

CREATE INDEX audiobook_files_book_id_idx ON audiobook_files (book_id);

-- ── audiobook_chapters ───────────────────────────────────────────────────────
-- Optional chapter metadata extracted from file tags or a supplied manifest.
-- `position` is the global chapter index within the whole book.
-- `start_time_secs` is the byte-offset within the referenced file, not the book.
CREATE TABLE audiobook_chapters (
    id               UUID             PRIMARY KEY DEFAULT gen_random_uuid(),
    book_id          UUID             NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    file_id          UUID             REFERENCES audiobook_files (id) ON DELETE CASCADE,
    position         INTEGER          NOT NULL,
    title            TEXT             NOT NULL,
    start_time_secs  DOUBLE PRECISION NOT NULL,
    created_at       TIMESTAMPTZ      NOT NULL DEFAULT now()
);

CREATE INDEX audiobook_chapters_book_id_idx ON audiobook_chapters (book_id, position);
