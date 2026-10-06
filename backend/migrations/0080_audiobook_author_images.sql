-- Author photos, fetched from Wikimedia Commons the same way music artist
-- images are (see 0032/0033/0034) — an exact copy of that shape, added
-- directly to audiobook_authors rather than a separate cache table, since
-- authors (unlike music artists) already have one global row each.
--
-- image_fetched_at doubles as the miss cache: NULL means never looked up,
-- non-NULL with image_object_id still NULL means "asked, nothing found" —
-- see MISS_TTL_DAYS in db/authors.rs for how long that is believed.
--
-- is_user_set and the attribution columns mean exactly what they mean for
-- music_artist_images: a deliberately chosen picture is never overwritten by
-- a later automatic lookup, and a Commons licence's attribution requirement
-- travels with the image rather than being derived later.
ALTER TABLE audiobook_authors
    ADD COLUMN is_user_set      BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN image_fetched_at TIMESTAMPTZ,
    ADD COLUMN image_source     TEXT,
    ADD COLUMN image_author     TEXT,
    ADD COLUMN image_license    TEXT,
    ADD COLUMN image_license_url TEXT,
    ADD COLUMN image_source_url TEXT;
