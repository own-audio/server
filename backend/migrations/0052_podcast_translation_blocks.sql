-- Migration 0052: Phase 4 of docs/podcast-translation-plan.md — per-block narration
-- tracking for podcast episode translations, mirroring generation_blocks (migration 0022)
-- so a retried job resumes instead of re-paying for blocks already narrated.
CREATE TABLE podcast_translation_blocks (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    translation_id  UUID    NOT NULL REFERENCES podcast_episode_translations (id) ON DELETE CASCADE,
    position        INTEGER NOT NULL,
    text            TEXT    NOT NULL,
    char_count      INTEGER NOT NULL,
    -- 'pending' | 'complete' | 'failed'
    tts_status      TEXT    NOT NULL DEFAULT 'pending',
    audio_object_id UUID    REFERENCES media_objects (id) ON DELETE SET NULL,
    tts_attempts    INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,

    CONSTRAINT podcast_translation_blocks_position_uq UNIQUE (translation_id, position)
);

CREATE INDEX podcast_translation_blocks_translation_idx ON podcast_translation_blocks (translation_id, position);
