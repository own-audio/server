-- Migration 0022: audiobook generation (voice profiles, generation jobs/chapters/blocks)
-- ─────────────────────────────────────────────────
-- Phase 2 of docs/audiobook-generation-plan.md: upload → translate → LLM
-- preprocess → TTS → assemble pipeline. `generation_jobs` drives progress
-- through `stage`; `generation_chapters`/`generation_blocks` are populated
-- once real preprocessing (Phase 5) and TTS (Phase 6) land — for now the
-- stub pipeline only tracks per-chapter block counters for progress display.
--
-- On success, a generation_job publishes into the existing audiobook_books /
-- audiobook_files / audiobook_chapters tables (Phase 7) rather than owning a
-- parallel library — `audiobook_book_id` records that link.

CREATE TABLE voice_profiles (
    id                            TEXT    PRIMARY KEY, -- provider voice id, e.g. 'en-US-Neural2-D'
    provider                      TEXT    NOT NULL DEFAULT 'google',
    display_name                  TEXT    NOT NULL,
    language                      TEXT    NOT NULL,
    gender                        TEXT    NOT NULL CHECK (gender IN ('male', 'female', 'neutral')),
    preview_object_id             UUID    REFERENCES media_objects (id) ON DELETE SET NULL,
    cost_per_million_chars_cents  INTEGER NOT NULL
);

INSERT INTO voice_profiles (id, provider, display_name, language, gender, cost_per_million_chars_cents) VALUES
    ('cs-CZ-Neural2-A', 'google', 'Tereza', 'cs', 'female', 1600),
    ('en-US-Neural2-D', 'google', 'Marcus', 'en', 'male', 1600),
    ('de-DE-Neural2-F', 'google', 'Lena',   'de', 'female', 1600);

CREATE TABLE generation_jobs (
    id                    UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id               UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title                 TEXT        NOT NULL,
    source_format         TEXT        NOT NULL CHECK (source_format IN ('epub', 'mobi', 'txt')),
    source_language       TEXT        NOT NULL,
    target_language       TEXT        NOT NULL,
    voice_profile_id      TEXT        NOT NULL REFERENCES voice_profiles (id),
    output_mode           TEXT        NOT NULL CHECK (output_mode IN ('single_m4b', 'multi_file')),
    -- 'extracting' | 'translating' | 'preprocessing' | 'narrating' | 'assembling' | 'complete' | 'failed'
    stage                 TEXT        NOT NULL DEFAULT 'extracting',
    -- Ordered stage list this specific job runs through (translation omitted when source == target).
    stages                JSONB       NOT NULL,
    raw_object_id         UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    final_object_id       UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    estimated_char_count  INTEGER     NOT NULL,
    quoted_price_cents    INTEGER     NOT NULL,
    actual_cost_cents     INTEGER,
    charged_price_cents   INTEGER,
    currency              TEXT        NOT NULL DEFAULT 'USD',
    error_reason          TEXT,
    -- Set by Phase 7 once the finished audiobook is published into the library.
    audiobook_book_id     UUID        REFERENCES audiobook_books (id) ON DELETE SET NULL,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at            TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX generation_jobs_user_id_idx ON generation_jobs (user_id);

CREATE TABLE generation_chapters (
    id                 UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    generation_job_id  UUID    NOT NULL REFERENCES generation_jobs (id) ON DELETE CASCADE,
    position           INTEGER NOT NULL,
    title              TEXT    NOT NULL,
    -- 'pending' | 'in_progress' | 'complete' | 'failed'
    tts_status         TEXT    NOT NULL DEFAULT 'pending',
    blocks_total       INTEGER NOT NULL DEFAULT 0,
    blocks_done        INTEGER NOT NULL DEFAULT 0,
    audio_object_id    UUID    REFERENCES media_objects (id) ON DELETE SET NULL,
    duration_secs      INTEGER,

    CONSTRAINT generation_chapters_job_position_uq UNIQUE (generation_job_id, position)
);

CREATE INDEX generation_chapters_job_id_idx ON generation_chapters (generation_job_id, position);

CREATE TABLE generation_blocks (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    chapter_id      UUID    NOT NULL REFERENCES generation_chapters (id) ON DELETE CASCADE,
    position        INTEGER NOT NULL,
    text            TEXT    NOT NULL,
    char_count      INTEGER NOT NULL,
    -- 'pending' | 'in_progress' | 'complete' | 'failed'
    tts_status      TEXT    NOT NULL DEFAULT 'pending',
    audio_object_id UUID    REFERENCES media_objects (id) ON DELETE SET NULL,
    tts_attempts    INTEGER NOT NULL DEFAULT 0,
    last_error      TEXT,

    CONSTRAINT generation_blocks_chapter_position_uq UNIQUE (chapter_id, position)
);

CREATE INDEX generation_blocks_chapter_id_idx ON generation_blocks (chapter_id, position);
