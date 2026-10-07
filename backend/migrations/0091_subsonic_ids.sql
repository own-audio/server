-- Subsonic album and artist ids are name-based UUIDs of (viewer, artist,
-- album), so no index can find one: every lookup used to aggregate the whole
-- catalog to recompute them. This remembers which names an id stands for, and
-- the index below finds an album's or artist's tracks by those names.
CREATE TABLE subsonic_ids (
    user_id UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    id      UUID NOT NULL,
    artist  TEXT NOT NULL,
    album   TEXT,             -- NULL for an artist id
    PRIMARY KEY (user_id, id)
);

-- The grouping expressions of db/subsonic.rs, verbatim; music_tracks is the
-- view of this table without the trash.
CREATE INDEX music_tracks_subsonic_group_idx ON music_tracks_all (
    (COALESCE(NULLIF(trim(artist), ''), 'Unknown Artist')),
    (COALESCE(NULLIF(trim(album), ''), 'Unknown Album'))
) WHERE trashed_at IS NULL;
