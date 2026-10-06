-- Migration 0019: family-shared content + per-member access control
-- ──────────────────────────────────────────────────────────────────
-- Privacy model (decision D1):
--   • Content keeps its `user_id` owner and gains a NULLABLE `family_id`.
--       family_id IS NULL  ⇒ PRIVATE to the owner. Invisible to everyone
--                            else, including family admins.
--       family_id = X      ⇒ SHARED with family X, subject to the policy /
--                            grant rules below.
--   • No backfill: every existing item stays private, so upgrading never
--     exposes a library that was personal before families existed.
--   • ON DELETE SET NULL: if a family is deleted, its content falls back to
--     private rather than being orphaned or cascade-deleted.

ALTER TABLE audiobook_books       ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;
ALTER TABLE podcast_feeds         ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;
ALTER TABLE music_tracks          ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;
ALTER TABLE music_playlists       ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;
ALTER TABLE audiobook_collections ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;
ALTER TABLE audiobook_series      ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;

-- Partial indexes: only shared rows are ever looked up by family.
CREATE INDEX audiobook_books_family_idx       ON audiobook_books       (family_id) WHERE family_id IS NOT NULL;
CREATE INDEX podcast_feeds_family_idx         ON podcast_feeds         (family_id) WHERE family_id IS NOT NULL;
CREATE INDEX music_tracks_family_idx          ON music_tracks          (family_id) WHERE family_id IS NOT NULL;
CREATE INDEX music_playlists_family_idx       ON music_playlists       (family_id) WHERE family_id IS NOT NULL;
CREATE INDEX audiobook_collections_family_idx ON audiobook_collections (family_id) WHERE family_id IS NOT NULL;
CREATE INDEX audiobook_series_family_idx      ON audiobook_series      (family_id) WHERE family_id IS NOT NULL;

-- ── Per-member default policy ────────────────────────────────────────────────
-- The starting point for a member on a given media kind. A missing row means
-- 'allow_all', so members are unrestricted until a family admin says otherwise
-- and nobody's experience changes on upgrade.
CREATE TABLE member_media_policy (
    family_id  UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    user_id    UUID        NOT NULL REFERENCES users (id)    ON DELETE CASCADE,
    media_kind TEXT        NOT NULL
                           CHECK (media_kind IN ('audiobook', 'podcast', 'music')),
    policy     TEXT        NOT NULL DEFAULT 'allow_all'
                           CHECK (policy IN ('allow_all', 'deny_all')),
    updated_by UUID        REFERENCES users (id) ON DELETE SET NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (family_id, user_id, media_kind)
);

-- ── Item-level overrides ─────────────────────────────────────────────────────
-- An explicit allow/deny for one item, overriding the member's default policy.
-- `item_id` is deliberately not a FK: it addresses one of several content
-- tables depending on `media_kind`. Rows are cleaned up when an item is
-- unshared or deleted (see db::access).
CREATE TABLE content_grants (
    id         UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    family_id  UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    user_id    UUID        NOT NULL REFERENCES users (id)    ON DELETE CASCADE,
    media_kind TEXT        NOT NULL
                           CHECK (media_kind IN ('audiobook', 'podcast', 'music')),
    item_id    UUID        NOT NULL,
    effect     TEXT        NOT NULL
                           CHECK (effect IN ('allow', 'deny')),
    granted_by UUID        REFERENCES users (id) ON DELETE SET NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT content_grants_unique UNIQUE (user_id, media_kind, item_id)
);

CREATE INDEX content_grants_lookup_idx ON content_grants (user_id, media_kind, item_id);
CREATE INDEX content_grants_item_idx   ON content_grants (media_kind, item_id);
CREATE INDEX content_grants_family_idx ON content_grants (family_id);

-- ── The visibility rule, in one place ────────────────────────────────────────
-- Every content query and every stream authorization calls this, so the rule
-- cannot drift between the REST API and the Subsonic surface.
--
-- Evaluation order:
--   1. The owner always sees their own content, shared or not.
--   2. Private content (family_id IS NULL) stops there — a family admin has
--      no more access to it than anyone else.
--   3. Shared content is visible to members of that family, gated by the
--      member's item-level grant, falling back to their per-kind default
--      policy, which itself defaults to allow_all.
--
-- Parameters are p_-prefixed so they cannot be mistaken for the columns of
-- the same name inside the subqueries.
CREATE OR REPLACE FUNCTION audio2_can_access(
    p_viewer          UUID,
    p_viewer_family   UUID,
    p_is_family_admin BOOLEAN,
    p_kind            TEXT,
    p_item_id         UUID,
    p_owner_id        UUID,
    p_item_family     UUID
) RETURNS BOOLEAN
LANGUAGE sql
STABLE
AS $$
    SELECT
        p_owner_id = p_viewer
        OR (
            p_item_family IS NOT NULL
            AND p_item_family = p_viewer_family
            AND (
                p_is_family_admin
                OR COALESCE(
                    (SELECT g.effect
                       FROM content_grants g
                      WHERE g.user_id    = p_viewer
                        AND g.media_kind = p_kind
                        AND g.item_id    = p_item_id),
                    CASE WHEN COALESCE(
                                (SELECT p.policy
                                   FROM member_media_policy p
                                  WHERE p.family_id  = p_item_family
                                    AND p.user_id    = p_viewer
                                    AND p.media_kind = p_kind),
                                'allow_all') = 'allow_all'
                         THEN 'allow'
                         ELSE 'deny'
                    END
                ) = 'allow'
            )
        );
$$;
