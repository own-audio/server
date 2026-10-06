-- Until now a Wikidata rate-limit or a dropped connection was recorded exactly
-- like "no source has a picture of this artist", and never retried — one bad
-- moment and an artist stayed blank for good. The lookup tells the two apart
-- now, but the rows written while it could not are still here and still wrong.
--
-- Only automatic misses go. A picture we hold is kept, and so is a row the user
-- set themselves.
DELETE FROM music_artist_images
WHERE image_object_id IS NULL AND NOT is_user_set;
