-- Artist images, cached per user by artist *name*.
--
-- Deliberately not an `artists` table. An artist in this schema is a
-- `GROUP BY artist` over `music_tracks` and nothing more — it has no row, no
-- id, and no lifecycle. Introducing one purely to hang a picture on would
-- create an entity the rest of the system doesn't have, which would then need
-- keeping in sync with a derived aggregate every time a track's artist changes
-- (MusicBrainz identification rewrites exactly that field). Keying by name
-- matches how artists actually exist here: rename the tracks and the image is
-- simply looked up fresh for the new name, which is the correct outcome rather
-- than a reconciliation problem.
--
-- `image_object_id` is nullable on purpose — a NULL row means "we asked the
-- provider and it had nothing", which is a different fact from "we never
-- asked". Without that negative cache, every render of a grid containing an
-- artist no provider knows would re-query the network forever.
CREATE TABLE music_artist_images (
    user_id         UUID NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    artist_name     TEXT NOT NULL,
    image_object_id UUID REFERENCES media_objects(id) ON DELETE SET NULL,
    -- When the provider was last asked. Lets a future re-check expire negative
    -- entries without another migration; nothing reads it yet.
    fetched_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, artist_name)
);
