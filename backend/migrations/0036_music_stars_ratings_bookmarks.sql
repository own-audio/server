-- Migration 0036: favourites, ratings, and bookmarks for music.
--
-- Added for the OpenSubsonic surface (`star`/`unstar`/`setRating`/
-- `getStarred2`/`getBookmarks`), which every Subsonic client offers as a
-- first-class button. Nothing else in audio2 exposes these for music yet;
-- the clients' own favourite icons currently live in local state only.

-- ── stars ────────────────────────────────────────────────────────────────────
-- Songs are real rows, so this is a plain FK'd join table.
CREATE TABLE music_track_stars (
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id   UUID        NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);

-- Artists and albums have no rows to point at — `music_tracks` stores them as
-- denormalized strings — so they are keyed by name, exactly as
-- `music_artist_images` already keys artists. Storing the synthetic UUID v5
-- from `subsonic::ids` instead would tie a user's favourites to that
-- namespace, and silently lose them all if it ever changed.
--
-- `album_name` is '' rather than NULL for an artist star so it can sit in the
-- primary key; NULL would make every artist star distinct from every other.
CREATE TABLE music_group_stars (
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    kind        TEXT        NOT NULL CHECK (kind IN ('artist', 'album')),
    artist_name TEXT        NOT NULL,
    album_name  TEXT        NOT NULL DEFAULT '',
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, kind, artist_name, album_name),
    CONSTRAINT music_group_stars_album_check CHECK (
        (kind = 'artist' AND album_name = '') OR
        (kind = 'album'  AND album_name <> '')
    )
);

-- ── ratings ──────────────────────────────────────────────────────────────────
-- Songs only. Subsonic's `setRating` accepts an album or artist id on some
-- servers, but no mainstream client sends one, and rating a denormalized
-- string group is not worth a second table.
--
-- 1..5: rating 0 means "unrate" in the protocol and is stored as the absence
-- of a row, not as a zero.
CREATE TABLE music_track_ratings (
    user_id    UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id   UUID        NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    rating     SMALLINT    NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);

-- ── bookmarks ────────────────────────────────────────────────────────────────
-- `bookmarks` was built for podcasts and audiobooks with a CHECK enforcing
-- exactly one of two targets. Music becomes a third target rather than a
-- fourth table, so `getBookmarks` can return one list across all media the way
-- the protocol expects.
ALTER TABLE bookmarks
    ADD COLUMN track_id UUID REFERENCES music_tracks (id) ON DELETE CASCADE;

ALTER TABLE bookmarks DROP CONSTRAINT bookmarks_target_check;

ALTER TABLE bookmarks ADD CONSTRAINT bookmarks_target_check CHECK (
    (episode_id IS NOT NULL)::int +
    (book_id    IS NOT NULL)::int +
    (track_id   IS NOT NULL)::int = 1
);

CREATE INDEX bookmarks_user_track_idx ON bookmarks (user_id, track_id);
