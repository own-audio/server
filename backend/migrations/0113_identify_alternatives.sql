-- What a single-track lookup also found, so another recording is a choice
-- among these rather than another lookup; replaced when the owner searches
-- again with their own words. And whether the proposal is the owner's pick
-- rather than the run's, so picking another edition leaves it alone.
ALTER TABLE identify_proposals
    ADD COLUMN alternatives    JSONB   NOT NULL DEFAULT '[]',
    ADD COLUMN chosen_by_owner BOOLEAN NOT NULL DEFAULT false;
