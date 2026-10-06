-- Where a track's lyrics came from: the file's own tag, or the user typing them.
--
-- The same distinction `music_artist_images.is_user_set` makes, for the same
-- reason. `lyrics` today is three-state (NULL never checked, '' checked and
-- absent, text = lyrics) and nothing re-reads it once set — but that is a
-- property of the current code, not of the data. The moment anything re-reads
-- tags (the rescan flow is the obvious candidate), a user's own typing would be
-- silently overwritten by whatever the file happens to contain, which is
-- usually nothing.
--
-- NULL means "not user-set": either never checked, or read from the file.
ALTER TABLE music_tracks
    ADD COLUMN lyrics_source TEXT;
