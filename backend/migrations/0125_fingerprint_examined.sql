-- When the fingerprint job last looked at a file with the health checks of
-- migration 0124 (does it decode, how much, how loud). NULL for every row made
-- before them: the sweep takes those again, 200 every two minutes in the
-- background, so a library fingerprinted by an older server is examined once
-- without anyone asking. The fingerprints come out the same, byte for byte.
ALTER TABLE track_fingerprints ADD COLUMN examined_at TIMESTAMPTZ;
