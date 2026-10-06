-- Cover art is generated from a fixed Gemini prompt today, which produced
-- dark, dramatic "novel cover" art for every book regardless of subject —
-- a blog post about self-hosting costs came back looking like a horror film.
-- Let the caller steer it, or opt out of a cover entirely.
--
-- IF NOT EXISTS because the columns were added by hand on the dev database
-- before this file got its final number.
ALTER TABLE generation_jobs
    ADD COLUMN IF NOT EXISTS cover_mode   TEXT NOT NULL DEFAULT 'auto',
    ADD COLUMN IF NOT EXISTS cover_prompt TEXT;

ALTER TABLE generation_jobs DROP CONSTRAINT IF EXISTS generation_jobs_cover_mode_check;
ALTER TABLE generation_jobs
    ADD CONSTRAINT generation_jobs_cover_mode_check
    CHECK (cover_mode IN ('auto', 'none', 'custom'));
