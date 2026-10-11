# /api/v2 plan

Step 3 of the own.audio API's order of work (`own-audio/api` design/DESIGN.md §14): the
`/api/v2` router on the entities of `docs/data-model-plan.md`. Written 2026-10-11.

## Decisions

1. **Beside v1, not on top of it.** `/api/v2` is its own router (`backend/src/v2/`) with its own
   OpenAPI document, `docs/api/openapi-v2.json`. Handlers reuse the `db` layer and the domain
   code; they do not call v1 handlers. v1 stays unchanged until our apps have moved, then goes
   (DESIGN §4).
2. **Draft until our apps ship on it.** Until then v2 may change in any way; its document says
   `2.0-draft` and the contract check (`scripts/check-api-contract.py`) leaves it alone. When
   the first app ships, it becomes `2.0` and the additive rule applies as it does to v1.
3. **Problem details for every error** (RFC 9457, `application/problem+json`): `type`, `title`,
   `status`, a stable `code`, `detail`, and `errors[]` for fields. One middleware on `/api/v2`
   turns every error answer into that shape — the handlers' own, the extractors' rejections
   (`401`, `403`), the rate limiter's `429`, axum's body rejections — so nothing slips through
   in another shape. `404` means missing or not visible.
4. **Every route declares its permission** in its OpenAPI operation as `x-permission`: `public`,
   `user`, `member`, `family_admin`, `instance_admin`, `can_upload`, `can_generate`, `owner`.
   The extractors enforce it; a unit test fails on a v2 operation without a declaration, and the
   conformance `access` suite checks each declaration against the server's answers (anonymous,
   member, admin). Security hardening plan §5.3.
5. **Lists are cursor-paged**: `limit` (default 100, at most 500), opaque `cursor`, answer
   `{items, next}`; a stable sort with the id as the tie-breaker. No offsets.
6. **Normalised records.** A track carries `album_id` and credits (contributor id, role,
   position, join phrase), never names it does not own; `include=` embeds the referenced
   records once per answer. Durations and positions in milliseconds, timestamps RFC 3339 UTC,
   `snake_case`.
7. **One discovery document**, `GET /.well-known/own-audio`, public: server id, name, edition,
   version, API versions, modules, sign-in methods, addresses, limits. It replaces
   `/api/v1/server` for v2 clients.

## Phases

Status 2026-10-11: V1 (`49b78c2`), V2 (`ab2e7b8`), V3a (`876e925`), V3b (`7d9bc8a`), V3c (`1459cff`) V4 (`e6b1021`), V5 (`2854060`) and V6 (`c504a49`, plus images) V7 (household) and V8 (events: `library` only — jobs carry no owner yet) and V9 (uploads with a required size, `Idempotency-Key`, limits in discovery) are done. The hosted edition mounts its billing, narration and translation handlers under `/api/v2` through `finalize_with` (`6c48864`); their bodies are still v1's. Next: step 4, the apps. Per-member media policies and item grants stay on v1 until a client needs them on v2. Images are addressed by their stored object, which never changes once written, rather than by content hash: same caching, without waiting for the checksum job. Series and collections as resources of their own wait for V7, with the household. V4 keeps v1's tables (one or two per medium) behind one model; they become one table each when v1 is removed. Favourites and ratings pushed offline apply in the order sent; only progress compares timestamps.

What the first app (own.audio Music) still needs from v2, so it can stop using the server's
Subsonic API for own.audio: playlists (list, create, edit, order), one search across artists,
albums and tracks, random tracks, play counts and "recently played" / "most played" orders,
lyrics, and similar tracks (instant mix). Until then the app takes those from Subsonic, whose
ids are v2's ids.

| Phase | Content | Test |
|---|---|---|
| **V1** | Router, problem details, `x-permission` and its test, cursor paging, `.well-known/own-audio`, `openapi-v2.json`, `GET /me` | conformance `v2`: problem shape on 401/404/422/429, every operation declares a permission, discovery |
| **V2** | Sign-in in the standard shape: `POST /auth/token` (password, refresh, device code, provider assertion), `POST /auth/revoke`, `POST /auth/device` (RFC 8628 wire format), sessions under `/me/sessions`, personal access tokens with scopes | conformance: every grant, refresh reuse revokes, RFC 8628 errors, a scoped token is refused outside its scope |
| **V3** | Catalogue: contributors (with `/artists`, `/authors`, `/narrators`, `/hosts` views), release groups, albums, tracks, recordings, genres, books, podcasts, episodes; credit editing; the E4 corrections in v2 shape | conformance: lists page without loss or repeats, visibility, cross-family |
| **V4** | User state: one progress model (ms, finished, last position per device), favourites, ratings, play events, bookmarks; `POST /sync/push` | conformance: merge rules of DESIGN §7.6 |
| **V5** | Sync: the 0078 change log generalised to every kind and to user state; `GET /sync/changes` with tombstones; `GET /sync/snapshot` (NDJSON); `410 sync_cursor_expired` | conformance: snapshot + changes reproduce the library; a rename is one change row |
| **V6** | Media: `POST /items/{id}/playback-info` (direct/remux/transcode on the existing planner), signed links bound to item, user, format and key id, images by content hash | conformance: ranges, `416`, a link for one item refuses another |
| **V7** | Household: family, members, invites, permissions; visibility in two words (`private`, `family`) | conformance `isolation` on v2 ids |
| **V8** | Events: `GET /events` (SSE) from the change log, heartbeat, `Last-Event-ID` | conformance: a change made elsewhere arrives |
| **V9** | Writes and limits: uploads in v2, `Idempotency-Key` on creates and paid work, the required size on uploads (hardening C1), documented maxima | conformance: a repeated key answers the first result |

The hosted edition's routes (billing, narration, translation) follow in `audio2` once V2 is in.
Then step 4: own.audio Music on v2, the book and podcast apps, the console; v1 removed after.
