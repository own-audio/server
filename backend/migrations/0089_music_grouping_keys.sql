-- Album grouping keys, computed once when a track is written instead of on
-- every row of every browse query (docs/CAPACITY.md: the regular expression
-- ran 600,000 times per album list). The expressions are the SQL twins of
-- MusicTrack::effective_album_artist / music::models::primary_artist and
-- must stay identical to them; db::music::ALBUM_ARTIST_SQL etc. now name
-- these columns.
ALTER TABLE music_tracks_all
    ADD COLUMN album_artist_key TEXT GENERATED ALWAYS AS (COALESCE(
        NULLIF(trim(album_artist), ''),
        NULLIF(regexp_replace(trim(artist), '\s+[(\[]?(feat\.?|ft\.?|featuring)\s.*$', '', 'i'), ''),
        'Unknown Artist')) STORED,
    ADD COLUMN primary_artist_key TEXT GENERATED ALWAYS AS (COALESCE(
        NULLIF(regexp_replace(trim(artist), '\s+[(\[]?(feat\.?|ft\.?|featuring)\s.*$', '', 'i'), ''),
        'Unknown Artist')) STORED,
    ADD COLUMN album_key TEXT GENERATED ALWAYS AS (COALESCE(NULLIF(trim(album), ''), 'Unknown Album')) STORED;

CREATE OR REPLACE VIEW music_tracks AS
    SELECT * FROM music_tracks_all WHERE trashed_at IS NULL;

CREATE INDEX music_tracks_album_group_idx
    ON music_tracks_all (family_id, album_artist_key, album_key)
    INCLUDE (user_id, duration_secs, cover_object_id, created_at)
    WHERE trashed_at IS NULL;
CREATE INDEX music_tracks_primary_artist_idx
    ON music_tracks_all (family_id, primary_artist_key)
    WHERE trashed_at IS NULL;
