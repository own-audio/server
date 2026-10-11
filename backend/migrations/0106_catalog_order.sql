-- The orders /api/v2 pages the catalogue in (docs/api-v2-plan.md, V3), each with
-- the id as tie-breaker so a cursor is exact. A page is read in index order,
-- keeping rows the viewer may see, until it is full: it never sorts the whole
-- library to hand out a hundred rows.
CREATE INDEX contributors_family_sort_idx ON contributors (family_id, lower(COALESCE(sort_name, name)), id);
DROP INDEX albums_family_title_idx;
CREATE INDEX albums_family_title_idx ON albums (family_id, lower(title), id);
CREATE INDEX albums_family_created_idx ON albums (family_id, created_at DESC, id DESC);
CREATE INDEX genres_family_name_idx ON genres (family_id, lower(name), id);
CREATE INDEX music_tracks_title_idx ON music_tracks_all (lower(title), id);
CREATE INDEX music_tracks_created_idx ON music_tracks_all (created_at DESC, id DESC);
