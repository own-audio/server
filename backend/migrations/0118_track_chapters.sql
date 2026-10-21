-- Chapter marks of a music track (docs/concerts-plan.md C1): a whole concert as
-- one file, a mark where each song starts. Read from the file itself, like a
-- folder book's (0117), so the file stays the truth and a new file replaces them.
CREATE TABLE music_track_chapters (
    track_id UUID    NOT NULL REFERENCES music_tracks_all (id) ON DELETE CASCADE,
    position INTEGER NOT NULL,
    title    TEXT    NOT NULL,
    start_ms BIGINT  NOT NULL,
    PRIMARY KEY (track_id, position)
);

-- When the file's marks were last read. NULL: not yet, or the file changed
-- since; the folder scanner reads those and sets it, so a file is probed once.
-- Set by hand (`PUT /api/v2/tracks/{id}/chapters`): the file's marks no longer
-- replace them.
ALTER TABLE music_tracks_all
    ADD COLUMN chapters_read_at TIMESTAMPTZ,
    ADD COLUMN chapters_edited_at TIMESTAMPTZ;

-- New chapters are a change to the track for offline clients (0109).
CREATE TRIGGER music_track_chapters_change AFTER INSERT OR UPDATE OR DELETE ON music_track_chapters
    FOR EACH ROW EXECUTE FUNCTION change_log_part('track_genre');

-- Concerts are albums whose release group is `live`: the Concerts list filters on it.
CREATE INDEX release_groups_kind_idx ON release_groups (family_id, kind) WHERE kind <> 'album';
