-- 0015: Authors, collections, series, favorites, sharing

-- ── Authors (normalized) ─────────────────────────────────────────────────────

CREATE TABLE audiobook_authors (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name        TEXT NOT NULL,
    sort_name   TEXT, -- e.g. "Tolkien, J.R.R." for sorting
    bio         TEXT,
    image_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_audiobook_authors_name ON audiobook_authors (lower(name));

-- Junction: book ↔ author (many-to-many)
CREATE TABLE audiobook_book_authors (
    book_id   UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    author_id UUID NOT NULL REFERENCES audiobook_authors (id) ON DELETE CASCADE,
    role      TEXT NOT NULL DEFAULT 'author', -- 'author', 'narrator', 'editor'
    PRIMARY KEY (book_id, author_id, role)
);

CREATE INDEX idx_book_authors_author ON audiobook_book_authors (author_id);

-- ── Series ───────────────────────────────────────────────────────────────────

CREATE TABLE audiobook_series (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_series_user ON audiobook_series (user_id);

-- Junction: book ↔ series with ordering
CREATE TABLE audiobook_series_books (
    series_id   UUID NOT NULL REFERENCES audiobook_series (id) ON DELETE CASCADE,
    book_id     UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    position    DOUBLE PRECISION NOT NULL DEFAULT 1, -- allows 1.5 for "Book 1.5"
    PRIMARY KEY (series_id, book_id)
);

-- ── Collections (user-created groupings) ─────────────────────────────────────

CREATE TABLE audiobook_collections (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name        TEXT NOT NULL,
    description TEXT,
    cover_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL,
    is_public   BOOLEAN NOT NULL DEFAULT false,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_collections_user ON audiobook_collections (user_id);

-- Junction: collection ↔ book
CREATE TABLE audiobook_collection_books (
    collection_id UUID NOT NULL REFERENCES audiobook_collections (id) ON DELETE CASCADE,
    book_id       UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    added_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    position      INT NOT NULL DEFAULT 0,
    PRIMARY KEY (collection_id, book_id)
);

-- ── Favorites ────────────────────────────────────────────────────────────────

CREATE TABLE audiobook_favorites (
    user_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    book_id    UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, book_id)
);

-- ── Share tokens ─────────────────────────────────────────────────────────────

CREATE TABLE audiobook_share_tokens (
    id          UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    owner_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    book_id     UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    token       TEXT NOT NULL UNIQUE,
    label       TEXT, -- e.g. "For Alice"
    max_uses    INT, -- NULL = unlimited
    used_count  INT NOT NULL DEFAULT 0,
    expires_at  TIMESTAMPTZ, -- NULL = never
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_share_tokens_owner ON audiobook_share_tokens (owner_id);
CREATE INDEX idx_share_tokens_token ON audiobook_share_tokens (token);

-- ── Borrowed audiobooks (redeemed share tokens) ──────────────────────────────

CREATE TABLE audiobook_borrowed (
    id             UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    borrower_id    UUID NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    book_id        UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    share_token_id UUID NOT NULL REFERENCES audiobook_share_tokens (id) ON DELETE CASCADE,
    borrowed_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at     TIMESTAMPTZ, -- owner can revoke access
    UNIQUE (borrower_id, book_id) -- a user can only borrow a book once
);

CREATE INDEX idx_borrowed_user ON audiobook_borrowed (borrower_id);

-- Borrowed books use the existing `audiobook_progress` table — the
-- (user_id, book_id) key already supports per-user progress for any book.
-- Same for bookmarks: (user_id, book_id).
-- No schema change needed for per-user progress tracking.

-- ── Tags (lightweight categorization) ────────────────────────────────────────

CREATE TABLE audiobook_tags (
    id   UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_tags_name ON audiobook_tags (lower(name));

CREATE TABLE audiobook_book_tags (
    book_id UUID NOT NULL REFERENCES audiobook_books (id) ON DELETE CASCADE,
    tag_id  UUID NOT NULL REFERENCES audiobook_tags (id) ON DELETE CASCADE,
    PRIMARY KEY (book_id, tag_id)
);

CREATE INDEX idx_book_tags_tag ON audiobook_book_tags (tag_id);
