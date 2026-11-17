# API v2 first

Rule for every own.audio client and for the server (Kornel, 2026-11-17): **new client code talks
to `/api/v2`.** `/api/v1` is used only where v2 has nothing yet, and then on purpose, with a
comment saying so. v1 stays served, unchanged, until our apps have moved; then it goes
(`api-v2-plan.md`, decision 1).

The contract: `docs/api/openapi-v2.json` (still `2.0-draft` — it may change until the first app
ships on it), the decisions in `docs/api-v2-plan.md`, the server code in `backend/src/v2/`.
The working client to copy from is `OwnAudioKit` in `music-ios`, `book-ios` and `podcast-ios`
(`Packages/OwnAudioKit/Sources/OwnAudioKit/`): music, books and podcasts on v2.

## What v2 does differently

1. **Discovery:** `GET /.well-known/own-audio` (public) — server id, name, edition, version, API
   versions, modules, sign-in methods, addresses, limits. It replaces `GET /api/v1/server` for a
   v2 client. Branch on modules and features, never on `edition`.
2. **Sign-in:** `POST /api/v2/auth/token` (grants: password, refresh, device code, provider
   assertion), `POST /api/v2/auth/device` (RFC 8628 — a TV's or speaker's code sign-in),
   `POST /api/v2/auth/revoke`. Two-factor is the problem code `mfa_required`; wrong credentials
   `invalid_grant`. Sessions are under `/me/sessions`.
3. **Errors:** always `application/problem+json`: `type`, `title`, `status`, a stable `code`,
   `detail`, `errors[]` per field. Branch on `code`, never on text. `404` means missing *or not
   visible to you*.
4. **Lists:** cursor-paged — `limit` (default 100, at most 500), opaque `cursor`, answer
   `{items, next}`. Follow `next` until it is null. No offsets.
5. **Records:** normalised — ids and `include=` instead of copied names; credits with roles.
   Durations and positions in **milliseconds**, timestamps RFC 3339 UTC, `snake_case`.
6. **Library sync:** `GET /sync/snapshot` (NDJSON) once, then `GET /sync/changes` with
   tombstones; `410 sync_cursor_expired` means take a snapshot again. `GET /events` (SSE) says
   something changed.
7. **User state:** `me/progress/{kind}/{id}` (ms, finished, last position per device),
   `me/favourites`, `me/ratings`, `me/bookmarks`, plays. Progress conflicts go by timestamp;
   favourites and ratings apply in the order sent.
8. **Playback:** `POST /api/v2/playback-info` with the device's capabilities (and `startMs`)
   answers a signed link bound to that item, user and format. Never build stream URLs by hand.
9. **Writes:** uploads carry their size; creates and paid work take an `Idempotency-Key`.
10. **Visibility** is `private` or `family`. Private means "not shared with the family" — never
    a lock icon.

## Still on v1 (check `openapi-v2.json` before relying on this list)

As of 2026-11-17 the iPhone apps call v1 only for: artwork, deleting the account
(`DELETE /api/v1/users/me`), stats and their visibility, and podcast discovery and directory
search (`/api/v1/podcasts/discover/*`, `/api/v1/podcasts/search`). Other v1-only areas: trash,
push devices, family join links, jobs, setup and admin, v1 multipart uploads. When v2 gains one
of these, move the call over. The full list of what the clients still miss, and the order
to close it in: `api-v2-gaps-plan.md`.

Not part of this rule: the OpenSubsonic API at `/rest` and gPodder sync at `/api/2`. They serve
third-party players and keep their own versioning.

## For the server

A new endpoint a client needs goes into `/api/v2` (with its `x-permission`, problem details,
cursor paging and an `openapi-v2.json` update) — not into v1. v1 changes stay additive and only
for clients already shipped against it (`API_COMPATIBILITY.md`). A gap that makes a client fall
back to v1 is worth closing in v2 rather than working around.

## For a client still on v1

`audio2-mac` (and through its submodule `audio2-tvos`), the Android apps, `audio2-win` and
`audio2-sync` were built on v1. In them:

- New features call v2.
- Code you touch for another reason moves to v2 when the move is small; a large move (sign-in,
  sync, playback) gets its own plan, phase by phase, and Kornel's go-ahead.
- Don't add new v1 calls when v2 has the endpoint.
- Shared packages: changing the networking layer of `audio2-mac` changes the Mac app and the TV
  app at once — build and test both.
