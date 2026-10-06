-- Migration 0057: in-app content reports
-- ───────────────────────────────────────
-- Required by Google Play twice over for the Android app (see
-- audio2-android-book/PLAY_COMPLIANCE.md): the AI-Generated Content policy
-- wants an in-app way to report offensive AI output, and the UGC policy wants
-- the same for content shared between accounts. One mechanism serves both.
--
-- Family-scoped on purpose. audio2 is self-hosted, so there is no central
-- moderator to escalate to — the family admin is the moderator, which is also
-- the honest answer for a private family library.

CREATE TABLE content_reports (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id        UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    -- Kept if the reporter later leaves or is deleted: an open report should
    -- not vanish because of that.
    reporter_user_id UUID        REFERENCES users (id) ON DELETE SET NULL,
    media_kind       TEXT        NOT NULL
                                 CHECK (media_kind IN ('audiobook', 'podcast', 'music')),
    -- Deliberately not a foreign key: the three media kinds live in different
    -- tables, and a report should outlive the item it is about.
    item_id          UUID        NOT NULL,
    reason           TEXT        NOT NULL
                                 CHECK (reason IN ('offensive', 'inaccurate',
                                                   'not_for_children', 'other')),
    note             TEXT,
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at      TIMESTAMPTZ,
    resolved_by      UUID        REFERENCES users (id) ON DELETE SET NULL
);

-- The admin list: open reports for one family, newest first.
CREATE INDEX content_reports_family_open_idx
    ON content_reports (family_id, created_at DESC)
    WHERE resolved_at IS NULL;
