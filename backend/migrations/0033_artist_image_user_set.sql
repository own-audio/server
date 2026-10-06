-- Marks an artist image the user chose themselves, as opposed to one fetched
-- from a public catalogue.
--
-- Without this the two are indistinguishable, and the automatic path would
-- overwrite a deliberate choice the next time it ran — the same mistake
-- `metadata/rescan` avoids by refusing on a track already identified via
-- MusicBrainz. A picture someone picked on purpose outranks anything a search
-- returns, permanently and without needing to be re-picked.
ALTER TABLE music_artist_images
    ADD COLUMN is_user_set BOOLEAN NOT NULL DEFAULT FALSE;
