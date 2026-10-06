-- Migration 0044: a family-level photo (distinct from each member's own
-- avatar, migration 0043) — shown at the top of the Family tab. Admin-set,
-- like the family name.

ALTER TABLE families
    ADD COLUMN avatar_object_id UUID REFERENCES media_objects (id) ON DELETE SET NULL;
