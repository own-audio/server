-- Where an identified track's MusicBrainz identity came from, and what an
-- identify replaced, so it can be undone. One row per identified track; a track
-- never identified has none. Kept beside music_tracks_all rather than as columns
-- on it: the music_tracks view would have to be recreated, and nothing that
-- reads tracks needs this.
CREATE TABLE music_track_identity (
    track_id      UUID        PRIMARY KEY REFERENCES music_tracks_all (id) ON DELETE CASCADE,
    -- 'identify': matched through MusicBrainz by this server (a client's apply,
    -- or a batch run). 'mbid_tag': the file itself carries MusicBrainz IDs, as a
    -- file saved by MusicBrainz Picard does; the file stays the authority.
    source        TEXT        NOT NULL CHECK (source IN ('identify', 'mbid_tag')),
    identified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    -- The batch run that made the match, so a run can be reverted as one. No
    -- foreign key: runs belong to an edition's own schema.
    run_id        UUID,
    -- The track's values before it was first identified, restored by a revert.
    -- NULL for tracks identified before this table existed: what they were
    -- before was never kept.
    before        JSONB
);

CREATE INDEX music_track_identity_run_idx ON music_track_identity (run_id)
    WHERE run_id IS NOT NULL;

-- Tracks matched before this migration are identified, with nothing to undo.
INSERT INTO music_track_identity (track_id, source, identified_at, before)
SELECT id, 'identify', updated_at, NULL
  FROM music_tracks_all
 WHERE musicbrainz_recording_id IS NOT NULL;
