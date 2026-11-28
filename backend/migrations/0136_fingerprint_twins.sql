-- Identify finds a song's twin — the same recording in another file that is
-- already identified — by its fingerprint, among songs of about the same length.
CREATE INDEX track_fingerprints_duration_idx ON track_fingerprints (duration_secs)
    WHERE fingerprint IS NOT NULL;
