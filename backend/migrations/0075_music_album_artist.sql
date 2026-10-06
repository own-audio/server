-- An album is a release, not an artist (docs/album-artist-plan.md).
--
-- Albums were grouped by track artist + album name, so one guest on one track
-- ("George Ezra feat. First Aid Kit") split an album in two, and a compilation
-- became one album per artist.
--
-- `album_artist` holds only an explicit value: the file's album-artist tag, a
-- compilation, a MusicBrainz release, or a manual edit. NULL means "derive it
-- from the track artist without its guests" at query time
-- (`db::music::ALBUM_ARTIST_SQL`), so an edit to the track artist carries the
-- album along and existing rows need no backfill.
ALTER TABLE music_tracks
    ADD COLUMN album_artist TEXT,
    ADD COLUMN is_compilation BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN musicbrainz_release_group_id TEXT;
