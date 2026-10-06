-- Migration 0054: podcast-translation-plan.md §0 always scoped a translation to the
-- requesting user's household, not just the requester — 0051's constraint shipped narrower
-- than that (user_id, not family_id), so two family members could each pay to translate the
-- same episode into the same language. Widen the dedup constraint and its supporting index
-- to match the family-scoped app-level checks (find_existing, list_for_episode, etc.).
ALTER TABLE podcast_episode_translations
    DROP CONSTRAINT podcast_episode_translations_uq;

ALTER TABLE podcast_episode_translations
    ADD CONSTRAINT podcast_episode_translations_uq
        UNIQUE (episode_id, family_id, target_language, voice_profile_id);

DROP INDEX IF EXISTS podcast_episode_translations_episode_idx;
CREATE INDEX podcast_episode_translations_episode_idx ON podcast_episode_translations (episode_id, family_id);
