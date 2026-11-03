-- Quarantine: a file no reader can open, one that does not decode, a stub of a
-- second or two, or one silent for its first two minutes (a DRM-protected rip)
-- is set aside by the scan or the fingerprint job. Like a missing file it
-- leaves every list and player (the view), but it stays, with its reason, to
-- be listed and released at /api/v2/library/quarantine. A file that changes on
-- disk is read again and released if it now opens.
ALTER TABLE music_tracks_all
    ADD COLUMN quarantined_at    TIMESTAMPTZ,
    ADD COLUMN quarantine_reason TEXT;

CREATE OR REPLACE VIEW music_tracks AS
    SELECT * FROM music_tracks_all
     WHERE trashed_at IS NULL AND missing_at IS NULL AND quarantined_at IS NULL;

CREATE INDEX music_tracks_quarantined_idx
    ON music_tracks_all (family_id, quarantined_at)
    WHERE quarantined_at IS NOT NULL;
