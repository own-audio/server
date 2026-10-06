-- Migration 0029: record the owning family on a generation job
-- ─────────────────────────────────────────────────
-- Generation artifacts (source, extracted text, TTS blocks, the assembled
-- book) are stored under the family prefix `f/{family_id}/...` like every
-- other object. The job row is the only place the pipeline stages can read
-- that from — they run in the worker with no request context.
--
-- Nullable: jobs created before this migration keep their unprefixed
-- `generation/{job_id}/...` keys, which still resolve because reads use the
-- key stored in media_objects, not a recomputed one.

ALTER TABLE generation_jobs
    ADD COLUMN family_id UUID REFERENCES families (id) ON DELETE SET NULL;
