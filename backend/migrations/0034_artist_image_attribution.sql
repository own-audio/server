-- Attribution for an automatically-fetched artist image.
--
-- Not optional metadata: images now come from Wikimedia Commons, whose licences
-- (CC BY, CC BY-SA) generally *require* naming the author and the licence. An
-- image stored without this cannot lawfully be displayed, so the columns live
-- next to the image rather than being derived later.
--
-- All nullable: a public-domain file requires no attribution, and a user's own
-- uploaded picture has none to record.
ALTER TABLE music_artist_images
    ADD COLUMN image_source      TEXT,
    ADD COLUMN image_author      TEXT,
    ADD COLUMN image_license     TEXT,
    ADD COLUMN image_license_url TEXT,
    ADD COLUMN image_source_url  TEXT;
