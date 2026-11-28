-- AcoustID lookups per month, server-wide: what an operator's key has spent,
-- and what `ACOUSTID__MONTHLY_LIMIT` is checked against.
CREATE TABLE acoustid_usage (
    month   DATE    PRIMARY KEY,
    lookups INTEGER NOT NULL DEFAULT 0
);
