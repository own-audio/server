-- Migration 0077: a 30-day trash
-- ─────────────────────────────
-- docs/file-sync-plan.md §5.1.
--
-- Deleting a book, a track or a playlist now moves it to the trash; it is
-- purged 30 days later by the `trash_purge` job. A stored podcast episode is
-- trashed as a copy: the episode stays in the feed's list, only its stored
-- audio goes.
--
-- WHY THE TABLES BECOME VIEWS
--
-- A trashed row must disappear from every query at once — the REST lists,
-- search, continue-listening, album and artist aggregates, playlists, stats,
-- recommendations, smart playlists and the Subsonic surface. Roughly 150 SQL
-- strings name these three tables, and a filter added to each of them is a
-- filter that the 151st query forgets.
--
-- So each table is renamed to `…_all` and a view takes its old name, showing
-- only rows that are not in the trash. Every existing query keeps working
-- unchanged and cannot see the trash; the view is automatically updatable, so
-- INSERT/UPDATE/DELETE through it still work and still skip trashed rows. Only
-- the trash code and the purge job read `…_all`.
--
-- Foreign keys stay on the base tables. That matters beyond integrity:
-- `db::media::find_unreferenced` builds its "still referenced" test from the
-- foreign-key catalog, so a trashed item's audio stays referenced and the
-- storage sweep cannot reclaim it while it can still be restored.
--
-- ⚠ A column added to a base table later is NOT in its view until the view is
-- recreated (`SELECT *` is expanded when the view is created). A migration
-- that adds a column must `ALTER TABLE …_all ADD COLUMN` and then
-- `CREATE OR REPLACE VIEW` — see CLAUDE.md §6.

-- ── Books, tracks, playlists ────────────────────────────────────────────────

ALTER TABLE audiobook_books
    ADD COLUMN trashed_at  TIMESTAMPTZ,
    ADD COLUMN trashed_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN trash_batch UUID;
ALTER TABLE audiobook_books RENAME TO audiobook_books_all;
CREATE VIEW audiobook_books AS
    SELECT * FROM audiobook_books_all WHERE trashed_at IS NULL;
CREATE INDEX audiobook_books_trashed_idx
    ON audiobook_books_all (trashed_at) WHERE trashed_at IS NOT NULL;

ALTER TABLE music_tracks
    ADD COLUMN trashed_at  TIMESTAMPTZ,
    ADD COLUMN trashed_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN trash_batch UUID;
ALTER TABLE music_tracks RENAME TO music_tracks_all;
CREATE VIEW music_tracks AS
    SELECT * FROM music_tracks_all WHERE trashed_at IS NULL;
CREATE INDEX music_tracks_trashed_idx
    ON music_tracks_all (trashed_at) WHERE trashed_at IS NOT NULL;

ALTER TABLE music_playlists
    ADD COLUMN trashed_at  TIMESTAMPTZ,
    ADD COLUMN trashed_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN trash_batch UUID;
ALTER TABLE music_playlists RENAME TO music_playlists_all;
CREATE VIEW music_playlists AS
    SELECT * FROM music_playlists_all WHERE trashed_at IS NULL;
CREATE INDEX music_playlists_trashed_idx
    ON music_playlists_all (trashed_at) WHERE trashed_at IS NOT NULL;

-- ── Stored podcast episodes ─────────────────────────────────────────────────
--
-- The episode row is part of the RSS list and stays. Trashing moves the stored
-- copy's object from `audio_object_id` to `trashed_audio_object_id`, so every
-- existing query simply sees an episode that is not stored — and billing,
-- which counts `audio_object_id`, stops counting it. The new column is a real
-- foreign key so the storage sweep still treats the object as referenced.

ALTER TABLE podcast_episodes
    ADD COLUMN trashed_audio_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL,
    ADD COLUMN trashed_at  TIMESTAMPTZ,
    ADD COLUMN trashed_by  UUID REFERENCES users (id) ON DELETE SET NULL,
    ADD COLUMN trash_batch UUID;
CREATE INDEX podcast_episodes_trashed_idx
    ON podcast_episodes (trashed_at) WHERE trashed_at IS NOT NULL;

-- ── Restoring charges the days spent in the trash ───────────────────────────
--
-- The trash is free unless the item comes back; a restore charges the days it
-- waited, so trashing everything before the daily billing run and restoring it
-- after gains nothing (plan §2 item 12).

ALTER TABLE credit_ledger DROP CONSTRAINT IF EXISTS credit_ledger_entry_type_check;
ALTER TABLE credit_ledger
    ADD CONSTRAINT credit_ledger_entry_type_check
    CHECK (entry_type IN ('welcome_grant', 'grant', 'storage_charge', 'topup',
                          'adjustment', 'narration_charge', 'trash_restore_charge'));
ALTER TABLE credit_ledger
    ADD CONSTRAINT credit_ledger_trash_restore_check
    CHECK (entry_type <> 'trash_restore_charge' OR amount_micro <= 0);

-- One row per restore, for the charge above and for the instance admin's view
-- of families that restore suspiciously often.
CREATE TABLE trash_restores (
    id            BIGSERIAL   PRIMARY KEY,
    family_id     UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    user_id       UUID        REFERENCES users (id) ON DELETE SET NULL,
    media_kind    TEXT        NOT NULL,
    item_id       UUID        NOT NULL,
    size_bytes    BIGINT      NOT NULL,
    days_in_trash INTEGER     NOT NULL,
    charged_micro BIGINT      NOT NULL,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX trash_restores_family_idx ON trash_restores (family_id, created_at DESC);
