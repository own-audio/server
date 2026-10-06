-- Italian, French and Spanish were offered as languages to narrate or translate into
-- (SUPPORTED_LANGUAGES), but had no voice, so the voice step was empty for them.
-- Same tier and voice identities as 0025, verified live against
-- GET https://texttospeech.googleapis.com/v1/voices on 2026-09-30, with a test synthesis.
INSERT INTO voice_profiles (id, provider, display_name, language, gender, cost_per_million_chars_cents) VALUES
    ('fr-FR-Chirp3-HD-Aoede',  'google', 'Camille', 'fr', 'female', 3000),
    ('fr-FR-Chirp3-HD-Charon', 'google', 'Lucas',   'fr', 'male',   3000),
    ('it-IT-Chirp3-HD-Aoede',  'google', 'Giulia',  'it', 'female', 3000),
    ('it-IT-Chirp3-HD-Charon', 'google', 'Marco',   'it', 'male',   3000),
    ('es-ES-Chirp3-HD-Aoede',  'google', 'Lucía',   'es', 'female', 3000),
    ('es-ES-Chirp3-HD-Charon', 'google', 'Javier',  'es', 'male',   3000)
ON CONFLICT (id) DO NOTHING;
