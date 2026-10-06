-- Migration 0025: fix voice_profiles seed data
-- ─────────────────────────────────────────────────
-- The voices seeded in 0022 were never checked against Google Cloud TTS's
-- actual catalog. 'en-US-Neural2-D' happens to be real (confirmed live in
-- Phase 6), but 'cs-CZ-Neural2-A' and 'de-DE-Neural2-F' do not exist —
-- Neural2 was never released for Czech at all, and German Neural2 only
-- ships as -G/-H, not -F. This caused every narration request for a Czech
-- book to fail with a production job stuck at "narrating" -> "failed".
--
-- Fix: replace the seed with voices verified live against
-- GET https://texttospeech.googleapis.com/v1/voices, using Chirp3-HD (the
-- current top-quality tier, $30/1M chars vs $16 for Neural2) so narration
-- quality is as good as Google currently offers. Two voices (one male, one
-- female) per language, using the same character names across languages
-- since Chirp3-HD voice identities are shared per-language.

INSERT INTO voice_profiles (id, provider, display_name, language, gender, cost_per_million_chars_cents) VALUES
    ('cs-CZ-Chirp3-HD-Aoede',  'google', 'Tereza', 'cs', 'female', 3000),
    ('cs-CZ-Chirp3-HD-Charon', 'google', 'Karel',  'cs', 'male',   3000),
    ('en-US-Chirp3-HD-Aoede',  'google', 'Sarah',  'en', 'female', 3000),
    ('en-US-Chirp3-HD-Charon', 'google', 'Marcus', 'en', 'male',   3000),
    ('de-DE-Chirp3-HD-Aoede',  'google', 'Lena',   'de', 'female', 3000),
    ('de-DE-Chirp3-HD-Charon', 'google', 'Klaus',  'de', 'male',   3000);

-- Re-point jobs that reference a now-removed voice to its Chirp3-HD
-- replacement of the same gender, so nothing is orphaned and the one job
-- that failed against the invalid Czech voice can be retried in place.
UPDATE generation_jobs
SET voice_profile_id = 'cs-CZ-Chirp3-HD-Aoede'
WHERE voice_profile_id = 'cs-CZ-Neural2-A';

UPDATE generation_jobs
SET voice_profile_id = 'en-US-Chirp3-HD-Charon'
WHERE voice_profile_id = 'en-US-Neural2-D';

-- 'cs-CZ-Neural2-A' and 'de-DE-Neural2-F' never existed on Google's side.
-- 'en-US-Neural2-D' is real but retired in favor of the higher-quality,
-- consistently-named Chirp3-HD tier used for the other two languages.
DELETE FROM voice_profiles WHERE id IN (
    'cs-CZ-Neural2-A',
    'de-DE-Neural2-F',
    'en-US-Neural2-D'
);
