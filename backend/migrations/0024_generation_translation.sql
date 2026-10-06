-- Migration 0024: Phase 4 translation artifacts
-- ─────────────────────────────────────────────────
-- Real Google Translate output (backend/src/audiobook_gen/translate.rs),
-- persisted so Phase 5 (LLM preprocessing) reads whichever of
-- extracted_object_id / translated_object_id represents the final-language
-- text for this job.

ALTER TABLE generation_jobs
    ADD COLUMN translated_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL;
