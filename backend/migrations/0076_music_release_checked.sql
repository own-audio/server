-- docs/album-artist-plan.md A2: tracks identified before the metadata service returned the
-- release's artist credit have no explicit album artist yet. The `music_release_backfill` sweep
-- re-reads each one's release once; this marks the ones it has, so a recording the mirror
-- cannot resolve isn't asked about forever.
ALTER TABLE music_tracks ADD COLUMN release_checked_at TIMESTAMPTZ;
