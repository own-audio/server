# Data model plan — entities for music, books and podcasts

Step 2 of the own.audio API's order of work (`own-audio/api` design/DESIGN.md §7, §14):
the server stops grouping music by tag strings and keeps real entities — contributors,
credits, release groups, albums, recordings, genres — that `/api/v2`, the Subsonic layer and
the console all read. Written 2026-10-11.

## Where we start

- Music has no entities. Artists and albums are `GROUP BY` over three generated tag keys on
  `music_tracks_all` (`album_artist_key`, `primary_artist_key`, `album_key`, migration 0089).
  Stars, ratings, artist images and Subsonic ids hang off those strings
  (`music_group_stars`, `music_group_ratings`, `music_artist_images`, `subsonic_ids`), so a
  renamed tag loses them and two families' "Unknown Artist" are one name.
- Books have `audiobook_authors` + `audiobook_book_authors` with a role — the right shape, but
  global rather than per household, and separate from music.
- Podcasts have no people at all.
- Tags reach the database through few paths: `db::music::insert_track`, `update_track`,
  `apply_musicbrainz_metadata`, `set_release_info`, and the folder scanner's updates.

## Decisions

1. **Entities belong to a household.** `family_id` is the owner's family (from
   `family_members`), not the track's `family_id` — that column means "shared", and a private
   track still belongs to its household's album and artists. An entity is visible to a member
   when at least one item credited to it is; no visibility is stored on entities.
2. **The database keeps them in step.** A trigger on `music_tracks_all` links a track to its
   entities whenever a tag column changes, and one on `family_members` relinks a user's tracks
   when they move household. Every write path — upload, scan, identify, edits, the hosted
   edition — is covered without each remembering to call something. The backfill is the same
   function run over every track in the migration.
3. **Matching.** MusicBrainz ids first, when the tags carry them; otherwise normalised text:
   contributors by `lower(trim(name))`, albums by album-artist key + album key, release groups
   by release-group id or one per album, recordings only by MusicBrainz recording id (no text
   guessing: two songs with one title are not one recording). Once an entity exists, its
   display name and sort name are its own and may be corrected; matching keeps using the key
   it was found by, so a rescan finds the corrected entity instead of making a new one.
4. **Credits from a tag string.** `artist` becomes credits split on the guest markers the
   generated keys already know (`feat.`, `ft.`, `featuring`) and on `;` — the multi-value
   separator taggers write. `&`, `,` and `and` are *not* split: "Simon & Garfunkel" and
   "Earth, Wind & Fire" are one act. Roles: `artist` for the first, `featured` after a guest
   marker. The album gets `album_artist` (or the primary artist) as its credit.
5. **Tag strings stay on the track as read.** Entities are derived; nothing about the file is
   overwritten, so a file can always be re-matched.
6. **Clean cut for what hangs off names.** Group stars, ratings and artist images move to
   entity ids in the migration that creates them; `subsonic_ids` goes, and the Subsonic layer
   uses entity ids (its ids change once — DESIGN §4). v1 endpoints keep their shapes and gain
   `id` fields, so the apps keep working until they move to v2.

## Phases

Status 2026-10-11: E1 (`dfefb49`), E2a (`1d1592e`), E2b (`fac8d4c`) and E3 for books are done; podcast hosts (rest of E3) and E4 next.

| Phase | Content | Test |
|---|---|---|
| **E1** | Schema (`contributors`, `contributor_keys`, `credits`, `release_groups`, `albums`, `recordings`, `genres`, `track_genres`; `music_tracks_all.album_id`, `recording_id`), the link function and triggers, the backfill. v1 artist and album lists gain `id`. | conformance `music`: an uploaded track has an album and its artists by id; a feat. guest is a second credit; a retag relinks |
| **E2** | Readers move: v1 artist/album lists and details from entities (by id, name routes kept); stars, ratings, artist images on entity ids; Subsonic browsing on entity ids, `subsonic_ids` dropped; the old grouping keys removed. | conformance `smoke`/Subsonic unchanged answers; 600k scale run |
| **E3** | Books: `audiobook_authors` → household contributors, `audiobook_book_authors` → credits (author, narrator, translator…). Podcasts: hosts from `<itunes:author>` / `<podcast:person>`. One contributor across media. | conformance: an author who also sings is one contributor |
| **E4** | Corrections: rename, merge, split contributors; rename and re-parent albums; all as entity operations recorded in the change feed. | conformance |

Kind vocabulary, one progress model and the two-word sharing come with `/api/v2` (step 3),
where they change wire shapes; the entities they name exist after E1–E3.

## Scale

Measured 2026-10-11 on the generated library (600,000 tracks, 100,000 albums,
21,000 contributors), PostgreSQL 16 in Docker on an Apple-silicon Mac:

| What | Time |
|---|---|
| Migration 0100, linking every track | 67 s |
| A new track through the triggers | 0.5 ms (a bare insert: 0.07 ms) |
| A user moving household with 620,000 tracks (`music_link_tracks`) | 98 s, inside the request that moves them |
| Rows added to the file-sync log by any of it | 0 |

Two traps found on the way, both in the set-based function: temp tables need
`ANALYZE` before they are joined, and `IN (subquery)` over hundreds of
thousands of ids turns quadratic once the subquery no longer fits in
`work_mem` — joins instead. The household move is fine for the rare big
library; if it ever becomes common, it moves to a job.

The generated 600,000-track library (`docs/CAPACITY.md`) is the yardstick, not a migration
source: the backfill must finish in minutes, the trigger must not double the cost of a scan,
and every list must stay index-backed.
