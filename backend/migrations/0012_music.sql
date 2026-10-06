-- Migration 0012: music (tracks, playlists, playlist_tracks)
-- ──────────────────────────────────────────────────────────
-- A music_track is a single song/mp3 owned by one user.
-- A music_playlist groups tracks in a user-defined order.
-- music_playlist_tracks is the many-to-many join table with ordering.

-- ── music_tracks ─────────────────────────────────────────────────────────────
CREATE TABLE music_tracks (
    id               UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id          UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    title            TEXT        NOT NULL,
    artist           TEXT,
    album            TEXT,
    genre            TEXT,
    track_number     INTEGER,
    duration_secs    INTEGER,
    cover_object_id  UUID        REFERENCES media_objects (id) ON DELETE SET NULL,
    audio_object_id  UUID        NOT NULL REFERENCES media_objects (id),
    created_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at       TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX music_tracks_user_id_idx ON music_tracks (user_id);

-- ── music_playlists ──────────────────────────────────────────────────────────
CREATE TABLE music_playlists (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id     UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    name        TEXT        NOT NULL,
    description TEXT,
    cover_object_id UUID    REFERENCES media_objects (id) ON DELETE SET NULL,
    created_at  TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at  TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX music_playlists_user_id_idx ON music_playlists (user_id);

-- ── music_playlist_tracks ────────────────────────────────────────────────────
-- Join table with ordering. `position` determines track order (1-based).
CREATE TABLE music_playlist_tracks (
    id          UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    playlist_id UUID    NOT NULL REFERENCES music_playlists (id) ON DELETE CASCADE,
    track_id    UUID    NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    position    INTEGER NOT NULL,
    added_at    TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT music_playlist_tracks_uq UNIQUE (playlist_id, position)
);

CREATE INDEX music_playlist_tracks_playlist_id_idx ON music_playlist_tracks (playlist_id);

-- ── music_progress ───────────────────────────────────────────────────────────
-- Per-track listen position for resume.
CREATE TABLE music_progress (
    user_id       UUID             NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    track_id      UUID             NOT NULL REFERENCES music_tracks (id) ON DELETE CASCADE,
    position_secs DOUBLE PRECISION NOT NULL DEFAULT 0,
    completed     BOOLEAN          NOT NULL DEFAULT false,
    updated_at    TIMESTAMPTZ      NOT NULL DEFAULT now(),

    PRIMARY KEY (user_id, track_id)
);
