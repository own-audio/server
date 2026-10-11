-- What hung off artist and album *names* moves onto the entities of 0100
-- (docs/data-model-plan.md, E2): stars, ratings and artist images. A renamed
-- tag no longer loses them, and two households never share one by spelling.
-- The Subsonic layer's name cache (subsonic_ids) goes: its ids are entity
-- ids now, and change once (DESIGN §4: no aliases).

-- ── Artist images: on the household's contributor ───────────────────────

ALTER TABLE contributors
    ADD COLUMN image_fetched_at  TIMESTAMPTZ,
    ADD COLUMN image_is_user_set BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN image_source      TEXT,
    ADD COLUMN image_author      TEXT,
    ADD COLUMN image_license     TEXT,
    ADD COLUMN image_license_url TEXT,
    ADD COLUMN image_source_url  TEXT;

-- Per contributor, the best image any member had for that name: one set by
-- hand first, then one actually found, then the newest miss.
UPDATE contributors c SET
    image_object_id   = ai.image_object_id,
    image_fetched_at  = ai.fetched_at,
    image_is_user_set = ai.is_user_set,
    image_source      = ai.image_source,
    image_author      = ai.image_author,
    image_license     = ai.image_license,
    image_license_url = ai.image_license_url,
    image_source_url  = ai.image_source_url
FROM (
    SELECT DISTINCT ON (k.contributor_id) k.contributor_id, i.*
      FROM music_artist_images i
      JOIN family_members fm ON fm.user_id = i.user_id
      JOIN contributor_keys k ON k.family_id = fm.family_id AND k.key = 'name:' || lower(trim(i.artist_name))
     ORDER BY k.contributor_id, i.is_user_set DESC, (i.image_object_id IS NOT NULL) DESC, i.fetched_at DESC
) ai
WHERE c.id = ai.contributor_id;

DROP TABLE music_artist_images;

-- ── Stars and ratings: on a contributor or an album ─────────────────────

CREATE TABLE music_stars (
    user_id        UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    contributor_id UUID        REFERENCES contributors (id) ON DELETE CASCADE,
    album_id       UUID        REFERENCES albums (id) ON DELETE CASCADE,
    created_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((contributor_id IS NULL) <> (album_id IS NULL))
);
CREATE UNIQUE INDEX music_stars_contributor_uq ON music_stars (user_id, contributor_id) WHERE contributor_id IS NOT NULL;
CREATE UNIQUE INDEX music_stars_album_uq ON music_stars (user_id, album_id) WHERE album_id IS NOT NULL;
CREATE INDEX music_stars_contributor_idx ON music_stars (contributor_id) WHERE contributor_id IS NOT NULL;
CREATE INDEX music_stars_album_idx ON music_stars (album_id) WHERE album_id IS NOT NULL;

CREATE TABLE music_ratings (
    user_id        UUID        NOT NULL REFERENCES users (id) ON DELETE CASCADE,
    contributor_id UUID        REFERENCES contributors (id) ON DELETE CASCADE,
    album_id       UUID        REFERENCES albums (id) ON DELETE CASCADE,
    rating         SMALLINT    NOT NULL CHECK (rating BETWEEN 1 AND 5),
    updated_at     TIMESTAMPTZ NOT NULL DEFAULT now(),
    CHECK ((contributor_id IS NULL) <> (album_id IS NULL))
);
CREATE UNIQUE INDEX music_ratings_contributor_uq ON music_ratings (user_id, contributor_id) WHERE contributor_id IS NOT NULL;
CREATE UNIQUE INDEX music_ratings_album_uq ON music_ratings (user_id, album_id) WHERE album_id IS NOT NULL;
CREATE INDEX music_ratings_contributor_idx ON music_ratings (contributor_id) WHERE contributor_id IS NOT NULL;
CREATE INDEX music_ratings_album_idx ON music_ratings (album_id) WHERE album_id IS NOT NULL;

-- An artist star found its contributor by name in the user's household; an
-- album star its album by album artist and title. One that matches nothing
-- (the music is gone) is dropped with the old table.
INSERT INTO music_stars (user_id, contributor_id, created_at)
SELECT DISTINCT ON (s.user_id, k.contributor_id) s.user_id, k.contributor_id, s.created_at
  FROM music_group_stars s
  JOIN family_members fm ON fm.user_id = s.user_id
  JOIN contributor_keys k ON k.family_id = fm.family_id AND k.key = 'name:' || lower(trim(s.artist_name))
 WHERE s.kind = 'artist'
 ORDER BY s.user_id, k.contributor_id, s.created_at;
INSERT INTO music_stars (user_id, album_id, created_at)
SELECT DISTINCT ON (s.user_id, a.id) s.user_id, a.id, s.created_at
  FROM music_group_stars s
  JOIN family_members fm ON fm.user_id = s.user_id
  JOIN albums a ON a.family_id = fm.family_id
               AND lower(a.artist_display) = lower(trim(s.artist_name)) AND lower(a.title) = lower(trim(s.album_name))
 WHERE s.kind = 'album'
 ORDER BY s.user_id, a.id, s.created_at;

INSERT INTO music_ratings (user_id, contributor_id, rating, updated_at)
SELECT DISTINCT ON (r.user_id, k.contributor_id) r.user_id, k.contributor_id, r.rating, r.updated_at
  FROM music_group_ratings r
  JOIN family_members fm ON fm.user_id = r.user_id
  JOIN contributor_keys k ON k.family_id = fm.family_id AND k.key = 'name:' || lower(trim(r.artist_name))
 WHERE r.kind = 'artist'
 ORDER BY r.user_id, k.contributor_id, r.updated_at DESC;
INSERT INTO music_ratings (user_id, album_id, rating, updated_at)
SELECT DISTINCT ON (r.user_id, a.id) r.user_id, a.id, r.rating, r.updated_at
  FROM music_group_ratings r
  JOIN family_members fm ON fm.user_id = r.user_id
  JOIN albums a ON a.family_id = fm.family_id
               AND lower(a.artist_display) = lower(trim(r.artist_name)) AND lower(a.title) = lower(trim(r.album_name))
 WHERE r.kind = 'album'
 ORDER BY r.user_id, a.id, r.updated_at DESC;

DROP TABLE music_group_stars;
DROP TABLE music_group_ratings;

-- ── Subsonic ids are entity ids now ─────────────────────────────────────

DROP TABLE subsonic_ids;
