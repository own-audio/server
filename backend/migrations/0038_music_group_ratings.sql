-- Migration 0038: ratings for albums and artists, not just songs.
--
-- 0036 rated songs only, on the reasoning that no mainstream client rates a
-- group. That was wrong: Amperfy has an "Album Rating Sync" and sends
-- `setRating` with an album id, which failed with error 70 because the id
-- resolved to no track.
--
-- Keyed by name, exactly as `music_group_stars` is, and for the same reason —
-- `music_tracks` stores artist and album as denormalized strings with no rows
-- to reference.
CREATE TABLE music_group_ratings (
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        TEXT        NOT NULL CHECK (kind IN ('artist', 'album')),
    artist_name TEXT        NOT NULL,
    album_name  TEXT        NOT NULL DEFAULT '',
    rating      SMALLINT    NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, kind, artist_name, album_name),
    CONSTRAINT music_group_ratings_album_check CHECK (
        (kind = 'artist' AND album_name = '') OR
        (kind = 'album'  AND album_name <> '')
    )
);
