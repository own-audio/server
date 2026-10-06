-- Migration 0006: playback progress and bookmarks
-- ─────────────────────────────────────────────────
-- podcast_progress tracks per-episode listen position per user.
-- audiobook_progress tracks one logical position for the whole book per user
--   (file_id + position_secs = where inside that file the user is).
-- bookmarks are timestamped markers a user places in an episode or book.

-- ── podcast_progress ─────────────────────────────────────────────────────────
CREATE TABLE podcast_progress (
    user_id       UUID             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    episode_id    UUID             NOT NULL REFERENCES podcast_episodes (id) ON DELETE CASCADE,
    position_secs DOUBLE PRECISION NOT NULL DEFAULT 0,
    completed     BOOLEAN          NOT NULL DEFAULT false,
    updated_at    TIMESTAMPTZ      NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, episode_id)
);

-- ── audiobook_progress ───────────────────────────────────────────────────────
-- One row per (user, book) – the current playback head across all files.
CREATE TABLE audiobook_progress (
    user_id       UUID             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    book_id       UUID             NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    file_id       UUID             NOT NULL REFERENCES audiobook_files (id) ON DELETE CASCADE,
    position_secs DOUBLE PRECISION NOT NULL DEFAULT 0,
    completed     BOOLEAN          NOT NULL DEFAULT false,
    updated_at    TIMESTAMPTZ      NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, book_id)
);

-- ── bookmarks ────────────────────────────────────────────────────────────────
-- Exactly one of episode_id or book_id must be non-null (enforced by CHECK).
CREATE TABLE bookmarks (
    id            UUID             PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id       UUID             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    episode_id    UUID             REFERENCES podcast_episodes (id) ON DELETE CASCADE,
    book_id       UUID             REFERENCES audiobook_books (id) ON DELETE CASCADE,
    -- file_id is set for audiobook bookmarks to locate the file.
    file_id       UUID             REFERENCES audiobook_files (id) ON DELETE SET NULL,
    position_secs DOUBLE PRECISION NOT NULL,
    label         TEXT,
    created_at    TIMESTAMPTZ      NOT NULL DEFAULT now(),

    CONSTRAINT bookmarks_target_check CHECK (
        (episode_id IS NOT NULL AND book_id IS NULL) OR
        (episode_id IS NULL     AND book_id IS NOT NULL)
    )
);

CREATE INDEX bookmarks_user_episode_idx ON bookmarks (user_id, episode_id);
CREATE INDEX bookmarks_user_book_idx    ON bookmarks (user_id, book_id);
