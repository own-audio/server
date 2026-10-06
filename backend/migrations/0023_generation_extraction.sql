-- Migration 0023: Phase 3 extraction artifacts
-- ─────────────────────────────────────────────────
-- Real EPUB/MOBI/TXT extraction (backend/src/audiobook_gen/extract.rs)
-- persists its output so Phase 5 (LLM preprocessing) can read it back
-- instead of re-parsing the raw upload.

ALTER TABLE generation_jobs
    ADD COLUMN extracted_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL,
    ADD COLUMN hints_object_id     UUID REFERENCES media_objects (id) ON DELETE SET NULL;
