-- Migration 0037: drop fetched artist images so they are re-fetched with the
-- licence credit burned in.
--
-- Images stored before the watermark existed carry no visible attribution, and
-- the Subsonic surface now serves them to third-party clients that render the
-- picture and nothing else. Rather than leave uncredited copies in place, the
-- rows are removed and the next request re-fetches and re-watermarks.
--
-- `is_user_set` rows are deliberately untouched: a picture the user chose is
-- theirs, carries no Commons licence, and must not be watermarked or discarded.
--
-- Deleting the row (rather than nulling the object) is what re-enables the
-- lookup — a row with a NULL object means "asked, nothing found" and would
-- suppress the re-fetch this migration exists to trigger. The orphaned objects
-- in storage are left behind, as elsewhere in this schema.
DELETE FROM music_artist_images
WHERE NOT is_user_set;
