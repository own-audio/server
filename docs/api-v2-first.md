# API v2 first

Rule for every own.audio client and for the server (Kornel, 2026-11-17): **new client code talks
to `/api/v2`.** `/api/v1` is used only where v2 has nothing yet, and then on purpose, with a
comment saying so. v1 stays served, unchanged, until our apps have moved; then it goes
(`api-v2-plan.md`, decision 1).

What v2 is for and the rules every v2 endpoint follows: `api-v2-rules.md`.
The contract: `docs/api/openapi-v2.json`. v2 is **stable since server 1.0.4**, the release that
completed `api-v2-completion-plan.md` (it said `draft` before, additive only all the same); a
client checks the revision in `GET /.well-known/own-audio` → `api[]` before relying on
something added later (revision 15 at 1.0.4). The decisions in `docs/api-v2-plan.md`, the server code in `backend/src/v2/`.
The working client to copy from is `OwnAudioKit` in `music-ios`, `book-ios` and `podcast-ios`
(`Packages/OwnAudioKit/Sources/OwnAudioKit/`): music, books and podcasts on v2.

## What v2 does differently

1. **Discovery:** `GET /.well-known/own-audio` (public) — server id, name, edition, version, API
   versions, modules, sign-in methods, addresses, limits. It replaces `GET /api/v1/server` for a
   v2 client. Branch on modules and features, never on `edition`.
2. **Sign-in:** `POST /api/v2/auth/token` (grants: password, refresh, device code, provider
   assertion, a provider's code with PKCE for desktop apps, token exchange for a helper
   process; `auth.grants` in discovery lists them), `POST /api/v2/auth/device` (RFC 8628 — a
   TV's or speaker's code sign-in), `POST /api/v2/auth/revoke`. Two-factor is the problem code
   `mfa_required`; wrong credentials `invalid_grant`. Sessions are under `/me/sessions`; the
   account (sign-up, password, two-factor, picture, Subsonic key) under `/auth/register`,
   `/auth/password-reset` and `/me`.
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

## Sign-in and token errors

These shapes are part of the contract (gaps plan §0) and stay as they are:

- **`POST /auth/token`** answers OAuth errors (RFC 6749 §5.2) in problem shape: `code` and
  `error` carry the same word, `detail` and `error_description` the same text.
  - `400 invalid_grant`: wrong email or password, a refresh token that is unknown, used or
    revoked, a wrong two-factor code, an approving account that is no longer active. **A dead
    refresh token is `400 invalid_grant`, not v1's `401`**: sign in again.
  - `400 invalid_request` (a field missing, a body that does not parse, a provider this server
    does not accept), `400 unsupported_grant_type`.
  - The device code grant: `400 authorization_pending`, `slow_down`, `access_denied`,
    `expired_token` (RFC 8628).
  - `403 mfa_required` with an `mfa_token`: finish with the `mfa-otp` grant.
  - `429 account_locked` with `Retry-After` (too many wrong passwords for one account), and
    `429 rate_limited` from the per-address bucket.
- **Every other operation**, when the access token is missing, malformed, expired or revoked:
  `401 unauthorized`. Refresh once; if the refresh answers `invalid_grant`, sign in again.
- `401 invalid_credentials` comes from operations that check an account inside a signed-in
  request (an account deleted while its token is still valid); treat it like `unauthorized`.
  `403 invalid_credentials` is different: a password the person typed to confirm something
  (`PUT /me/password`, `DELETE /me`) is wrong. The session is fine; ask again.
- A server without mail answers `501 feature_unavailable` (with `feature`) from the routes
  that would mail a link; discovery says so beforehand (`auth.password_reset`,
  `auth.email_verification`).
- `403 insufficient_scope` (a personal access token without the operation's `x-scope`),
  `403 demo_read_only`, `403 email_unverified`, `403 forbidden` (a role the operation needs).

## Still on v1: nothing the apps need

Since 1.0.4 every v1 function is in v2 or recorded as not carried over
(`api-v1-v2-coverage.md`): per-member media policies, item grants and audience, sharing a playlist
with chosen members, and the payment provider's webhook stay on v1. Pictures are signed
`/images/{id}` links like every other `image` (catalog art never expires, people's pictures after
one to two weeks), and every signed media link points at `/api/v2/media`.

The v1 calls the iPhone apps still made on 2026-11-17, and what replaces each:

| v1 | v2 |
|---|---|
| Artwork uploads (v1 multipart) | `POST /uploads`, then `PUT /tracks/{id}/image`, `/books/{id}/image`, `/playlists/{id}/image`, `/contributors/{id}/image`, `/me/image`, `/family/image`, `/family/members/{user_id}/image` (or the image as the body) |
| `DELETE /api/v1/users/me` | `DELETE /me` (revision 3; asks for the password of an account that has one) |
| Statistics and who sees them | `GET /me/stats`, `GET /family/stats`; `stats_visible_to_admin` in `PATCH /me/settings`, and on a managed member in `PATCH /family/members/{user_id}` |
| `/api/v1/podcasts/discover/*`, `/api/v1/podcasts/search` | `GET /podcasts/directory`, `/podcasts/directory/categories`, `/search`, `/similar`, `/preview` (revision 4) |

The other areas that were v1-only are in v2 too: the trash (`/trash`), push devices
(`/me/push-tokens`), joining a household (`POST /family/join`, `/family/join/preview`), jobs and
administration (`/admin/*`), setup (`POST /setup`), uploads (`POST /uploads`). Moving a client's
calls over is the clients' work now, each in its own plan; v1 gets no `Deprecation` or `Sunset`
headers until they have moved (`API_COMPATIBILITY.md` §5).

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
