-- Read-only library folders (docs/IMPLEMENTATION_PLAN.md Phase 4 A, issue #1).
-- Folders come from configuration (LIBRARY__*); this table remembers them so
-- their ids, and so their files' keys, stay stable across restarts.
CREATE TABLE library_folders (
    id                 UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The path as configured, inside the server's container.
    path               TEXT        NOT NULL UNIQUE,
    kind               TEXT        NOT NULL CHECK (kind IN ('music', 'audiobooks')),
    family_id          UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    -- Items found here are owned by this account: the family's first admin.
    owner_id           UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    visibility         TEXT        NOT NULL DEFAULT 'family' CHECK (visibility IN ('family', 'private')),
    scan_started_at    TIMESTAMPTZ,
    scan_finished_at   TIMESTAMPTZ,
    scan_error         TEXT,
    files_seen         INTEGER     NOT NULL DEFAULT 0,
    files_added        INTEGER     NOT NULL DEFAULT 0,
    created_at         TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Every audio file the scanner has handled, so the next scan can skip it when
-- its size and modification time are unchanged. A row stays after its item is
-- deleted: removing an item from a read-only folder hides it, and the scanner
-- does not bring it back unless the file itself changes.
CREATE TABLE library_files (
    folder_id     UUID        NOT NULL REFERENCES library_folders (id) ON DELETE CASCADE,
    rel_path      TEXT        NOT NULL,
    size_bytes    BIGINT      NOT NULL,
    mtime_secs    BIGINT      NOT NULL,
    -- 'track' or 'book_file'; the item it became, if any.
    item_kind     TEXT,
    item_id       UUID,
    missing       BOOLEAN     NOT NULL DEFAULT false,
    last_seen_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (folder_id, rel_path)
);

CREATE INDEX library_files_item_idx ON library_files (item_id);
