-- Migration 0051: Phase 3 of docs/podcast-translation-plan.md — the data model for
-- translating a podcast episode from its feed-published transcript. Personal-use only
-- (plan §0): a row belongs to the requesting user, stays in their household's library,
-- and is never distributed beyond it.
CREATE TABLE podcast_episode_translations (
    id                        UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    episode_id                UUID        NOT NULL REFERENCES podcast_episodes (id) ON DELETE CASCADE,
    user_id                   UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    family_id                 UUID        NOT NULL REFERENCES families (id) ON DELETE CASCADE,
    voice_profile_id          TEXT        NOT NULL REFERENCES voice_profiles (id),
    source_language           TEXT        NOT NULL,
    target_language           TEXT        NOT NULL,
    -- 'translating' | 'narrating' | 'assembling' | 'complete' | 'failed'
    status                    TEXT        NOT NULL DEFAULT 'translating'
        CHECK (status IN ('translating', 'narrating', 'assembling', 'complete', 'failed')),
    char_count                INTEGER     NOT NULL,
    quoted_price_cents        INTEGER     NOT NULL,
    actual_cost_cents         INTEGER,
    charged_price_cents       INTEGER,
    currency                  TEXT        NOT NULL DEFAULT 'USD',
    translated_text_object_id UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    audio_object_id           UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    error_reason              TEXT,
    created_at                TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at                TIMESTAMPTZ NOT NULL DEFAULT now(),

    -- A second request for the same (episode, language, voice) returns the existing row
    -- instead of double-charging.
    CONSTRAINT podcast_episode_translations_uq UNIQUE (episode_id, user_id, target_language, voice_profile_id)
);

CREATE INDEX podcast_episode_translations_episode_idx ON podcast_episode_translations (episode_id, user_id);
