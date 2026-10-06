-- Migration 0014: add audio notes to bookmarks
-- Allows users to attach a recorded audio message to a bookmark.
ALTER TABLE bookmarks
  ADD COLUMN audio_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL;
