-- A song's acoustic fingerprint (Chromaprint, made by ffmpeg from the first two
-- minutes of its audio), in the compressed base64 form AcoustID takes. What
-- identifies a recording whatever its tags say, and what tells two files of
-- one recording apart from two songs with one title.
--
-- Kept per file: `audio_object_id` and `size_bytes` are the file it was made
-- from, so a replaced or changed file is fingerprinted again. A file that
-- cannot be decoded keeps its row with `fingerprint` NULL and the reason in
-- `error` (a damaged download), and is not tried again until it changes.
CREATE TABLE track_fingerprints (
    track_id        UUID        PRIMARY KEY REFERENCES music_tracks_all (id) ON DELETE CASCADE,
    audio_object_id UUID        NOT NULL,
    size_bytes      BIGINT,
    fingerprint     TEXT,
    duration_secs   INTEGER,
    error           TEXT,
    computed_at     TIMESTAMPTZ NOT NULL DEFAULT now()
);
