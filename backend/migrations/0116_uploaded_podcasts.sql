-- Uploaded podcasts (docs/uploaded-podcasts-plan.md, U1): a show whose episodes its owner
-- uploads instead of a feed the server fetches. It is a podcast_feeds row with
-- source_type = 'upload' and feed_url = 'upload:<id>' (never fetched); its episodes carry
-- their audio from the start.

-- An uploaded episode's transcript is a stored file, not a publisher's URL. The storage
-- sweep finds this column through its foreign key; db::storage_usage counts it.
ALTER TABLE podcast_episodes
    ADD COLUMN IF NOT EXISTS transcript_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL;

-- Creating an uploaded show again with the same guid answers the existing one, so an
-- uploader that lost the answer never makes a second show.
CREATE UNIQUE INDEX IF NOT EXISTS podcast_feeds_upload_guid_uq
    ON podcast_feeds (user_id, podcast_guid)
    WHERE source_type = 'upload' AND podcast_guid IS NOT NULL;
