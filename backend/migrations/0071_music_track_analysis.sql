-- Migration 0071: measured acoustic properties of a track
-- ───────────────────────────────────────────────────────
-- docs/music-signals-and-smart-playlists-plan.md §3.
--
-- MusicBrainz does not have this data and never did — it is a database of
-- identity and relationships, not of signal. AcousticBrainz had it but stopped
-- collecting in 2022 and its high-level labels were never trustworthy. Spotify's
-- audio-features API closed to new applications in 2024. So we measure it
-- ourselves, from files we already hold, which is a capability no streaming
-- service's customer has.
--
-- All measurement is server-side, on one extractor (decided 2026-09-21). Two
-- desktops each measuring would have produced values that are not comparable,
-- and because energy is a percentile across the library, the scale would have
-- degraded silently — nothing failing, results merely getting worse.
--
-- WHAT IS STORED IS RAW MEASUREMENT, NOT DERIVED ENERGY.
--
-- "Energy" is not a physical quantity; Spotify's was a proprietary blend and
-- there is no ground truth to match. That makes the problem easier, not harder:
-- we need a scale that is consistent *within one library*, which a percentile
-- over these columns gives. But a percentile moves every time the library
-- grows, so storing it would mean re-decoding everything on every import.
-- Energy is derived over these columns instead (S4).
--
-- `analysis_raw` keeps the extractor's full output because a second pass over
-- the library is expensive — it means fetching every object out of R2 again.
-- Postgres storage is cheap by comparison, so extract everything on the first
-- pass and let later questions be answered with SQL.
--
-- `analysis_version` exists so a changed *formula* can be re-derived. The goal
-- is never having to bump it to re-fetch *data*.

ALTER TABLE music_tracks
    -- Measured well on steady-beat music; unreliable on rubato classical, free
    -- jazz and ambient. Watch for octave errors (70 reported as 140).
    ADD COLUMN bpm               REAL,
    -- 'C', 'F#', … and 'major'/'minor'. Useful for sequencing, too weak to
    -- filter on.
    ADD COLUMN music_key         TEXT,
    ADD COLUMN key_scale         TEXT CHECK (key_scale IN ('major', 'minor')),
    -- EBU R128 integrated loudness. An exact measurement, not an estimate.
    ADD COLUMN loudness_lufs     REAL,
    -- Peak-to-RMS in dB. Exact.
    ADD COLUMN dynamic_range     REAL,
    -- Hz. The "brightness" axis, and the strongest single input to energy.
    ADD COLUMN spectral_centroid REAL,
    -- Detected onsets per second. The "busyness" axis.
    ADD COLUMN onset_rate        REAL,
    ADD COLUMN analysis_raw      JSONB,
    ADD COLUMN analysis_version  SMALLINT,
    ADD COLUMN analyzed_at       TIMESTAMPTZ;

-- The work queue is a query, not a table: "everything not yet measured at the
-- current version". That is what makes the backfill restartable for free —
-- state lives in the data, so a killed worker costs nothing.
CREATE INDEX music_tracks_analysis_pending_idx
    ON music_tracks (analysis_version)
    WHERE analysis_version IS NULL;

-- Selection filters on tempo and derives energy from the rest.
CREATE INDEX music_tracks_bpm_idx
    ON music_tracks (bpm)
    WHERE bpm IS NOT NULL;
