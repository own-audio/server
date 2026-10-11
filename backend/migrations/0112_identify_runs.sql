-- Identify runs: a background match of many tracks against MusicBrainz whose
-- results are proposals, accepted or rejected by the owner, never applied by
-- themselves. Nothing here touches a file.

CREATE TABLE identify_runs (
    id            UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    -- The owner: a run only ever covers tracks this user owns.
    user_id       UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    family_id     UUID        REFERENCES families (id) ON DELETE CASCADE,
    -- What the user picked: the whole library, a library folder, an album, a
    -- playlist, an artist, or a list of tracks.
    scope_kind    TEXT        NOT NULL CHECK (scope_kind IN
                  ('library', 'folder', 'album', 'playlist', 'artist', 'tracks')),
    scope_id      UUID,
    scope_track_ids UUID[],
    scope_label   TEXT,
    -- queued → running → ready (every group looked at) | failed | cancelled.
    -- 'waiting' while the source asks us to slow down.
    status        TEXT        NOT NULL DEFAULT 'queued' CHECK (status IN
                  ('queued', 'running', 'waiting', 'ready', 'failed', 'cancelled')),
    error         TEXT,
    tracks        INTEGER     NOT NULL DEFAULT 0,
    groups_total  INTEGER     NOT NULL DEFAULT 0,
    groups_done   INTEGER     NOT NULL DEFAULT 0,
    -- Consecutive refusals from the source, for the backoff.
    backoff_level SMALLINT    NOT NULL DEFAULT 0,
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    started_at    TIMESTAMPTZ,
    finished_at   TIMESTAMPTZ
);
CREATE INDEX identify_runs_user_idx ON identify_runs (user_id, created_at DESC);

-- One album (the tracks of one album entity) or one single track.
CREATE TABLE identify_groups (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id      UUID        NOT NULL REFERENCES identify_runs (id) ON DELETE CASCADE,
    kind        TEXT        NOT NULL CHECK (kind IN ('album', 'track')),
    label       TEXT        NOT NULL,
    track_ids   UUID[]      NOT NULL,
    -- pending until looked up; done after.
    done        BOOLEAN     NOT NULL DEFAULT false,
    -- Every release the source offered, best first, with its track mapping, so
    -- "another edition" is a choice among these, not another lookup.
    candidates  JSONB       NOT NULL DEFAULT '[]',
    chosen      SMALLINT,
    position    INTEGER     NOT NULL
);
CREATE INDEX identify_groups_run_idx ON identify_groups (run_id, done, position);

-- What a run proposes for one track.
CREATE TABLE identify_proposals (
    id          UUID        PRIMARY KEY DEFAULT gen_random_uuid(),
    run_id      UUID        NOT NULL REFERENCES identify_runs (id) ON DELETE CASCADE,
    group_id    UUID        NOT NULL REFERENCES identify_groups (id) ON DELETE CASCADE,
    track_id    UUID        NOT NULL REFERENCES music_tracks_all (id) ON DELETE CASCADE,
    -- sure: accept without looking; review: look first; none: nothing found.
    class       TEXT        NOT NULL CHECK (class IN ('sure', 'review', 'none')),
    -- Why it is not sure: 'incomplete', 'partial', 'layout', 'duplicate',
    -- 'ambiguous', 'album_differs', 'weak', 'not_found'.
    reasons     TEXT[]      NOT NULL DEFAULT '{}',
    -- Everything an accept writes, so accepting needs no lookup.
    proposal    JSONB,
    -- Keep the file's own album and set only the recording (compilations).
    keep_album  BOOLEAN     NOT NULL DEFAULT false,
    decision    TEXT        NOT NULL DEFAULT 'pending' CHECK (decision IN
                ('pending', 'accepted', 'rejected')),
    decided_at  TIMESTAMPTZ,
    UNIQUE (run_id, track_id)
);
CREATE INDEX identify_proposals_run_idx ON identify_proposals (run_id, class, decision);
