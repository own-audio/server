-- Migration 0026: capture the book's author for generation jobs
-- ─────────────────────────────────────────────────
-- Was never collected — generated books were always published with
-- author = NULL. Needed so the cover art can render a real "by <author>"
-- line instead of leaving it off, and so the published book's library
-- metadata is correct too (see backend/src/audiobook_gen/assemble.rs).

ALTER TABLE generation_jobs ADD COLUMN author TEXT;
