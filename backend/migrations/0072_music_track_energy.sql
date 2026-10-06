-- Migration 0072: derived energy
-- ──────────────────────────────
-- docs/music-signals-and-smart-playlists-plan.md §3.3.
--
-- "Energy" is not a physical quantity. Spotify's was a proprietary blend and
-- there is no ground truth to match, which makes this easier rather than
-- harder: nothing has to be *correct*, it has to be **consistent and monotonic
-- within one library**. A percentile gives exactly that, and "give me 0.4–0.7"
-- then means something even though the number means nothing on its own.
--
-- WHY A VIEW AND NOT A COLUMN
--
-- A percentile moves every time the library grows. Stored on the track it would
-- be wrong the moment the next import finished, and correcting it would mean
-- re-decoding audio — which is the one expensive thing here. Derived over the
-- raw measurements it is a cheap refresh instead, and the measurements never
-- need touching again.
--
-- HOW IT IS COMPOSED
--
-- Each dimension is turned into its own percentile first, then the percentiles
-- are averaged, then the result is ranked again. Ranking before combining means
-- the units never have to be reconciled (Hz against LUFS against onsets per
-- second) and one outlier cannot drag the scale.
--
--   loudness         louder masters read as more energetic
--   onset rate       busier reads as more energetic
--   spectral centroid brighter reads as more energetic
--   dynamic range    INVERTED: a wide dynamic range is an open, breathing
--                    recording; a compressed one is a loud one
--
-- THE WEIGHTS ARE PROVISIONAL. They are a starting point to be judged by ear
-- against one real library, which is what S4's verification step is for. They
-- live here, in one place, and changing them costs a REFRESH.
--
-- A track measured only partially still gets a score: a missing dimension
-- contributes the median (0.5) rather than dropping the track out of every
-- playlist it should have been in.

CREATE MATERIALIZED VIEW music_track_energy AS
WITH ranked AS (
    SELECT
        id AS track_id,
        PERCENT_RANK() OVER (ORDER BY loudness_lufs)     AS p_loud,
        PERCENT_RANK() OVER (ORDER BY onset_rate)        AS p_onset,
        PERCENT_RANK() OVER (ORDER BY spectral_centroid) AS p_bright,
        PERCENT_RANK() OVER (ORDER BY dynamic_range)     AS p_dynamic,
        loudness_lufs, onset_rate, spectral_centroid, dynamic_range
    FROM music_tracks
    WHERE analysis_version IS NOT NULL
),
composite AS (
    SELECT
        track_id,
        0.35 * COALESCE(CASE WHEN loudness_lufs     IS NULL THEN NULL ELSE p_loud    END, 0.5)
      + 0.30 * COALESCE(CASE WHEN onset_rate        IS NULL THEN NULL ELSE p_onset   END, 0.5)
      + 0.25 * COALESCE(CASE WHEN spectral_centroid IS NULL THEN NULL ELSE p_bright  END, 0.5)
      + 0.10 * COALESCE(CASE WHEN dynamic_range     IS NULL THEN NULL ELSE 1.0 - p_dynamic END, 0.5)
          AS score
    FROM ranked
)
SELECT
    track_id,
    -- Ranked a second time so the output is spread evenly over [0, 1]. A
    -- weighted average of percentiles clusters around the middle, which would
    -- make "0.4–0.7" match most of the library.
    PERCENT_RANK() OVER (ORDER BY score)::real AS energy,
    score::real AS raw_score
FROM composite;

-- REFRESH ... CONCURRENTLY needs this, and concurrent refresh is what keeps
-- playlist queries answerable while the view rebuilds.
CREATE UNIQUE INDEX music_track_energy_pk ON music_track_energy (track_id);
CREATE INDEX music_track_energy_idx ON music_track_energy (energy);
