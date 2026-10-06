-- A playlist made by the smart-playlist generator ("Ask for a playlist" or a
-- ready-made one) and saved as an ordinary playlist. generated_at marks it so
-- clients can list what was generated; kept_at is set when the listener says
-- they want to keep it, so the ones nobody confirmed can be offered for
-- clearing out. Otherwise the playlist is like any other.
ALTER TABLE music_playlists_all ADD COLUMN generated_at timestamptz;
ALTER TABLE music_playlists_all ADD COLUMN kept_at timestamptz;

-- The view was created with SELECT *, which Postgres expands at creation time;
-- recreate it so it carries the new columns.
CREATE OR REPLACE VIEW music_playlists AS
    SELECT * FROM music_playlists_all WHERE trashed_at IS NULL;
