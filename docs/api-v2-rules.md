# API v2 — aim and rules

What `/api/v2` is for, what it does differently from `/api/v1`, and the rules every v2 endpoint
follows. Written 2026-11-17, when Kornel decided that v2 takes over every v1 function
(`api-v2-completion-plan.md`). The full design, with the reasons and the rejected alternatives,
is `design/DESIGN.md` in the `own-audio-api` repository; this page is the short form for
whoever adds or moves an endpoint.

## Aim

v1 grew endpoint by endpoint, alongside the apps. It works, but each medium got its own shapes:
three progress models, names used as ids, seconds in one place and milliseconds in another,
several error shapes, and lists that load everything. v2 is the same server's functions in one
consistent model, so that:

1. **One model covers a household's audio.** Music, audiobooks and podcasts; what each person
   keeps private and what the family shares; progress, bookmarks and favourites on every device.
2. **Apps work offline first.** A device keeps a copy of the catalog and its user's state,
   learns what changed (deletions included), and sends what it changed while offline.
3. **It is secure by construction.** No credential in a URL, short-lived tokens, every route
   declares its permission and the server checks it every time.
4. **It is built from common standards,** so every platform's libraries work unchanged.
5. **It scales to one family with a huge catalog** (600,000 songs, `CAPACITY.md`) without
   costing server memory.
6. **It is documented and tested.** Untested behavior is not part of the API.

v2 **replaces** v1. When every v1 function is in v2 and our apps have moved, v1 is removed.
Until then v1 is served unchanged for the apps built on it and gets no new features
(`api-v2-first.md`).

## What v2 does differently

| | v1 | v2 |
|---|---|---|
| Discovery | `GET /api/v1/server` | `GET /.well-known/own-audio`: id, versions, modules, sign-in methods, addresses, limits |
| Sign-in | separate routes per method | one OAuth 2.0 token endpoint, `POST /auth/token`, with grants: password, refresh, device code (RFC 8628), provider id token, two-factor |
| Ids | some names as ids (artists, albums) | a stable, opaque UUID for every entity; renaming keeps the id |
| Errors | `{error: "..."}` in several shapes | RFC 9457 problem details everywhere, with a stable `code` |
| Lists | often the whole list | cursor-paged where they grow with the catalog: `limit` (≤ 500), `cursor`, `{items, next}` |
| Records | names copied into every record | normalized: ids and credits, `include=` embeds referenced records once |
| Time | seconds and milliseconds mixed | milliseconds; RFC 3339 UTC timestamps |
| Progress | one model per medium | one model: `me/progress/{kind}/{id}`, last position per device |
| Sync | per-medium change lists | one change feed with tombstones (`/sync/snapshot`, `/sync/changes`), live events (`/events`) |
| Playback | stream URLs and presigned links per medium | `POST /playback-info` with the device's capabilities → a signed link for one item, user and format; transcoding decided by the server |
| Sharing | several words and rules | two words: `private` (not shared with the family) and `family` |
| Writes | retries may duplicate | `Idempotency-Key` on creates and paid work |
| Permissions | checked in handlers | declared per route as `x-permission`, enforced by extractors, tested by a matrix |

## Rules for every v2 endpoint

1. **One shape for one thing.** Reuse the existing v2 kinds, records, progress model and sharing
   words; never add a second shape for something v2 already models.
2. **Declare the permission** (`x-permission`: `public`, `user`, `member`, `family_admin`,
   `instance_admin`, `can_upload`, `can_generate`, `owner`). A route without one fails a test.
3. **Errors are problem details** with a stable `code`. `404` means missing *or not visible to
   you*, never "forbidden".
4. **Every list that can grow with the catalog is cursor-paged** (with the catalog, a history or
   time) and answered in SQL with an index; no `fetch_all` over a catalog table. Lists bounded by
   the household (members, invites, sessions, tokens, push devices, library folders, settings)
   may be arrays. (Kornel, 2026-11-17.)
5. **Ids are UUIDs, times are milliseconds, keys are `snake_case`,** timestamps RFC 3339 UTC.
6. **Credentials only in headers and bodies.** Media and images go through signed links.
7. **Changes reach the change feed.** Anything a client caches writes to the change log, so
   `/sync/changes` delivers it, deletions as tombstones.
8. **Optional features are discovered.** A function an operator can switch off is a module in
   `/.well-known/own-audio`; when it is off, the route answers a problem, not a fake success.
9. **Creates and paid work take `Idempotency-Key`.**
10. **Additive only once released.** New endpoints and optional fields are fine, with a revision
    bump (`V2_REVISION`); removing, renaming or retyping is not. `scripts/check-api-contract.py`
    checks it.
11. **Tested before it is part of the API:** a handler test, and conformance checks for success,
    each permission level (anonymous, member, admin, another family), the problem shapes, and
    paging without loss or repeats where it lists.
12. **Documented in the same commit:** the OpenAPI annotations (`docs/api/openapi-v2.json`,
    regenerated), the `CHANGELOG.md` line, and the coverage table (`api-v1-v2-coverage.md`) when
    a v1 function moves.

## Moving a v1 function

- Move the **function**, not the route: if v2 already has a resource for the thing, add to it
  rather than copying v1's path. Several v1 routes often become one v2 resource.
- Fix what v1 got wrong on the way (a name used as an id, seconds, a list without paging), and
  say so in the coverage table.
- A v1 function that is deliberately not carried over is recorded in `api-v1-v2-coverage.md`
  with the reason (today: per-member policies, access and grants, and the payment provider's
  webhook).
- v1 keeps working until it is removed: a move never changes v1's behavior.

Not covered by these rules: OpenSubsonic (`/rest`) and gPodder (`/api/2`). They serve third-party
players and keep their own versioning.
