-- Which disc of a multi-disc album a track is on. Read from the file's tags (Vorbis
-- DISCNUMBER/DISCTOTAL, ID3 TPOS, MP4 disk); `disc_checked` marks a track whose file was read
-- for it, so tracks uploaded before this are read once, not on every request.
ALTER TABLE music_tracks_all
    ADD COLUMN disc_number  INTEGER CHECK (disc_number > 0),
    ADD COLUMN disc_total   INTEGER CHECK (disc_total > 0),
    ADD COLUMN disc_checked BOOLEAN NOT NULL DEFAULT false;

CREATE OR REPLACE VIEW music_tracks AS
    SELECT * FROM music_tracks_all WHERE trashed_at IS NULL;
