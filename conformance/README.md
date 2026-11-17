# Conformance suite

Black-box tests of the server API, run against any own.audio server by URL:
a laptop, the compose stack, canary, production. This is the executable half
of [docs/API_COMPATIBILITY.md](../docs/API_COMPATIBILITY.md): when the suite
and the document disagree, one of them is wrong and gets fixed in the same
commit.

```bash
# local compose stack (defaults shown)
python3 conformance/run.py --base-url http://127.0.0.1:8080 \
    --admin-email admin@audio2.local --admin-password admin \
    --compose-dir ../audio2          # enables the SQL-backed suites

# a remote server: only what the API alone can prove
python3 conformance/run.py --base-url https://api-canary.example \
    --admin-email ci-smoke@example --admin-password "$CANARY_ADMIN_PASSWORD"

python3 conformance/run.py --list            # suite names
python3 conformance/run.py --only smoke,trash
python3 conformance/run.py --report out.json # machine-readable result
```

After a run, `run.py` lists the `/api/v2` operations that never answered 2xx (the report's
`v2_operations` holds the full split). `--v2-coverage-strict` makes that a failure on a full run.

Exit code is 1 on any failed check. Skips never fail a run.

## What it needs from the server

An instance-admin account. Every suite creates its own users, families and
media under unique `…-<suffix>@example.com` addresses and deletes them
afterwards; nothing it makes is meant to survive, and it never touches
accounts it did not create. The admin's own password is not changed.

The suite reads `GET /api/v1/server` first and skips the checks for any
feature the server reports as `false`. A server older than that endpoint
(404) is treated as the pre-discovery baseline: local auth, presigned
uploads, file sync and Subsonic on, everything else off.

## Suites

| Suite | Covers | Needs |
|---|---|---|
| `server` | `GET /server` shape, `404 not_found` vs `501 feature_unavailable`, features agree with `/auth/providers` | — |
| `smoke` | health, login, users, refresh tokens, library listing, streaming, progress, smart playlists, families, Subsonic | — |
| `families` | private vs family visibility, sharing, parental policies, who can stream what | — |
| `join` | link and QR invites, account claim, invite guardrails | — |
| `mobile` | cross-device play queue, incremental sync with tombstones, grouped browsing, bulk ops, push registration | — |
| `stats` | listening history batches, idempotent retries, device attribution, day bucketing, stats privacy | — |
| `trash` | 30-day trash: delete, list, restore, purge, permissions | `--compose-dir` (backdates rows through SQL) |
| `filesync` | the own.audio folder: paths, sync feed and cursor, shortcuts, holdings, auto-stored episodes | loopback `--base-url` and `--compose-dir` (serves an RSS feed the server fetches from this machine) |
| `gpodder` | the gPodder sync API: basic auth with the app key, devices, subscriptions both ways, play actions as progress and progress as play actions, removals | loopback `--base-url` and Docker (serves two RSS feeds the server fetches from this machine) |
| `v2` | `/api/v2`: discovery, problem details, sign-in grants, the catalogue, user state, sync, playback and the media route, images and pictures, the household, uploads, playlists | — |
| `v2_account` | `/api/v2` account and sign-in: sign-up, password change and reset, address confirmation, security events, two-factor, profile and picture, Subsonic key, deleting the account, token exchange, the code grant's checks, discovery's sign-in fields; that a server without mail says so | the mailed links' checks need `--compose-dir` |
| `v2_podcasts` | `/api/v2` podcast feeds and the directory: follow (idempotent, once per household), unfollow, refresh, auto-store, the back catalogue and single episodes kept on the server, unplayed counts and their paging, transcripts as timed lines, the directory's routes (or `501` where the catalogue is off), the change-feed entries of each | the feed part needs a loopback `--base-url` and a stack that may fetch from this machine (`-f conformance/compose.feeds.yml`); skipped otherwise |
| `v2_trash` | `/api/v2` deletion and the trash: delete a track or a book, batches with a result per item, `Idempotency-Key`, restore one or a batch (v1 batches too), purge, empty, who may (owner, household admin, another member, another household), paging, problem shapes, the change feed's removals and returns | the purge's object check needs `--compose-dir`; another household's ids need a server with several |
| `v2_books` | `/api/v2` books: a book's details, files (add many, rename, reorder, remove), cover, tags and identity; series and their books in one call; collections; contributors made, described, pictured and removed; who may (owner, household admin on a shared book, another member, a member who may not add, another household), `Idempotency-Key` on the creates, paging, the change feed and snapshot kinds `series` and `collection` | the identity match needs the household's book source to answer; skipped otherwise |
| `v2_household` | `/api/v2` household remainder: the household's photo and members' pictures (self or an admin), blocking (refused at sign-in and in `/sync/changes`, and let in again), leaving and the last admin (or `one_household` on a single-household server), accounts made by an admin, previewed without signing in and claimed (a code works once), regenerated codes, the household's settings, content reports and their paging; the change feed's `family` and `member` records; `Idempotency-Key` on the creates; who may (self, admin, another member, another household) | an expired code needs `--compose-dir`; leaving needs a server with several households |
| `v2_music` | `/api/v2` music remainder: track feedback (round trip, the listener's own change feed and its tombstone, paging), lyrics written as text or timed lines, a file's own tags and reading them back, the owner's duplicates (two copies of one file, paging), a sentence to a smart-playlist rule (`422 not_understood`), track and playlist covers, keeping a generated playlist, playlists and smart playlists in the change feed (edits, a shared track, the trash, tombstones) and the snapshot; who may (owner, another member, another household) | the identified-track refusal of a rescan needs `--compose-dir` |
| `v2_listening` | `/api/v2` listening state: the queue on two devices (`/sync/changes`, a `library` event on `/events`, `If-Match` and `412 revision_mismatch`, one queue with v1's), settings in milliseconds (refused out of range, read by v1, in the change feed), the notification inbox (paging, `ack` by ids and `up_to`, new and read ones in the change feed), push devices seen and removed only by their owner, track feedback in `POST /sync/push`, the snapshot's `queue` and `settings` | — |
| `v2_stats` | `/api/v2` statistics: plays in a batch and a retried one counted once, totals per kind and the most listened, day buckets in the listener's time zone, weeks and months, the plays history paged without loss, who sees one's figures (`stats_visible_to_admin`, an absent member, a managed account, the change feed), the household's figures for admins only, problem shapes | — |
| `v2_search` | `/api/v2` search across media and the lists' filters: books, podcasts, episodes and contributors found by title or a credited name, only what the caller can see, titles that start with the words first, `include=contributors`, the default still music only; `owner=me` and `visibility=private|family` on tracks, albums, books and podcasts, both together, the problem shapes, each filtered list paged without loss | — |
| `v2_walk` | `/api/v2` walked from `docs/api/openapi-v2.json`: the permission matrix, another member's (or household's) ids, problem shapes, paging without loss in every order | — |
| `library` | read-only library folders: scan, tags, books as folders in natural order, streaming from a folder, removing hides | a server with folders configured: `tools/make_library_fixtures.sh /tmp/oa-library` and `-f conformance/compose.library.yml`; skipped otherwise |

## Writing a suite

A module in `suites/` with `NAME`, a docstring whose first line is the
summary, optionally `REQUIRES = {"db"}` or `{"local"}`, and `run(ctx)`.
Everything goes through `Ctx` in `core.py`: `call`, `check`, `skip`,
`make_user` (cleans up after itself), `upload_track`, `presign_upload`,
`sql`, `on_cleanup`. A song the suite keeps using should carry `Ctx.tone()` (real
audio): the server's fingerprint job quarantines `Ctx.audio()`'s random bytes,
which hides them from every list part-way through a run. Raise `Skip` to stop a suite cleanly; any
`AssertionError` (including `ApiError` from an unexpected status) aborts the
suite as a failure and still runs its cleanups.

Rules:

- A check tests the contract, not an implementation detail. If it only
  holds on one edition, gate it on `ctx.feature(...)`.
- Labels are stable: CI diffs them between runs.
- No credentials in this directory, ever. They come from the command line.

## History

Consolidated 2026-10-06 from eight scripts in the original repository
(`backend_smoke_test`, `family_sharing_test`, `family_join_test`,
`mobile_sync_test`, `stats_test`, `trash_test`, `filesync_test`;
`seed_test_family` stayed a tool). The load test was not carried over.
