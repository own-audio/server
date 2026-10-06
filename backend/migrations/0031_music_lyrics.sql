-- Migration 0031: lyrics cache column on music_tracks
-- ─────────────────────────────────────────────────
-- NULL = never checked; '' = checked, the file had no embedded lyrics tag;
-- non-empty = the parsed lyrics text. Lazily populated by GET
-- /tracks/{id}/lyrics on first request, not on upload — avoids parsing every
-- uploaded file up front for a field most tracks won't have.

ALTER TABLE music_tracks
    ADD COLUMN lyrics TEXT;
