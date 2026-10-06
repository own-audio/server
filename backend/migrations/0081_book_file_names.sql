-- Migration 0081: renumber doubly numbered book files
-- ─────────────────────────────────────────────────────
-- The first default names (migration 0078) put the position in front of a file
-- title that already started with it: `06 - 06 - Chapter.mp3`. The naming is
-- fixed in code; these files lose their name here and get the right one the
-- next time the sync feed sees their book (filesync::paths::ensure_book_files).
-- Only names showing the doubled number are touched. No file had been put in
-- the own.audio folder by hand yet when this ran, so every such name is a
-- default one.
UPDATE audiobook_files SET relative_path = NULL
 WHERE relative_path ~ '^([0-9]+) - \1 - ';
