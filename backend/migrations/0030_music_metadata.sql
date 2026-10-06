-- Migration 0030: MusicBrainz identity columns on music_tracks
-- ──────────────────────────────────────────────────────────
-- Nullable MBIDs so a track can be linked to a MusicBrainz recording/
-- release/artist once identified via the metadata search+apply endpoints.
-- No new artist/album tables — artist/album stay free-text on music_tracks
-- (see 0012); this only adds a stable external identity for tracks a user
-- has actually matched.

ALTER TABLE music_tracks
    ADD COLUMN musicbrainz_recording_id TEXT,
    ADD COLUMN musicbrainz_release_id   TEXT,
    ADD COLUMN musicbrainz_artist_id    TEXT;
