# Who the server is for, and how big it has to scale

Decided by Kornel, 2026-10-07. The design target every change to this server
is measured against.

## Positioning

**One install serves one family.** The open-source server is for a household
that runs its own server: a home lab, a NAS that runs containers, a small
VPS. It is not built for many unrelated families on one server; that is what
the hosted service at own.audio is for.

- **People:** a family, however many that is. There is no member limit
  (Kornel, 2026-10-07: "let people use it as they want"). A family is
  typically a handful to a dozen people, and that is what the sizing below
  assumes. Accounts are created by invite; open sign-up stays off.
- **Catalog:** large. A data-hoarder household: **600,000 songs and 1,000
  audiobooks**, plus podcasts, often on a disk or NAS share that the server
  only reads (read-only library folders,
  [issue #1](https://github.com/own-audio/server/issues/1)).
- **Footprint:** smaller than Navidrome and Audiobookshelf, the two servers
  such a household would otherwise run side by side. Measured as the whole
  stack (our server plus PostgreSQL) against the two of them together, as
  container memory with the same catalog and load. See
  [RAM_USAGE.md](RAM_USAGE.md) for why and for where we stand.

## What scales with what

The catalog is the database's and the storage's job, not the server's. That
is the rule this design follows:

| Grows with | Belongs in | Must not grow |
|---|---|---|
| number of songs, books, episodes | PostgreSQL rows and indexes, storage | server memory, request latency |
| listening history | PostgreSQL | server memory |
| people and their devices | connections, sessions | — (a family is small) |
| concurrent streams | storage and network (the server redirects to the file, it does not proxy audio) | server memory |

In practice that means every list is paginated, every catalog-wide question
is answered in SQL with an index, and media is streamed, never read into
memory whole.

### Rough sizing at the target

| | Estimate |
|---|---|
| Server process | under 50 MiB with the tuned allocator (28 MiB peak measured under the test suite) |
| PostgreSQL | about 2–4 GB on disk for 600k tracks with their media rows and indexes; 64–256 MB `shared_buffers` |
| Storage | the music itself: about 5 TB at 600k songs in MP3, more in FLAC; unchanged by the server |
| Host | 1 GB RAM for server and PostgreSQL with a small catalog, 2 GB for the full target |

These are estimates until the scale test below has run.

## Known gaps against the target (audit, 2026-10-07)

The rule above is **not true of today's code**. A read-only audit of
`backend/src` found these places where memory or time grows with the catalog.
Estimates are for 600k tracks, about 50k albums and 1.2M media objects; none
was measured yet.

1. **Subsonic albums and artists** (lookups by id fixed 2026-10-07: a key
   read through `subsonic_ids`; the list calls still aggregate) (`subsonic/browsing.rs`, `db/subsonic.rs`).
   Album and artist ids are hashes, so every album or artist request
   aggregates the whole catalog to find one: getAlbumList2, getAlbum,
   getArtist, search, stars, and getCoverArt for album tiles (one full
   aggregation per tile, which ties up the connection pool). About 1–3 s each.
   Fix: real album and artist tables keyed by the id; sort, filter and page
   in SQL.
2. **`GET /music/tracks` is not paginated** (fixed in alpha.6: streamed,
   flat memory) (`music/mod.rs`, `db/music.rs`):
   the whole catalog, lyrics included, about 0.7–1 GB of memory and a 350 MB
   response. Fix: keyset pagination, no lyrics in lists.
3. **`GET /library/changes` without `since`** (fixed 2026-10-07: streamed) (`library/mod.rs`,
   `db/sync.rs`): every track as JSON, about 1 GB. Fix: page it like
   `/sync/tree`, or retire it for `/sync/tree`.
4. **File-sync path checks and organise** (path checks fixed in alpha.7;
   organise stays one transaction on purpose: preview and apply make the
   same moves, and swapping two items' places is atomic; with indexed path
   checks it is linear in the items moved) (`filesync/paths.rs`,
   `filesync/organise.rs`): a prefix check that cannot use an index, so a
   large import costs time quadratic in the library size, and organise runs
   in one long transaction. Fix: compare against the candidate's own
   prefixes with a prefix index, batch and commit in chunks.
5. **`GET /sync/tree/ids`** (fixed 2026-10-07: streamed): every id in one response, about 100 MB. Fix: page
   it.
6. **Storage reconcile** (fixed 2026-10-08: paged) (manual job): all object keys and the whole bucket
   listing in memory, about 300 MB. Fix: walk the listing page by page.
7. **Smart playlists and random songs** (fixed 2026-10-08: one batched
   fetch; random songs shuffle ids only): `ORDER BY random()` over the
   catalog, then one query per result track. Fix: sampled selection, one
   batched fetch.
8. **No indexes for browsing** (measured 2026-10-07 within target: search
   0.12–0.14 s, artists 0.37 s, albums 0.7 s at 600,000 tracks; `pg_trgm`
   waits until a real library shows a need): artist, album, genre and title lookups and
   `%text%` search read every row; the visibility filter defeats index use.
   Fix: expression indexes on the normalised columns, `pg_trgm` for search.
9. **getStarred** (fixed 2026-10-08): one query per starred track and a linear album search.
10. **Background work** (fixed 2026-10-07: the next batch goes in as soon
    as the previous one drains): audio analysis and checksum backfill run 200 items
    an hour, so a 600k first import would take about four months. Fix:
    throughput sized for a first scan.
11. **Whole files read into memory** (fixed 2026-10-07 for tags, lyrics
    and analysis: streamed to a temporary file) for tag reading, lyrics and analysis
    (`music/mod.rs`, `jobs/worker.rs`); a large FLAC is 100–300 MB, times
    the job concurrency. Fix: stream to a temporary file, as uploads
    already do.

Already fine: streaming and downloads redirect to storage, uploads stream to
disk, `/sync/tree` is keyset-paginated, podcasts page their episodes, the
sweeps and backfills work in batches, and audiobooks, collections and
playlists are small enough at 1,000 books to stay unpaginated.

The work is tracked in [issue #2](https://github.com/own-audio/server/issues/2) and planned in
[IMPLEMENTATION_PLAN.md](IMPLEMENTATION_PLAN.md) ("Scale and footprint").

## Measured: 600,000 tracks and 1,000 books (2026-10-07)

`conformance/tools/scale_catalog.py` on the compose stack (Apple-silicon Mac,
local storage), generated rows, no audio. Server process memory (`VmRSS`);
idle 20 MiB with the catalog loaded.

| Call | Before (alpha.5 + allocator settings) | After streaming the track list |
|---|---|---|
| `GET /music/tracks`, the whole list (309 MB of JSON) | 3.0 s, **peak 805 MiB** | 3.2 s, **peak 23 MiB** |
| `GET /music/albums` | 2.0 s, peak 271 MiB | 2.0 s, peak 29 MiB |
| `GET /music/artists` | 2.5 s | 2.6 s |
| `GET /music/genres` | 1.1 s | 1.1 s |
| `GET /audiobooks` (1,000 books) | 0.01 s | 0.01 s |
| `GET /library/search` | 0.27 s | 0.28 s |
| Subsonic `getAlbumList2` (50 newest) | 1.7 s | 1.7 s |
| Subsonic `getArtists` | 2.6 s | 2.6 s, peak 66 MiB |
| Subsonic `search3` | 3.6 s | 3.6 s, peak 73 MiB |
| Server after the run | 308 MiB | 73 MiB |

Then two more changes, measured on a freshly generated catalog of the same
size: the visibility check written inline instead of a per-row function
call (the owner and family admins short-circuit; members' media policy is
read once per query and their grants as one set), and the album grouping
keys stored as generated columns with an index instead of a regular
expression per row per query.

| Call | Before | Now |
|---|---|---|
| `GET /music/tracks`, whole list | 3.0 s, peak 805 MiB | 1.8 s, peak 19 MiB |
| `GET /music/albums` (100,000 albums, 9.7 MB) | 2.0 s | 0.67 s (0.22 s of it in PostgreSQL) |
| `GET /music/artists` | 2.5 s | 0.37 s |
| `GET /music/genres` | 1.1 s | 0.10 s |
| `GET /library/search` | 0.27 s | 0.14 s |
| Subsonic `getAlbumList2` (50 newest) | 1.7 s | 0.44 s |
| Subsonic `getArtists` | 2.6 s | 0.68 s |
| Subsonic `search3` | 3.6 s | 1.2 s |
| Server after the run | 308 MiB | 67 MiB |

Memory is flat. What is still above the 300 ms target is mostly the size of
the answer (every album at once) or the Subsonic id scheme (gap 1 below:
album and artist ids are hashes, so a lookup still aggregates the catalog).

## Measured: lookups by id, sync lists, after issue #2 (2026-10-07)

Same tool and catalog size (600,000 generated tracks, 1,000 books), the
image before and after the change, on one database. Server memory is the
process's peak during the call.

| Call | Before | After |
|---|---|---|
| `GET /library/changes`, full sync (192 MB) | 1.9 s, **peak 1,889 MiB** | 2.0 s, **peak 21 MiB** |
| `GET /sync/tree/ids` (63 MB) | 0.3 s, **peak 1,565 MiB** | 0.8 s, **peak 21 MiB** |
| Subsonic `getAlbum` | 0.44 s | 0.011 s (the first after an upgrade 1.3 s, once) |
| Subsonic `getArtist` | 1.0 s | 0.011 s |
| Subsonic `getCoverArt`, an album tile | 0.40 s | 0.003 s |
| Subsonic `getArtists` | 0.71 s | 0.28 s |
| Server after the run | 1,561 MiB | 56 MiB |

The id table holds 120,000 rows for this catalog (100,000 albums, 20,000
artists) for the one user who browsed it.

## Measured: a first scan of 600,000 files (2026-10-10)

Library folders (issue #1) with `conformance/tools/scale_library.py`: 600,000
songs (5,000 artists, 50,000 albums of 12) and 1,000 books of 10 files, each
a real MP3 of about 2 KB with its own tags, in a Docker volume on an
Apple-silicon Mac (a bind mount would have measured Docker Desktop's file
sharing). Compose stack, local storage, a fresh database.

| | 1.0.0-beta.4 | after the path-check fix |
|---|---|---|
| Files after 10 minutes | 26,436 | 116,136 |
| Rate | 3,800 a minute falling to 1,000 by minute 27 | about 10,000 a minute, steady |
| First scan, 610,000 files | not finished (stopped at 48,000 after 27 minutes; hours on that curve) | **58 minutes**, no errors |
| Server memory during the scan | 16–33 MiB | **17–18 MiB** (one 30 MiB peak) |
| Database afterwards | — | 1.4 GB |
| Rescan with nothing changed | — | **66 s**, peak 22 MiB |
| `GET /music/albums` (50,000 albums) afterwards | — | 0.93 s |

The fall-off was the file-sync path claim (`filesync/paths.rs`): one OR with a
`LIKE $param` that the generic plan of a prepared statement could not turn
into an index range, so every new item read every path its owner had (40 ms
a file at 44,000 files). It is three index lookups now (0.14 ms).

## Measured: unplayed episodes per show (2026-11-17)

`GET /api/v2/me/podcasts/unplayed` (v2 phase 2) counts, for one page of shows, the episodes the
listener has not finished: the page's ids first, then one `GROUP BY` over their episodes and the
listener's progress. On a synthetic household of 300 shows and 100,200 episodes, 30,000 of them
with progress, a page of 101 shows (a third of all episodes) took **12 ms** (`EXPLAIN ANALYZE`,
hash joins over the episodes; a smaller share of the catalog uses `podcast_episodes (feed_id)`).
The existing indexes serve it — the episodes' `feed_id`, the progress key `(user_id,
episode_id)` — so no migration was added.

## Measured: sync essentials and continue listening (2026-11-17)

v2 phase 3 (`api-v2-completion-plan.md`), `EXPLAIN ANALYZE` on PostgreSQL 16 with its default
settings (Apple-silicon Mac): 610,000 generated tracks (20,000 artists, 50,000 albums) with a
path each, 1,000 books, 30,000 track positions (half unfinished), and the 2.5 million rows the
change log wrote while it was filled.

| Query | Time |
|---|---|
| A page of the ids-only snapshot (`fields=ids`, 5,000 track ids), household admin / member, start and middle of the catalog | 3–9 ms (primary key walk); about 120 pages, roughly 1 s of database time and 37 MB of NDJSON for the catalog |
| 500 track records with `path` and `sha256` (the snapshot's and the change feed's batch) | 4.6 ms (one `sync_paths` and one `media_objects` key lookup per track) |
| The checksum trigger finding the tracks of a file | 0.03 ms (new `music_tracks_audio_object_idx`; without it, a scan of every track per computed checksum) |
| `/sync/changes?kinds=track`, 500 rows from the start of the log | 7 ms |
| `/sync/changes?kinds=progress,episode` from the start, when the kinds asked for are rare | 280–390 ms: the read walks the log since the cursor and filters; a cursor from the last sync has little to walk. No index (`(kind, seq)` would help only this worst case). |
| Continue listening (`kind=track,book,episode&unfinished=true`, 25) | 33 ms → **0.7 ms** with `music_progress_recent_idx` / `podcast_progress_recent_idx` `(user_id, updated_at DESC, id DESC)` |
| Member records of a household | 0.02 ms |

Writing the change log costs a little more where a track appears: 60,000 generated tracks in
one statement took 44 s without the new "what the track makes visible" trigger and 48 s with it
(about 10 %; a first version that also re-read the track's credits and genres on insert cost
50 %). Inserting many tracks in one statement grows faster than linearly (20,000 in 12 s,
60,000 in 44 s, before this phase): batches of tens of thousands, as the scan already does, keep
it linear (55,000 a batch, 40 s each, up to 610,000).

## Measured: the trash on v2 (2026-11-17)

v2 phase 4 (`api-v2-completion-plan.md`), `EXPLAIN ANALYZE` on PostgreSQL 16 with its default
settings (Apple-silicon Mac): 605,000 generated tracks and 1,000 books, 5,000 of the tracks in
the trash (spread over 30 days, in batches of 20).

| Query | Time |
|---|---|
| `GET /api/v2/trash`, first page of 100, the owner's (`scope=mine`) or the household's (`scope=family`) | 56 ms |
| The same, a page from the middle (cursor at item 2,500) | 9.5 ms |
| A batch's items (`POST /trash/batches/{batch}/restore`) | 0.7 ms |

No migration: each table's partial `trashed_at` index (0077) keeps every trash query to the
trashed rows, so the catalog's size does not enter. What grows is the trash itself: a page
reads every trashed row newer than its cursor, with its size and names, and sorts the top 100
(the order's tie-breakers `kind, id` are not in the index). A trash of 50,000 items would make
the first page about half a second; the fix then is `(trashed_at DESC, id DESC) WHERE trashed_at
IS NOT NULL` per table, so each branch is read in order and the page stops after 100 rows.
`DELETE /api/v2/trash` walks the same pages 200 rows at a time.

## Measured: books' tags, series and collections (2026-11-17)

v2 phase 5 (`api-v2-completion-plan.md`), on PostgreSQL 16 (compose stack, Apple-silicon Mac):
1,000 shared books, 50 tags at 5 a book, 100 series of 10 books, 50 collections of 20.

| Request (whole answer, page of 100) | Time |
|---|---|
| `GET /api/v2/book-tags` | 3 ms |
| `GET /api/v2/books?tag=` | 3 ms |
| `GET /api/v2/series` (each with its books) | 2.4 ms |
| `GET /api/v2/collections` (each with its count) | 4.8 ms (the query 3 ms) |
| `GET /api/v2/collections/{id}/books` | 4.8 ms |

Not run on the 600,000-track fixture: none of these reads a catalog-sized table. Books are 1,000
at most, and tags, series and collections are bounded by them; the contributor routes look up
one contributor by id, and "still credited" (`DELETE /contributors/{id}`) is the credits'
`(contributor_id, item_kind)` index. Migration 0127 adds `(book_id)` indexes on series and
collection membership, so the change-log trigger goes from a book to its series and
collections without a scan.

## Measured: duplicates on v2 (2026-11-17)

v2 phase 6 (`api-v2-completion-plan.md`), `EXPLAIN ANALYZE` on PostgreSQL 16 (compose stack,
Apple-silicon Mac): 601,000 generated tracks of one owner, every file with a checksum, 3,968 of
them in pairs (0.7 %).

| `GET /api/v2/tracks/duplicates`, a page of 100 groups | Time |
|---|---|
| First page | 60 ms |
| A page from the middle (cursor half way through the checksums) | 56 ms |
| A library without a single duplicate (the whole checksum index walked for an empty page) | 2.3 s |

No migration for it: the walk goes along `media_objects_sha256_idx` in checksum order and joins
each object's track through `music_tracks_audio_object_idx` (0126), stopping after 100 groups, so
memory stays flat and a page costs what it walks. The fewer duplicates a library has, the more it
walks per page; the worst case is the empty answer above. If that ever matters, a partial index of
duplicated checksums (kept by the checksum job) makes it a key read. Filling the fixture also
showed that one 50,000-track `INSERT` now takes minutes where 5,000-track statements take 4 s
each (600,000 in about 9 minutes): load in batches of thousands, as the scan does.

Migration 0128 adds `music_playlist_tracks (track_id)`: the playlists that hold a track are now a
key read, for the new trigger and for the cascade when a track is purged, which scanned every
playlist entry before.

## Measured: inserting tracks in bulk (2026-11-17)

Before v2 phase 7, to see whether the change-log triggers of 0109–0128 make a bulk insert of
tracks grow faster than linearly (phase 6 saw one 50,000-track `INSERT` run past six minutes).
The scale tool's own statement (`INSERT … SELECT` of N tracks with their objects), on a
throwaway PostgreSQL 17 with every migration through 0128, Apple-silicon Mac:

| Tracks in one statement | Time | Per track |
|---|---|---|
| 2,000 | 1.4 s | 0.70 ms |
| 5,000 | 3.4 s | 0.68 ms |
| 10,000 | 6.7 s | 0.67 ms |
| 20,000 | 14.5 s | 0.73 ms |
| 50,000 | 39.8 s | 0.80 ms |

Onto a growing catalog, 50,000 at a time: 37 s onto 55,000 tracks, then 26–29 s each onto
105,000, 155,000, 205,000, 255,000 and (after `ANALYZE`) 305,000; 5,000 onto 355,000 in 2.3 s.
The change log takes four rows a track (the track, its album, recording and release group). On
the compose stack (PostgreSQL 16, the server running) `scale_catalog.py` added 50,000 tracks and
1,000 books in 54 s, and another 50,000 in 61 s.

So no trigger is at fault and nothing was changed: the cost per track stays flat with the size
of the statement and of the catalog. Phase 6's six minutes came back in none of these runs; it
was the fixture's one long statement on a busy stack, not the triggers. One track at a time —
the upload path — takes 2.9 ms (median; 6 ms p95, 14 ms at most) per `INSERT` on 355,000 tracks,
each in an album of its own.

## Measured: plays history and statistics on v2 (2026-11-17)

Phase 9 of `api-v2-completion-plan.md`. The history and the statistics grow with time, not the
catalog: 2,100,000 plays in `listening_sessions` on the compose stack's PostgreSQL 16 — one
listener with 1,000,000 over ten years (a play every five minutes), eleven with 100,000 each —
`EXPLAIN ANALYZE` against the heavy listener:

| Query | Plan | Time |
|---|---|---|
| `GET /me/plays`, first page of 100 | index scan `(user_id, started_at)`, incremental sort for the id | 0.08 ms |
| …a page five years back, as first written (`(started_at, id) < (…)` only) | the same scan, the row comparison a filter over 500,000 rows | 69 ms |
| …with `started_at <= $2` beside it (as shipped), generic plan | the bound is an index condition | 0.07 ms |
| `GET /me/stats`, 30 days (8,200 plays) | bitmap scan of the same index | 1.1 ms |
| …a year grouped by month (100,000 plays) | bitmap scan, sort | 27 ms |
| …the year's top ten | bitmap scan, hash aggregate, top-N | 26 ms |

No index added: `(user_id, started_at DESC)` from 0020 serves all of it once the page query
names its bound on `started_at`. A statistic costs the plays in its range, never the catalog;
a year of a heavy listener is under 30 ms. The hourly rollup (`listening_daily`) was not used:
it counts UTC days and lags by up to an hour, and the question is the listener's own days now.

## How we will know

- A **scale test**: a generated catalog of 600k tracks and 1,000 books in a
  test database (rows, not audio), the conformance suite and a Subsonic
  client's browse pattern against it, server memory and p95 latency
  recorded. Pass: memory flat compared with an empty catalog, browse calls
  under 300 ms.
- The **footprint comparison** in [RAM_USAGE.md](RAM_USAGE.md), repeated with
  the same catalog for Navidrome and Audiobookshelf.
