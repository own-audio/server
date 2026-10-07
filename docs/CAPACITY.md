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

1. **Subsonic albums and artists** (`subsonic/browsing.rs`, `db/subsonic.rs`).
   Album and artist ids are hashes, so every album or artist request
   aggregates the whole catalog to find one: getAlbumList2, getAlbum,
   getArtist, search, stars, and getCoverArt for album tiles (one full
   aggregation per tile, which ties up the connection pool). About 1–3 s each.
   Fix: real album and artist tables keyed by the id; sort, filter and page
   in SQL.
2. **`GET /music/tracks` is not paginated** (`music/mod.rs`, `db/music.rs`):
   the whole catalog, lyrics included, about 0.7–1 GB of memory and a 350 MB
   response. Fix: keyset pagination, no lyrics in lists.
3. **`GET /library/changes` without `since`** (`library/mod.rs`,
   `db/sync.rs`): every track as JSON, about 1 GB. Fix: page it like
   `/sync/tree`, or retire it for `/sync/tree`.
4. **File-sync path checks and organise** (`filesync/paths.rs`,
   `filesync/organise.rs`): a prefix check that cannot use an index, so a
   large import costs time quadratic in the library size, and organise runs
   in one long transaction. Fix: compare against the candidate's own
   prefixes with a prefix index, batch and commit in chunks.
5. **`GET /sync/tree/ids`**: every id in one response, about 100 MB. Fix: page
   it.
6. **Storage reconcile** (manual job): all object keys and the whole bucket
   listing in memory, about 300 MB. Fix: walk the listing page by page.
7. **Smart playlists and random songs**: `ORDER BY random()` over the
   catalog, then one query per result track. Fix: sampled selection, one
   batched fetch.
8. **No indexes for browsing**: artist, album, genre and title lookups and
   `%text%` search read every row; the visibility filter defeats index use.
   Fix: expression indexes on the normalised columns, `pg_trgm` for search.
9. **getStarred**: one query per starred track and a linear album search.
10. **Background work**: audio analysis and checksum backfill run 200 items
    an hour, so a 600k first import would take about four months. Fix:
    throughput sized for a first scan.
11. **Whole files read into memory** for tag reading, lyrics and analysis
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

## How we will know

- A **scale test**: a generated catalog of 600k tracks and 1,000 books in a
  test database (rows, not audio), the conformance suite and a Subsonic
  client's browse pattern against it, server memory and p95 latency
  recorded. Pass: memory flat compared with an empty catalog, browse calls
  under 300 ms.
- The **footprint comparison** in [RAM_USAGE.md](RAM_USAGE.md), repeated with
  the same catalog for Navidrome and Audiobookshelf.
