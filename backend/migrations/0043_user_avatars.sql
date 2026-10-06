-- Migration 0043: family member avatars.
-- A profile picture the user sets for themselves, visible to fellow family
-- members in the Family tab. Reuses `media_objects` (the same table every
-- other cover/artwork already lives in) rather than a bespoke avatars table.

ALTER TABLE users
    ADD COLUMN avatar_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL;
