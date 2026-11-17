# /api/v2 completion — every v1 function in v2

Kornel's decision of 2026-11-17: every `/api/v1` function moves to `/api/v2`, with the
optimisations that pay off on the way, and everything tested. v2 is declared stable only when
this is done (phase 14). Written the same day from the audit in `api-v1-v2-coverage.md`
(141 covered, 19 partly, 149 missing, 7 not carried over), the clients' needs in
`api-v2-gaps-plan.md` and the decisions in `api-v2-plan.md`.

**Every endpoint follows [`api-v2-rules.md`](api-v2-rules.md)** — the aim, what v2 does
differently, the twelve rules and how to move a v1 function. This plan does not repeat them; it
says what to build, in which order, and how we know it is done. Where the plan departs from a
rule, it says so in §1.4.

Paths below drop the `/api/v1` and `/api/v2` prefixes, as the coverage doc does.

---

## 1. Scope and rules

### 1.1 What "done" means

- Every row of `api-v1-v2-coverage.md` is **covered** or **not carried over** with a reason.
  No `missing`, no `partly`. `scripts/check-v2-coverage.py` (§4.3) proves it.
- The hosted edition's rows count as covered only when their bodies follow the rules (§3,
  phase 13). Today they answer under `/api/v2` with v1 bodies; the audit counts them covered,
  this plan treats them as partly.
- Nothing in v2 depends on a v1 route. Three places do today:
  - `storage/links.rs` `MEDIA_PATH = "/api/v1/media"`: v2's playback redirects and upload targets
    for local storage (and S3 behind `STORAGE__PROXY`) are v1 links.
  - `families/mod.rs` builds `avatar_url` as `/api/v1/family/avatar` and
    `/api/v1/users/{id}/avatar`, and `v2/household.rs` hands them out as `Family.image` and
    `Member.image`. Those are token-protected v1 routes, not signed `/images/{id}` links; the
    audit marks them covered, but they are not. (`OwnAudioKit.picture()` already fetches a full
    link as it is, so switching is safe for the iPhone apps.)
  - The conformance `access` and `isolation` suites walk `openapi.json` (v1) only.
  All three are fixed in phase 0.
- v1 keeps working unchanged (rules, "Moving a v1 function"). Its suites keep passing after
  every phase.
- After phase 14, `v1` is still served; removing it is a separate step with its own deprecation
  (API_COMPATIBILITY.md §5) once the apps have moved.

### 1.2 The seven exclusions

They stay, for the reason already given in `api-v2-plan.md`:

| v1 | Reason |
|---|---|
| `GET`/`PUT /family/content/{kind}/{item_id}/audience` | Per-member access model stays on v1 |
| `POST /family/content/{kind}/{item_id}/request-access` | Same model |
| `GET /family/members/{user_id}/access` | Per-member policies |
| `PUT /family/members/{user_id}/grants` | Item grants |
| `PUT /family/members/{user_id}/policy` | Per-member media policies |
| `POST /billing/stripe/webhook` (hosted) | Stripe calls it, not a client |

One thing to revisit, not to change now: the gaps plan lists "per-member policies, grants and
audience" as wanted by Windows and Android Books (`(W Ab)`). If either client really needs them,
this becomes a phase of its own after 14; it would bring `content_grants` into the change feed
anyway (phase 3 adds the trigger already, see below). Question 1.

Two more rows are proposed as **not carried over** (question 2):

| v1 | Reason |
|---|---|
| `POST /music/tracks/discs` | A one-off back-fill of disc numbers for old uploads. It becomes a background job that runs once after the upgrade; no client needs to trigger it. |
| `PUT /music/playlists/{id}/share` (members part), `GET /music/playlists/{id}/audience` | Only if Kornel counts sharing a playlist with chosen members as part of the per-member access model. Otherwise they move in phase 6. |

### 1.3 How each phase is shaped

Each phase is one to five commits on `main`, each commit complete on its own:

1. Handlers in `backend/src/v2/`, calling the domain code, never v1 handlers
   (`api-v2-plan.md` decision 1).
2. Migration when needed (forward-only, numbered from 0126).
3. `OPENAPI_WRITE=1 cargo test openapi`; `V2_REVISION` up by one per commit that adds to the
   contract.
4. Tests to the bar of §4.
5. `CHANGELOG.md` line under `[Unreleased]` naming the operations and the v2 revision; the rows in
   `api-v1-v2-coverage.md` flipped; the line in `api-v2-gaps-plan.md` ticked.
6. If the new route writes, it is checked against the demo's v2 allow-list (`demo.rs`
   `v2_allows`): listening state is allowed, editing is refused.

### 1.4 Where this plan departs from `api-v2-rules.md`, and what is already in the tree

1. **Rule 4, "every list is cursor-paged".** Several v2 lists already answer a bare array:
   `/family/members`, `/family/invites`, `/me/sessions`, `/me/tokens`,
   `/smart-playlists/presets`, the credit and chapter lists. They are bounded by the household,
   not the catalog, and the iPhone apps read them, so changing them would not be additive. The
   plan keeps bounded lists (members, invites, sessions, tokens, push devices, library folders,
   settings) as arrays and pages everything that grows with the catalog, the history or time.
   Decided (Kornel, 2026-11-17): rule 4 reworded in phase 2 — lists that grow with the catalog
   are paged, lists bounded by the household may be arrays.
2. **Rule 11, "a handler test".** The repo has no database-backed Rust tests: `cargo test` runs
   in CI without PostgreSQL, and behaviour is proved by the conformance suite. §4.1 defines what a
   handler test is here. Kornel (2026-11-17): handler tests may use a real PostgreSQL
   (`#[sqlx::test]`), with PostgreSQL added to the CI test job by the first phase that writes
   one. Phase 0 needed none (its logic is tested without a database, its behaviour by the
   conformance suite).
3. **Rule 6, credentials only in headers and bodies.** `GET /auth/device/{user_code}` already has
   the user code in the path (RFC 8628's own pattern; approving needs a signed-in user). Kept.
   New routes put invite and claim codes in the body (phase 7).
4. **Rule 7, changes reach the change feed.** Playlists and smart playlists are in v2 but not in
   the change log. Phase 6 fixes that (done: kinds `playlist`, `smart_playlist`).
5. **Rule 1 in the hosted edition.** Translation progress has its own route
   (`PUT /translation/episodes/{id}/progress`) beside `/me/progress`. Phase 13 fixes that.
6. **Rule 10 and the hosted bodies.** The iPhone apps on the App Store already call
   `/api/v2/narration`, `/summaries` and `/family/billing` with today's v1-shaped bodies, so those
   bodies cannot simply be replaced. Phase 13 fixes them additively where it can and lists the
   rest as known deviations. Question 5.
7. **The "v2 is stable" change.** Decided (Kornel, 2026-11-17) and committed with phase 0: the
   revision counter (`V2_REVISION`, in discovery and as `2.<revision>-draft` in
   `openapi-v2.json`) and the additive check (`check-api-contract.py` compares v2 too) are in,
   because shipped apps use v2 and must not break; `V2_STATUS` stays `draft` until phase 14, and
   every document says v2 "becomes stable at the release that completes this plan".

---

## 2. Optimisations worth doing on the way

Only those that save something measurable. Each lands in the phase named.

| # | What | Why, and what it saves | Phase |
|---|---|---|---|
| 1 | **`include=` and several kinds on `GET /me/progress`** (`kind=book,episode&include=items`) | Continue listening today is one call per kind plus one per item to get its record: about 3 + 20 requests on the TV and Android. One request instead, the records embedded once in `included`. Replaces v1 `/library/continue`. | 3 |
| 2 | **`kinds=` on `GET /sync/changes`** (the snapshot has it already) | A podcast-only or book-only client stops reading and decoding every track change. With 600k tracks a re-tag or an identify run is thousands of rows it throws away. Filter in the SQL (`kind = ANY($n)`), not after. | 3 |
| 3 | **An ids-only snapshot** (`GET /sync/snapshot?fields=ids`: `{kind, id, version}` per line) | The sync core's reconcile needs ids, not records: about 50 bytes a line instead of about 1 KB, so roughly 30 MB instead of 600 MB at 600k tracks, and no record assembly on the server. Replaces v1 `/sync/tree/ids`. | 3 |
| 4 | **Permanent image links for catalog art** | `/images/{id}` addresses an object that never changes; only the signature expires (weekly). Links kept by a cache, a widget or the car die after one to two weeks, and the change feed does not resend unchanged records. A signature over the id alone, no expiry, for covers and artist pictures: links survive for good, CDNs and `URLSession` cache them `immutable`, records never have to be re-read to refresh art. People's pictures (family, member, account) keep the weekly expiry. Question 7. Old links keep validating. | 0 |
| 5 | **`/stream` serves local storage itself** | For local storage the v2 stream link redirects to a signed `/api/v1/media` link: two round trips before the first byte, on every play and every seek that reopens. Serving the bytes from `/stream` (same range code as `http::media`) removes one. S3 keeps the redirect. | 0 |
| 6 | **One search across all media** (`GET /search?types=…,books,podcasts,episodes`) | TV and Windows search screens make one request instead of three or four, and the server ranks everything one way. Books (1,000) and episodes (tens of thousands) are small next to tracks; measured on the scale catalog before any new index. | 10 |
| 7 | **Batch where clients loop** | `POST /trash` with up to `batch_max` items (deleting an album is one call and one restorable batch, not twenty); `POST /books/{id}/files` with many uploads; `PUT /books/{id}/files/order` and `PUT /series/{id}/books` as one ordered replace instead of add/remove per item; `POST /me/notifications/ack` with `up_to`. | 4, 5, 8 |
| 8 | **Merge v1's near-duplicates into one resource** | Fewer routes to test and document, one shape per thing (rule 1). See the table below. | throughout |
| 9 | **User state through the change feed and `/events`** | Queue and notifications written to the change log as user kinds (`queue`, `notification`) reach other devices through `/sync/changes` and the existing `library` event, instead of every device polling `GET /playback/queue` and the inbox. `/events` already watches the log per user; nothing new to run. | 8 |
| 10 | **Image credit inside the record** | v1 needs `GET …/image-info` per artist page; v2 puts `image_credit` (source, author, licence, link) on `Contributor`. One request less per page. | 5 |
| 11 | **Unplayed counts in one grouped query per page** | One `GROUP BY feed_id` over the page's podcasts, with an index on episodes `(feed_id, published_at)` and progress `(user_id, episode_id)`, rather than a count per show. | 2 |
| 12 | **Indexes for the new lists, only where `EXPLAIN` on the scale catalog asks** | Candidates: trash `(family_id, deleted_at DESC, id)`, security events and notifications `(user_id, created_at DESC, id)`, plays `(user_id, played_at DESC, id)` for history and stats, `(owner_id, id)` on tracks and books for the "private" filter. Each is added with the query it serves and its timing goes into `CAPACITY.md`. `pg_trgm` stays out until a measurement asks for it (CAPACITY.md gap 8). | 4, 7–10 |

Merges (optimisation 8):

| v1 | v2 |
|---|---|
| 4 TOTP routes (`GET`, `setup`, `enable`, `DELETE`) | `two_factor` on `GET /me`; `POST`, `PUT`, `DELETE /me/two-factor` |
| Upload cover/picture for track, book, playlist, artist, author; family, member and own avatar; companion file "use as cover" | One pattern: `PUT /{resource}/image` with `{upload_id}` or `{file_id}`, `DELETE /{resource}/image` |
| Artist/author picture "forget and look again" (two routes) | `DELETE /contributors/{id}/image` |
| `GET /playback/settings`, `PUT …/audiobook-defaults`, `GET`/`PUT …/eq` | `GET`/`PATCH /me/settings` |
| `GET`/`PUT /family/settings/audio-analysis`, `…/book-source` | `GET`/`PATCH /family/settings` |
| Block, unblock member | `blocked` in `PATCH /family/members/{id}` |
| Lock, unlock family; families and families-in-trash lists | `PATCH /admin/families/{id}` `{locked}`; `GET /admin/families?state=` |
| `/podcasts/discover/similar` and `/podcasts/{id}/similar` | `GET /podcasts/directory/similar` |
| `POST /podcasts/{id}/sync-images` | Part of `POST /podcasts/{id}/refresh` |
| `GET /stats/me/history` | `GET /me/plays` (the plays v2 already records) |
| Series and collection add/remove per book | One ordered `PUT` of the list |
| `GET /setup/status`, the demo in `GET /server`, `/auth/providers` client ids | Fields of `/.well-known/own-audio` |
| `/auth/fork` | The token-exchange grant (RFC 8693) on `POST /auth/token` |
| Google/Microsoft desktop code exchange | The `authorization_code` grant with PKCE on `POST /auth/token` |

Not worth it now: ETags on v2 lists (sync makes them moot), a trigram index (not measured as
needed), pagination of the bounded household lists (§1.4).

---

## 3. Phases

Sizes: **S** one commit, **M** two or three, **L** four or five. "v1 ops" counts the coverage
rows the phase closes (missing + partly); the sum is 168 = 149 + 19.

| # | Phase | v1 ops | Size | Unblocks |
|---|---|---|---|---|
| 0 | Groundwork: v2 media path, people's pictures, image links, test tools — **done** | 2 | M | everything; v1 removal |
| 1 | Account and sign-in — **done** | 23 | L | W Ab Am Ap M |
| 2 | Podcast feeds and directory — **done** | 16 | L | M T W Ap |
| 3 | Sync essentials and continue listening — **done** | 3 | M | S (so M W), A, T |
| 4 | Deletion and trash — **done** | 7 | M | M W Am, S |
| 5 | Books: editing, files, series, collections, tags, authors, images — **done** | 34 | L | W Ab M |
| 6 | Music remainder and playlists in sync — **done** | 17 | M | M W Am |
| 7 | Household remainder — **done** | 18 | L | M W Ab |
| 8 | Listening settings, queue, notifications, push — **done** | 11 | M | M T W A |
| 9 | Statistics — **done** | 5 | S | M W A |
| 10 | Search across media, list filters — **done** | 2 | S | T W |
| 11 | File sync remainder — **done** | 11 | L | S M W |
| 12 | Instance admin, setup, jobs, library folders — **done** | 18 | L | W, console |
| 13 | Hosted edition (`audio2`) — **done** | 1 + 23 reworked | L | W A, apps |
| 14 | Release: stable, docs, strict coverage | — | S | — |

Permissions below use the `x-permission` words; "self" means the caller's own record, checked in
the handler.

### Phase 0 — groundwork (M) — done 2026-11-17

Done, v2 revision 2: `GET`, `HEAD`, `PUT /api/v2/media`, every signed media link pointing there
(v1's route takes the same links); `/stream` sending what the server keeps itself; permanent
links for catalog art, weekly ones for people's pictures, `Family.image` and `Member.image`
among them; a new storage object per uploaded picture, so an image never changes under its
link; `scripts/check-v2-coverage.py` (report mode, in CI); the conformance suite `v2_walk`
(permission matrix, someone else's ids, problem shapes, paging) and the operation coverage
report (`run.py`, `--v2-coverage-strict` to fail on it); the auth error shapes in
`api-v2-first.md`. What the plan said, with these differences:

- The operation coverage check reports, it does not fail a run yet: a full run on the compose
  stack leaves 20 of 139 operations without a 2xx — parts of identify (listing runs,
  proposals and groups: their results come from MusicBrainz, which the stack cannot reach), the
  quarantine, album merge and release group, renaming the household, removing a member,
  `family/join`, `DELETE /me/sessions`, the visibility and credits of a book or podcast, and
  reads of an episode, podcast or recording by id.
  `--v2-coverage-strict` turns it into a failure; each phase adds the checks for what it
  touches, and CI switches to strict when the list is empty, by phase 14 at the latest.
- The "other family" walk runs on servers with several households (the hosted edition); on a
  one-household install (the compose stack, CI) the same walk runs with another member of the
  household against the owner's private items, plus every `owner` operation on what the owner
  shares.

Tables a client caches that do not write to the change log yet (each phase below adds its
triggers): `music_playlists` and `music_playlist_tracks`, `music_smart_playlists` (phase 6,
done);
`audiobook_series`, `audiobook_series_books`, `audiobook_collections`,
`audiobook_collection_books`, `audiobook_tags`, `audiobook_book_tags` (phase 5); `sync_paths`,
`content_grants`, `family_members` (phase 3, done); `companion_files_all`, `sync_shortcuts`
(phase 11); `play_queues`, `pending_notifications` (phase 8, done; with `user_settings` and
`eq_settings`), `device_push_tokens` (phase 8 decided against: see there).

v1 ops: `GET /media`, `PUT /media` (partly).

| v2 | Permission | Notes |
|---|---|---|
| `GET`, `HEAD`, `PUT /media?k&e&s` | `public` (the signature is the authorisation) | Same handler as v1's. The signature covers method, key and expiry, not the path, so v1 links still verify; `MEDIA_PATH` becomes `/api/v2/media`. Body limit disabled as on v1. |
| `GET /stream` | unchanged | Serves local-storage originals itself (optimisation 5). |
| `GET /images/{id}` | unchanged | Accepts the permanent signature form (optimisation 4); `images::link` hands it out for catalog art. |
| `Family.image`, `Member.image` | — | Signed `/images/{object_id}` links (weekly) instead of `/api/v1/…/avatar`. |

Also:

- `scripts/check-v2-coverage.py` (§4.3), in report mode until phase 14.
- Conformance: the v2 permission matrix and isolation walker over `openapi-v2.json` (§4.2) — v2
  has anonymous and member checks today, no other-family walk; a generic paging walker; operation
  coverage in `--report`.
- The v2 auth error shapes written into `api-v2-first.md` (gaps plan §0).
- A list of tables a client caches that are not in the change log yet (playlists, smart
  playlists, series, collections, book tags, `sync_paths`, `content_grants`, companion files,
  family members, queue, notifications); each phase below adds its triggers.

Tests: range requests, `416`, `PUT` upload and a `HEAD` on the v2 media path; an old
`/api/v1/media` link still plays; a v2 playback link on local storage answers 206 without a
redirect; a member's picture fetched without a token; a permanent image link works after its
weekly expiry would have passed (built with a past week in a unit test).
CHANGELOG: "`/api/v2/media` — the server's signed media route for v2; v2 links no longer point
at `/api/v1`." "`Family.image` and `Member.image` are signed image links."

### Phase 1 — account and sign-in (L) — done 2026-11-17

Done, v2 revision 3: every operation in the table below, the two grants, the discovery fields;
the 23 rows covered. Handler tests run through the v2 router with a database pool that points
nowhere (validation and problem codes, the grants' checks, the sign-in bucket, discovery), the
code grant's exchange against a stub provider; no `#[sqlx::test]` was needed, so the CI test job
is unchanged. The conformance suite `v2_account` covers behaviour. What the plan said, with these
differences:

- `DELETE /me/two-factor` takes a code or a recovery code, not the password: v1's rule, so a
  stolen session plus a guessed or phished password cannot remove the second factor.
- `DELETE /me` asks for the password only when the account has one; an account that signs in
  with a provider only sends no body (as v1, which asks for nothing).
- `PUT /me/image` takes `{upload_id}` (an image upload of any kind) or the image itself as the
  body, so a member who may not add to the library can still set a picture, as on v1.
- `GET`/`POST /me/subsonic-key` answer `{username, key}`.
- A server without mail answers `501 feature_unavailable` from `POST /auth/password-reset` and
  `POST /me/email-verification` (rule 8), where v1 answers `200`; discovery gains
  `auth.email_verification` beside `auth.password_reset`, and `auth.grants`.
- The routes that hand out or change a credential (password, two-factor, Subsonic key, deleting
  the account) refuse a personal access token (`403`), as `POST /me/tokens` does.
- The sign-in bucket also covers `PUT /me/password`, `PUT`/`DELETE /me/two-factor` and
  `DELETE /me` (v1 has no limit there beyond the API's).
- `Me` is not a change-feed kind; a renamed or re-pictured member reaches other devices through
  `family_members` in the change log (phase 3).
- On a server without mail (CI, the compose stack) `POST /auth/password-reset` and
  `POST /me/email-verification` answer `501` and never 2xx; without `--compose-dir` (CI today)
  the mailed links' checks are skipped too, so `…/password-reset/confirm` and
  `POST /auth/email-verification` join them. With an SMTP host set and `--compose-dir`, all of
  phase 1 answers 2xx (checked 2026-11-17).

v1 ops: `POST /auth/register`, `/auth/password`, `/auth/password/forgot`, `/auth/password/reset`,
`/auth/email/resend`, `/auth/email/verify`, `GET /auth/security-events`, the four TOTP routes,
`PATCH`/`DELETE /users/me`, `POST`/`DELETE /users/me/avatar`, `GET /users/me/subsonic-key`,
`POST …/regenerate`, `POST /auth/fork`; partly: `/auth/google`, `/auth/microsoft`,
`/auth/providers`, `GET /server` (demo), plus `GET /setup/status`.

| v2 | Permission | Request → answer |
|---|---|---|
| `POST /auth/register` | `public` (login rate bucket) | `{email, password, display_name, invite_code?}` → `201 Me`. `403 registration_closed` without an open registration or a valid invite. Signing in stays `POST /auth/token`. |
| `POST /auth/password-reset` | `public` | `{email}` → `202`, always, whether or not the address exists. |
| `POST /auth/password-reset/confirm` | `public` | `{token, password}` → `204`; ends every session. |
| `POST /auth/email-verification` | `public` | `{token}` → `204` (the console's landing page posts the mailed token). |
| `POST /me/email-verification` | `user` | → `202`; resends the mail. |
| `PUT /me/password` | `user`, scope `user:write` | `{current_password, new_password}` → `204`. |
| `GET /me/security-events` | `user` | Paged `{id, type, at, device, ip_hint}`. |
| `POST /me/two-factor` | `user` | → `{secret, otpauth_uri}` (pending until confirmed). |
| `PUT /me/two-factor` | `user` | `{code}` → `{recovery_codes}`; turns it on. |
| `DELETE /me/two-factor` | `user` | `{password}` or `{code}` → `204`. |
| `GET /me` | `user` | Adds `two_factor`, `image`, `recommendations_enabled`, `discovery_languages`. |
| `PATCH /me` | `user` | `{display_name?, recommendations_enabled?, discovery_languages?}` → `Me`. |
| `DELETE /me` | `user` | `{password}` → `204`; what v1 does to the library, unchanged. |
| `PUT`, `DELETE /me/image` | `user` | `{upload_id}` → `Me`. The image pattern (optimisation 8). |
| `GET /me/subsonic-key`, `POST /me/subsonic-key` | `user` | → `{key}`; `POST` makes a new one. |
| `POST /auth/token` | `public` | New grants: `urn:ietf:params:oauth:grant-type:token-exchange` (RFC 8693: `subject_token` = a refresh token → a new session chain for a helper process) and `authorization_code` with PKCE (`provider`, `code`, `code_verifier`, `redirect_uri`). |
| `/.well-known/own-audio` | `public` | Adds `auth.clients` (Google/Microsoft desktop and web ids, Apple web), `demo` (the public demo's sign-in), `setup.complete`. |

Migrations: none expected (v1's tables).
Tests: every route at each permission level; mail-based flows read their token through SQL
(`REQUIRES = {"db"}`, as `trash` does); two-factor end to end with the suite's existing TOTP
helper; reset ends other sessions; a changed password refuses the old one; token exchange gives a
working session that can be revoked on its own; register refused when closed, accepted with an
invite; problem codes `invalid_credentials`, `registration_closed`, `invalid_token`,
`weak_password`; the login bucket applies to register and reset. The PKCE grant is tested
against a stub provider in a unit test (no real Google in CI).
CHANGELOG: one line per group (sign-up, password, e-mail verification, two-factor, profile,
Subsonic key, grants, discovery fields).

### Phase 2 — podcast feeds and directory (L) — done 2026-11-17

Done, v2 revision 4: every operation in the table below and `POST /podcasts/refresh`; the 16 rows
covered. Handler tests through the v2 router (signed-in only, v1's outbound bucket on follow,
refresh and preview) and against a stub catalogue (languages one at a time, the browse offset);
unit tests for feed addresses, feed errors, the preview's order and the transcript's speakers.
The conformance suite `v2_podcasts` follows a feed it serves itself
(`conformance/compose.feeds.yml` lets a stack fetch it), and walks every write to the change
feed. Rule 4 is reworded (§1.4 item 1). What the plan said, with these differences:

- No `modules.podcast_directory`: the directory follows the two modules servers already report
  (rule 1) — `podcast_search` for search (the catalogue, or Apple's public search without it,
  as v1), `podcast_discovery` for categories, browse and similar. Off is `501
  feature_unavailable`; a directory that does not answer is `502 directory_unavailable` (v1:
  `500`). The preview needs neither: it reads the feed itself.
- Directory answers are `Page`s. Search, categories and similar are one page (no `cursor`: the
  directory answers its best, and categories are its fixed taxonomy); browse is cursor-paged,
  the catalogue's offset inside the cursor. `limit` 1–100 (25), similar 1–50 (20). Results carry
  `podcast_id` when the caller has the show; browse and similar leave those out, as v1.
- `POST /podcasts` is `x-permission: member` (was `can_upload`): following is any member's, as
  on v1; an uploaded show still needs `can_upload`, checked in the handler and by `v2_podcasts`
  (no longer by `v2_walk`'s `can_upload` sweep). Errors: `invalid_feed_url` (also an address the
  outbound guard refuses), `youtube_not_supported`, `feed_unreadable` (`422`; v1 answered `500`).
- `PATCH /podcasts/{id}` and `POST …/store` also need `can_upload`, and `PUT /episodes/{id}/stored`
  is `can_upload` (v1's rules). `DELETE /episodes/{id}/stored` is `owner` (the owner or a
  household admin, v1's rule), not `member`.
- Refresh and keeping an episode are queued (`202`) where v1 fetched during the request:
  `feed_refresh` (deduplicated; with the art when asked through v2) and `episode_download` (asked
  by name it stores even an episode whose copy was dropped before, as v1's download). The job
  worker is woken by these, so the work starts at once instead of at the next idle poll.
  `POST /podcasts/refresh` refreshes every visible show (Subsonic's `refreshPodcasts` did; v1's
  REST had nothing). An uploaded show answers `409 uploaded_podcast` to refresh, auto-store and
  store (v1: `204`, no-op, on refresh).
- Unfollowing writes each episode's removal to the change log in the same transaction: the
  cascade could not (the show is gone before its episodes). `Podcast` gains `source` and
  `auto_store`.
- `GET /me/podcasts/unplayed` lists every visible show once, zero included, paged by id. No
  migration: the existing indexes serve it (12 ms for a page of 101 shows over 100,200 episodes,
  `CAPACITY.md`).
- The transcript answers `{format: segments|text, segments, text?}` with `speaker` from WebVTT
  voices and Podcast Index JSON; `404 no_transcript`, `422 transcript_too_large`, `502
  transcript_unavailable`. v1's route is unchanged.
- CI: the conformance job adds `compose.feeds.yml` (the suite's feeds, so follow and refresh
  run there; if the runner cannot reach them the feed checks skip, never fail) and
  `compose.mail.yml` (a mailpit catcher: `POST /auth/password-reset` answers `202`, nothing is
  sent). `--compose-dir .` stays out: on the one-household stack the `trash` and `filesync`
  suites fail with it (they assume an outsider in another household: `POST /family/invites` →
  `403`, a shortcut to an outsider's member → `201`), so the mailed links' checks
  (`…/password-reset/confirm`, `POST /auth/email-verification`) still wait — phase 14, or
  whichever phase makes those two suites one-household-aware.

v1 ops: `POST /podcasts/subscribe`, `DELETE /podcasts/{id}`, `POST …/refresh`,
`GET /podcasts/unplayed-counts`, `PUT …/auto-store`, `POST …/store-all`,
`POST`/`DELETE …/episodes/{ep_id}/download`, `POST …/sync-images`, `POST /podcasts/search`,
`GET /podcasts/discover/browse`, `…/categories`, `…/preview`, `…/similar`,
`GET /podcasts/{id}/similar`; partly: `…/transcript`.

| v2 | Permission | Request → answer |
|---|---|---|
| `POST /podcasts` | `member` | Adds `{feed_url}` (follow) beside today's uploaded show; `Idempotency-Key`; an already-followed feed answers the existing `Podcast` with `200`. |
| `DELETE /podcasts/{id}` | `owner` or `family_admin` | Unfollow. For an uploaded show it answers `409` until phase 4 moves it to the trash. |
| `POST /podcasts/{id}/refresh` | `member` | → `202`; also stores the show's and episodes' art (was `sync-images`). |
| `PATCH /podcasts/{id}` | `owner` | `{auto_store}`. |
| `POST /podcasts/{id}/store` | `owner` | → `202`; keeps every episode on the server. |
| `PUT`, `DELETE /episodes/{id}/stored` | `member` | Keep one episode on the server, or drop the server's copy. `Episode.stored` already tells. |
| `GET /me/podcasts/unplayed` | `member` | Paged `{podcast_id, unplayed}` (optimisation 11). |
| `GET /episodes/{id}/transcript` | `member` | `{segments: [{start_ms, end_ms, speaker?, text}]}`; `404` when none. |
| `GET /podcasts/directory/search?q` | `member`, outbound bucket | Paged directory results (`feed_url`, title, author, image, followed?). |
| `GET /podcasts/directory/categories`, `GET /podcasts/directory?category=` | `member` | Browse. |
| `GET /podcasts/directory/preview?feed_url=` | `member` | A show not followed, with its latest episodes, nothing stored. |
| `GET /podcasts/directory/similar?podcast_id=|feed_url=` | `member` | Replaces both v1 "similar" routes. |

Discovery: `modules.podcast_directory` (rule 8) — off answers `501 feature_unavailable`.
Migration: an index for unplayed counts if `EXPLAIN` asks.
Tests: the suite already serves an RSS feed from the test machine (`filesync`, `gpodder`): follow
it, refresh, the episodes arrive in `/sync/changes`; follow twice is one podcast; unfollow is a
removal in the change feed; store and drop an episode; unplayed counts drop after progress;
transcript parsed from a served VTT/SRT; directory routes skipped when the module is off (CI has
no outbound network), checked through a stub in a unit test.

### Phase 3 — sync essentials and continue listening (M) — done 2026-11-17

Done, v2 revision 5: everything in the table below; the 3 rows covered (`GET /library/continue`,
`GET /sync/tree`, `GET /sync/tree/ids`), and the gaps plan's §1 lines for file facts, triggers,
`kinds=`, the ids-only snapshot, `member` and continue listening ticked. Migration 0126. Unit
tests for the kinds, include and progress-kind parsing; the conformance suite `v2_sync` covers
behaviour (a copy built from a snapshot and the change feed equals a fresh snapshot, for the
owner and for another member). What the plan said, with these differences:

- The ids-only snapshot is `fields=ids` with `{"type":"id","kind","id"}` lines, no `version`:
  the reconcile needs ids, the change feed brings updates. A state line resumes its kind whole.
- `MemberRecord` is `{user_id, display_name, label, image, left_at}` (`label` as on `Member`).
  Someone who moves to another household keeps the name the household knew (new table
  `family_former_members`); a deleted account stays a record with `left_at` and **no name**
  (decided the safer way: an erased account leaves no name behind). On a one-household server
  nobody leaves but by deleting the account, so the name-kept case is tested only where there
  are several households. A former member has no `image`.
- `include=` on `/me/progress` takes `items`, `podcasts`, `albums`, `contributors`; embedded
  books are the list shape (no files). `Included` gains `tracks`, `books`, `episodes`,
  `podcasts`.
- `kinds=` on `/sync/changes` also takes one kind of state (`progress.episode`), and a `reset`
  always comes through. `member` is first in the snapshot's kinds and in its default.
- More triggers than the plan named, each because the reproduce check needed it: a grant is a
  change of the item (and what it refers to) for that member only; a member's media policy is a
  `reset` for that member only (the whole-kind gap the reconcile used to cover); a computed
  checksum re-sends the items that play the file (three partial indexes on `audio_object_id`);
  an item that appears, is shared, unshared, trashed, quarantined or missing also logs its
  album, release group, recording, genres and contributors, and a show its episodes (before, a
  member could get a shared track without its album, a shared show without its episodes); a
  credit logs its contributor, a track genre its genre.
- The change log's one-row-per-transaction index now includes the household, so a move between
  households logs the member for both.
- Joining a household is a `reset` for the newcomer, and for the household only when the
  newcomer brings a library (0109 reset everyone on every new account).
- Measured on 610,000 tracks (`CAPACITY.md`): two indexes for continue listening (progress by
  listener and time, music and episodes: 33 ms → 0.7 ms), three for the checksum trigger; the
  ids-only snapshot is about 37 MB; the new triggers add about 10 % to inserting tracks.
- Fixed on the way: deleting an account that follows a podcast failed (`500`, the gPodder log
  wrote for the deleted account). `POST /books` and uploaded episodes queue their checksums at
  once, as tracks do.
- `filesync` runs on a one-household server (its outsider checks skip there); with
  `--compose-dir` it passes. `trash` still fails there and waits for phase 4, so CI keeps
  running without `--compose-dir` until then (done in phase 4).


v1 ops (partly/missing): `GET /library/continue`, `GET /sync/tree`, `GET /sync/tree/ids`. Plus
the gaps plan's blocking lines: `kinds=`, the `member` kind, `include=` on progress, file fields,
change triggers.

| v2 | Permission | Change |
|---|---|---|
| `GET /me/progress` | `member` | `kind` takes a comma list; `include=items` embeds the records (optimisation 1); `unfinished=true` for continue listening, ordered by last played. |
| `GET /sync/changes` | `member` | `kinds=` (optimisation 2). |
| `GET /sync/snapshot` | `member` | `fields=ids` (optimisation 3); adds kind `member`. |
| Records `Track`, `Book`, `Episode` | — | `path` (the own.audio folder path), `sha256`; `BookFile.relative_path` and `sha256`; also in the create answers. Optional fields: additive. |
| Kind `member` | — | `{user_id, display_name, image, left_at?}` for owner names, people who left included. |

Migration: change-log triggers for `sync_paths` (a rename is one change row of the item) and
`content_grants`, and for `family_members` (the `member` kind).
Tests: snapshot + changes reproduce the library with `kinds=` and without; ids-only snapshot
lists the same ids as the full one; a rename through v1's file sync arrives as one change; a
member who leaves still resolves as owner; `include=items` embeds each record once and only
visible ones; paging without loss on the multi-kind progress list.

### Phase 4 — deletion and trash (M) — done 2026-11-17

Done, v2 revision 6: every operation in the table below; the 7 rows covered, and the gaps plan's
lines for deleting a track and a book and for the trash ticked. No migration. Handler tests (no
token is `401` on every route) and unit tests for the kinds and the request's checks; the
conformance suite `v2_trash` covers behaviour. What the plan said, with these differences:

- The trash's kinds are v2's words: `track`, `book`, `playlist`, `episode` (the server's stored
  copy of an episode, as `DELETE /episodes/{id}/stored`; the episode stays listed) and `file` (a
  companion file of the own.audio folder, the kind phase 11 gives them in sync). An unknown kind
  is `400 invalid_kind`.
- `POST /trash` answers `200 {batch, items}` with one `ItemResult` per item in order:
  `client_id` is the item's id, `status` `ok`, `duplicate` (named twice), `not_found`,
  `forbidden` or `invalid_kind` — the single delete's rule per item. No items is `400
  invalid_body`, more than `batch_max` (500) `400 too_many`. `Idempotency-Key` is the v2
  middleware's, as on every write: a retry answers the first answer, the same batch.
- `DELETE /tracks/{id}`, `DELETE /books/{id}` are each a batch of their own; v1's
  `X-Trash-Batch` header is not read on v2 (one `POST /trash` is one batch). A batch deleted
  through v1 restores through v2.
- `GET /trash` names who and when as `owner_id`, `deleted_by` (member ids), `deleted_at`,
  `purge_at`, and adds `size_bytes` and `restore_charge_micro` (the edition's quote, `0` here;
  the hosted edition charges a restore). It also takes `scope=family`, a household admin's view
  of everything shared, as v1; `invalid_scope` otherwise. Cursor-paged by `(deleted_at, kind,
  id)`, newest first.
- `POST /trash/batches/{batch}/restore` answers `{batch, items: [ItemResult], charged_micro}`
  rather than a bare array, so it can grow; `404` when nothing in the batch is the caller's to
  restore. `POST /trash/{kind}/{id}/restore` answers `204` as planned (v1 answers the charge;
  the list quotes it beforehand).
- `DELETE /trash` takes `scope` too and answers `{purged}`; it walks the trash 200 rows at a
  time instead of reading it whole as v1 does.
- The change feed needed nothing new: the triggers on `music_tracks_all` and
  `audiobook_books_all` (0109, 0126) log a deletion, a restore and a purge, with what the item
  makes visible — `v2_trash` checks that a song that was its album's only one takes the album
  with it and brings it back. Playlists and companion files reach the feed in phases 6 and 11,
  as planned.
- An uploaded podcast still answers `409 uploaded_podcast` to `DELETE /podcasts/{id}`: the trash
  holds no shows (v1 deletes an uploaded show outright, its objects left for the storage sweep,
  which is not something to copy). Its episodes' stored copies go to the trash one by one or as
  a batch. A show kind in the trash moves to phase 11.
- `trash` (v1) runs on a one-household server: the owner is promoted to the household's admin
  and no invitation is sent. A full run with `--compose-dir` passes on the compose stack, so CI's
  conformance job now runs with `--compose-dir .`: `trash` and `filesync` run in CI, and so do
  the mailed links' checks of phase 1 (`…/password-reset/confirm`,
  `POST /auth/email-verification`) against the mail catcher.
- Measured on 605,000 tracks with 5,000 of them in the trash (`CAPACITY.md`): no index added —
  each table's partial `trashed_at` index (0077) keeps every trash query to the trashed rows
  (first page 56 ms, a middle page 9.5 ms, a batch 0.7 ms). The cost grows with the trash, not
  the catalog; the index that would make a page stop after 100 rows is written down there for
  when a trash grows to tens of thousands.
- `filesync` and `v2_walk` upload a real tone for their songs now: with `trash` and `v2_trash`
  running before them, the fingerprint job reached their random bytes part-way through and
  quarantined them (the same reason `v2_sync` already used one). `Ctx.tone()` in
  `conformance/core.py`; suites that keep a song around for a while should use it.

v1 ops: `DELETE /audiobooks/{id}`, `DELETE /music/tracks/{id}`, `GET /trash`,
`POST /trash/batches/{batch}/restore`, `POST /trash/empty`, `DELETE /trash/{kind}/{id}`,
`POST /trash/{kind}/{id}/restore`.

| v2 | Permission | Request → answer |
|---|---|---|
| `DELETE /tracks/{id}`, `DELETE /books/{id}` | `owner` or `family_admin` | → `204`; into the 30-day trash, one batch. |
| `POST /trash` | `member` | `{items: [{kind, id}]}` (≤ `batch_max`) → `{batch, items: [ItemResult]}`; `Idempotency-Key` (optimisation 7). |
| `GET /trash` | `member` | Paged, newest first; `{kind, id, title, batch, deleted_at, purge_at}`. |
| `POST /trash/batches/{batch}/restore` | `member` | → `[ItemResult]`. |
| `POST /trash/{kind}/{id}/restore` | `member` | → `204`. |
| `DELETE /trash/{kind}/{id}` | `owner` or `family_admin` | Purge one. |
| `DELETE /trash` | `member` | Empty the caller's trash. |

Migration: the trash list index if needed.
Tests: what the v1 `trash` suite proves, on v2: delete → gone from lists, a removal in
`/sync/changes`, back on restore; batch restore brings the whole batch; purge removes the objects
(SQL-backed); another family's id is `404`; a member cannot purge someone else's item.

### Phase 5 — books, series, collections, tags, authors, images (L) — done 2026-11-17

Done, v2 revision 7: every operation in the table below; the 34 rows covered, and the gaps
plan's §5 lines ticked. Migration 0127. Handler tests (no token is `401` on every route, the
outbound bucket on the identity routes, an identity search against a stub Open Library) and unit
tests for tags, places in a series, the search's checks and the source's failures; the
conformance suite `v2_books` covers behaviour, and `v2_walk` walks series and collections. What
the plan said, with these differences:

- Who may is v1's per function: the owner edits a book's details, adds files, sets its cover
  and tags; renaming, removing and reordering files and identifying are also a household
  admin's on a shared book (`x-permission: owner` on all of them). Identity search is the
  owner's or that admin's too (v1: anyone who sees the book; a search nobody may keep is no use).
- `PATCH /books/{id}` takes title, description, publisher, `published_year` and ISBN. Subtitle
  and language are not there: books have no such columns (a search's `subtitle` is shown, then
  dropped, as on v1). Adding them is a migration and an optional field later.
- `POST /books/{id}/files` takes `{files: [{upload_id, relative_path, title?}]}`, the shape of
  `POST /books`'s files (rule 1), not bare `upload_ids`: the path decides the play order, as
  v1's. Answers the book (`200`).
- `DELETE /books/{id}/files/{file_id}` refuses the last file (`409 last_file`; decided the safer
  way: v1 leaves a book with nothing to play, v2 points at `DELETE /books/{id}`, which the trash
  undoes). `PUT /books/{id}/files/order` answers `204` and `400 invalid_file` as the playlist
  order does; v1 left files it was not given unplaced.
- The identity routes answer `{items: [BookMatch]}` and take `{volume_id, fields}` (v1's
  fields); source failures are `502 source_unavailable`, `503 source_refused` (spent Google
  quota or refused key) and `400 invalid_volume_id`.
- `Book.tags` (names) is in the full record only, like `files` and `series`; `GET /books?tag=`
  matches a name in any case. Tags stay server-wide names (v1's table); a reader learns of one
  only through a book they can see.
- Series and collections have `visibility` and an owner, read through the audiobook policy like
  books. `POST /series` is `can_upload`, `POST /collections` any member's (a playlist's rule);
  changing either is its owner's, deleting also a household admin's for a shared one. A
  replace keeps the books the owner cannot see. `Series.books` holds the visible books with
  their places (`SeriesBook {book_id, position}`); collections count their books and page them
  (`CollectionEntry {position, added_at, book}`). v1 still lists only the caller's own.
- `Book.series` names only the series the reader can see (it named another member's private
  series too).
- Contributors: `POST /contributors` answers the household's contributor of that name with
  `200` when there is one. An uncredited contributor is found by `PATCH`, the image routes and
  `DELETE` (`GET` still shows only those credited on something visible). `DELETE` refuses a
  credited one (`409 in_use`) where v1 took the credits away. `Contributor.image_credit` is
  `null` for a picture set by hand.
- Migration 0127: change-log triggers on series (a rename, a share or a new owner also sends its
  books, whose records name it), places in a series (the series and the book), collections and
  their books (the collection), book tags and a renamed tag (the book); `change_log_item` adds
  the series and collections a book is in (a book shared, unshared or trashed changes what they
  show); indexes on the two junctions by book. Kinds `series` and `collection` are in the
  default snapshot and change feed: the iPhone apps ask for explicit kinds and keep only theirs,
  the TV skips kinds it does not know.
- Not measured on the 600k fixture: nothing here reads a catalog-sized table but by index. Books
  (1,000 at most), their tags, series and collections are small; the contributor lookups are by
  id with the credits' contributor index. Timings on 1,000 books are in `CAPACITY.md`.
- Phase 6 can point `POST /music/artists/upload-image`, `DELETE /music/artists/image` and
  `GET /music/artists/image-info` at the contributor routes and `image_credit`: an artist is a
  contributor.


v1 ops: every Audiobooks row still missing or partly except `DELETE /audiobooks/{id}`: file add,
reorder, rename, delete, upload-file, metadata search/apply, upload-cover, `PUT /audiobooks/{id}`
(title, description), author create/delete/bio, author image upload/forget/info, tags (3),
collections (8), series (7 incl. the two partly).

| v2 | Permission | Request → answer |
|---|---|---|
| `PATCH /books/{id}` | `owner` | Title, subtitle, description, language, year, publisher, ISBN → `Book`. |
| `POST /books/{id}/files` | `owner` | `{upload_ids: […]}` → `Book`; `Idempotency-Key`. |
| `PATCH /books/{id}/files/{file_id}` | `owner` | `{title}`. |
| `DELETE /books/{id}/files/{file_id}` | `owner` | `204`. |
| `PUT /books/{id}/files/order` | `owner` | `{file_ids}` (as `PUT /playlists/{id}/entries/order`). |
| `POST /books/{id}/identity/search`, `PUT /books/{id}/identity` | `owner`, outbound bucket | Named as the tracks' identity routes; the family's book source. |
| `PUT`, `DELETE /books/{id}/image` | `owner` | The image pattern. |
| `POST /contributors` | `can_upload` | `{name, sort_name?, bio?}`. |
| `PATCH /contributors/{id}` | as today | Adds `bio`; `Contributor` gains `bio` and `image_credit`. |
| `DELETE /contributors/{id}` | `family_admin` | `409 in_use` while credited. |
| `PUT`, `DELETE /contributors/{id}/image` | `can_upload` | Upload; or forget and look it up again. Serves artists too (phase 6). |
| `GET /book-tags` | `member` | Paged, with counts. |
| `PUT /books/{id}/tags` | `owner` | `{tags: […]}`; `Book.tags`; `GET /books?tag=`. |
| `GET`, `POST /series`; `GET`, `PATCH`, `DELETE /series/{id}` | `member` / `can_upload` / owner | `Series {id, name, description, books: [{book_id, position}]}`. |
| `PUT /series/{id}/books` | owner | The ordered list in one call. |
| `GET`, `POST /collections`; `GET`, `PATCH`, `DELETE /collections/{id}` | `member` / owner | The playlist shape for books, `visibility` as everywhere. |
| `GET`, `POST /collections/{id}/books`; `DELETE /collections/{id}/books/{book_id}` | owner | Paged list; add many; remove one. |

Migration: change-log triggers for series, series membership, collections and their entries, book
tags; snapshot and changes gain kinds `series` and `collection`.
Tests: per route at each permission level; reorder survives a reload and `/sync/changes`; a
series change arrives as one change row; collections page without loss; tags filter; identity
search against a stub source in a unit test, `PUT identity` through the suite's book; uploads
through `POST /uploads` only (no multipart, by design).

### Phase 6 — music remainder and playlists in sync (M) — done 2026-11-17

Done, v2 revision 8: every operation in the table below but the audience and copies; the 17 rows
closed (14 covered, 3 not carried over: `share`, `audience`, `tracks/discs`), and the gaps plan's
§4 music lines ticked, the artist pictures' among them. Migration 0128. Handler tests (no token
is `401` on every new route) and unit tests for lyrics as text or timed lines, the intent's
checks, the new kinds and the demo's allow-list; the conformance suite `v2_music` covers
behaviour, and `v2_walk` walks feedback and the new image and lyrics routes. What the plan said,
with these differences:

- `PUT /tracks/{id}/lyrics` takes `{text}` (plain or LRC) or timed `{lines: [{start_ms, text}]}`
  — the shape `GET` answers (rule 1), not `{plain, synced}` — and answers `Lyrics`. `null` or
  empty takes the owner's lyrics away and the file's own are read again (v1 stored an empty
  text, which the read takes as "none", for good).
- `GET /tracks/{id}/file-tags` is the owner's (`x-permission: owner`), as the plan said; v1 lets
  anyone who sees the track ask, and reading a whole file on request is not every member's. It
  adds `disc_number` and `disc_total`.
- `POST /tracks/{id}/rescan` reads the file during the request and answers the track (`200`),
  as v1, rather than `202`: it is one file, the owner waits for its answer, and a queued rescan
  would need a job and polling for no gain. `409 identified`, `422 no_tags` (v1: `400`).
- `GET /tracks/duplicates` answers `{key, kind: identical, track_ids}` groups, cursor-paged by
  the key (the file's SHA-256) in SQL, `include=tracks` to embed them; the caller's own tracks
  only, as v1. "The stored grouping keys" of the plan are the album grouping keys (0089); v1
  groups by the checksum and so does v2. Measured on 601,000 tracks (`CAPACITY.md`): 60 ms a page, 2.3 s for
  the empty answer of a library without duplicates; no migration.
- Feedback is `GET /me/feedback` (cursor-paged, `value=`), `PUT`/`DELETE
  /me/feedback/{kind}/{id}` with kind `track` only (`400 unsupported_kind` otherwise), values
  `dislike` and `ban` (v1 stores `banned`). Kind `feedback.track` in the snapshot (`feedback`)
  and the change feed; not in `POST /sync/push` yet (an offline dislike waits for the network).
  The demo account may not give feedback (favourites are refused there too); it may ask
  `POST /smart-playlists/intent`, a read sent as `POST`.
- `POST /smart-playlists/intent` reads the household's genre names from the genre table instead
  of v1's grouping of every visible track; `422 not_understood`, `400 invalid_body`.
- Images: `PUT`/`DELETE /tracks/{id}/image` mark the edit history (undo puts the old cover back,
  so its object is kept, as v1's upload); a track cover also needs the right to add to the
  library, as a book's. `PUT`/`DELETE /playlists/{id}/image` drop the old picture when nothing
  else uses it; without one, a playlist shows its first track's cover.
- `PATCH /playlists/{id}` takes `kept` both ways (keep, or let go again); `409 not_generated` on
  a playlist made by hand (v1: `404`). `Playlist` gains `generated` and `kept`. The record keeps
  `track_count` (of the entries the reader can see) rather than adding `entry_count`: one shape
  for one thing.
- Migration 0128: change-log triggers on playlists (the trash included: a playlist deleted,
  restored or purged reaches `/sync/changes`, which phase 4 left for this phase), their entries,
  smart playlists and track feedback; a track that is shared, unshared, trashed, quarantined or
  missing, or changes its length or cover, logs the playlists that hold it (their records count
  and picture only what the reader can see); a grant on a track or a playlist logs the playlists
  it shows (0127's `change_log_item` again). New index `music_playlist_tracks (track_id)`: the
  trigger's lookup, and the cascade when a track is purged scanned every entry before.
- Kinds `playlist`, `smart_playlist` and `feedback` are in the default snapshot and change feed:
  music-ios and book-ios ask for explicit kinds and keep only theirs (`OwnAudioSource.keeps`),
  podcast-ios does not sync, and the TV skips kinds it does not know (`LibraryCache`); all of
  them decode a change's record by kind, so an unknown kind costs nothing (checked 2026-11-17).
- Artist pictures: `PUT`/`DELETE /contributors/{id}/image` and `Contributor.image_credit` (phase
  5) serve artists. `DELETE` now looks again in the background, as its description said:
  an artist's photo when the contributor is credited on music, an author's when only on books
  (v1's two lookups, moved into functions both versions call). Before, v2 only cleared it and
  nothing looked again until a v1 client asked.
- Sharing a playlist with chosen members, live or as copies, and its audience stay on v1
  (question 2). The disc-number back-fill is not a route; it becomes a job in phase 12.


v1 ops: `POST /music/artists/upload-image`, `DELETE /music/artists/image`,
`GET /music/artists/image-info`, `GET /music/duplicates`, `GET /music/feedback`,
`POST /music/intent`, `DELETE /music/tracks/{id}` (in phase 4), `POST …/upload-cover`,
`PUT`/`DELETE …/feedback`, `PUT …/lyrics`, `GET …/metadata/file-tags`, `POST …/metadata/rescan`,
`POST /music/playlists/{id}/upload-cover`, `GET …/audience`, `PUT …/keep`; partly: `…/share`;
`POST /music/tracks/discs` proposed as not carried over (§1.2).

| v2 | Permission | Request → answer |
|---|---|---|
| `PUT /tracks/{id}/lyrics` | `owner` | `{plain?, synced?: [{start_ms, text}]}`. |
| `GET /tracks/{id}/file-tags` | `owner` | The file's own tags, read now. |
| `POST /tracks/{id}/rescan` | `owner` | → `202`. |
| `GET /tracks/duplicates` | `member` | Paged groups `{key, track_ids}`, in SQL on the stored grouping keys. |
| `GET /me/feedback`, `PUT`/`DELETE /me/feedback/track/{id}` | `member` | `{value: dislike|ban}`; user state like favourites, kind `feedback.track` in sync. |
| `POST /smart-playlists/intent` | `member` | `{text}` → a rule, or `422 not_understood`. |
| `PUT`, `DELETE /tracks/{id}/image`, `/playlists/{id}/image` | `owner` | The image pattern. |
| `PATCH /playlists/{id}` | `owner` | Adds `kept` (keep a generated playlist). |
| `GET`, `PUT /playlists/{id}/audience` | `owner` | Chosen members, live; `POST /playlists/{id}/copies` `{user_ids}` for copies. Only if question 2 says so. |

Migration: change-log triggers on playlists, entries and smart playlists; kinds `playlist` and
`smart_playlist` in snapshot and changes (a playlist record carries `entry_count` and
`updated_at`; entries stay a paged read).
Tests: feedback round-trip and in sync; a playlist edit arrives in `/sync/changes`; a deleted
playlist is a tombstone; duplicates find the suite's two copies of one track; lyrics write then
read; rescan reads new tags (SQL-backed fixture).

### Phase 7 — household remainder (L) — done 2026-11-17

Done, v2 revision 9: every operation in the table below; the 18 rows covered, and the gaps plan's
§2 join line and §7 household, leaving and settings lines ticked. Migration 0129. Handler tests
(no token is `401` on every new route; the claim checks the password before the database; the
preview shares the device-code bucket, the claim the sign-in bucket) and unit tests for leaving
and removing (the last admin, one household, unclaimed accounts) and the reports' words; the
conformance suite `v2_household` covers behaviour. v1's rules are shared, not copied: block,
leaving or removing (`families::leave_or_remove`, its decision a pure `removal_step`),
provisioning, regenerating, looking up a code, claiming, switching analysis, choosing a book
source and storing the household's photo are functions both versions call; v1's answers are
unchanged and its `join` and `families` suites pass. What the plan said, with these differences:

- `POST /family/members` answers `{member, invite}` — the `Invite` shape the household already
  has (rule 1), whose `code` and `expires_at` are the claim code and its expiry — rather than
  `{member, claim_code, claim_expires_at}`. It takes `login_email` (required, as v1: what the
  person signs in with), `display_name`, `label`, `age_bracket`, `can_upload`, `can_generate`; a
  child given uploads is `400`, a taken login `409 email_taken`. `Invite` gains `member_user_id`.
- `POST /auth/claim` takes `{code, password}` and answers `Me` (`201`). No `email?`: the login is
  the one the admin chose and the preview shows; letting the claimant change it would need the new
  address proven first (decided the safer way; additive later if a client asks).
- `POST /family/join/preview`: unknown `404`; used `410 invite_used`, expired
  `410 invite_expired` (v1 answers `200` with a `status`); `kind` is `invite` or `claim`;
  `family: {name, image}`. The claim answers the same codes (`not_a_claim_code` for a join code).
- Blocking also refuses a server administrator (`403 cannot_block_instance_admin`), new on v2 and
  decided the safer way: on a one-household server a household admin could otherwise lock the
  operator out. Blocking oneself is `400 cannot_block_self`, as v1. Unblocked accounts sign in
  again; blocked ones are signed out at once (`401` on `/sync/changes`, refresh refused).
- `DELETE /family/members/{id}` is `x-permission: member` (self, or an admin for anyone), its
  `x-scope` kept `family:admin` so personal access tokens keep the scope they had. On a server
  with one household nobody leaves it (`409 one_household`, as v1), so leaving is checked only
  where there are several; the last admin is checked on one household by briefly making the
  suite's admin a member again.
- Pictures take `{upload_id}` or the image itself (5 MB), as `/me/image`, and answer `Family` /
  `Member`; a member sets their own, an admin anyone's.
- Settings: `{audio_analysis, book_source}` are v1's two records; `PATCH` takes
  `{audio_analysis?: {enabled}, book_source?: {source?, google_books_api_key?}}`, the book source
  first so a refusal changes nothing; a server that offers no choice is `501 feature_unavailable`
  (`book_source_choice`; v1 `403`), a bad key `400 invalid_key`, Google without one
  `400 key_required`.
- Reports: `POST` takes `{kind, item_id, reason, note?}` (`item_id`, as bookmarks; kinds
  `track`, `book`, `podcast`) and only what the reporter can see (`404`; v1 took any id); `GET`
  has `state=open|resolved|all`, open by default; `PATCH {resolved, note?}` also opens a report
  again. Migration 0129 adds `resolution_note` and an index of every report by household and
  time (open ones keep 0057's). The demo account may not report (its allow-list is unchanged).
- The change feed: a new kind `family`, the record `GET /family` answers (with the reader's
  `me`), logged when the household's name or photo changes and, for that member only, when their
  role or rights do; in the default snapshot right after `member` (clients skip kinds they do not
  know, phase 6). `MemberRecord` gains `blocked`, logged when an account is blocked or let in.
- Fixed on the way: `PATCH` and `DELETE /family/members/{id}` answered `401` (v1's "not found")
  for someone not in the household; `404` now, as documented.
- Step 0 (before the phase): bulk track inserts are linear, no trigger at fault; the phase-6
  minutes were not reproduced (`CAPACITY.md`).


v1 ops: `POST`/`DELETE /family/avatar`, content reports (3), `POST /family/invites/{id}/regenerate`,
`POST /family/members/provision`, member avatar (2), block/unblock, family settings (4),
`GET /join/{code}`, `POST /join/{code}/claim`; partly: a member leaving themselves.

| v2 | Permission | Request → answer |
|---|---|---|
| `PUT`, `DELETE /family/image` | `family_admin` | The image pattern. |
| `PUT`, `DELETE /family/members/{id}/image` | self or `family_admin` | |
| `PATCH /family/members/{id}` | `family_admin` | Adds `blocked`. |
| `DELETE /family/members/{id}` | self or `family_admin` | A member may now leave; the last admin may not (`409 last_admin`). |
| `POST /family/members` | `family_admin` | Provision `{display_name, age_bracket, …}` → `{member, claim_code, claim_expires_at}`; `Idempotency-Key`. |
| `POST /family/invites/{id}/regenerate` | `family_admin` | → `Invite` with a new code. |
| `POST /family/join/preview` | `public` (device bucket) | `{code}` → household name, picture, inviter, kind (invite or claim). Code in the body (rule 6). |
| `POST /auth/claim` | `public` (login bucket) | `{code, email?, password}` → `Me`; then `/auth/token`. |
| `GET`, `PATCH /family/settings` | `member` / `family_admin` | `{audio_analysis, book_source}`. |
| `GET /family/reports` | `family_admin` | Paged. |
| `POST /family/reports` | `member` | `{kind, id, reason}`. |
| `PATCH /family/reports/{id}` | `family_admin` | `{resolved: true, note?}`. |

Tests: what the v1 `join` and `families` suites prove, on v2: preview without signing in, claim
then sign in, a code used twice refused, an expired code `410`; a blocked member is refused at
sign-in and in `/sync/changes`; leave household; the last admin cannot leave; settings read by a
member, changed by an admin only; reports visible to admins only.

### Phase 8 — listening settings, queue, notifications, push (M) — done 2026-11-17

Done, v2 revision 10: every operation in the table below; the 11 rows covered, and the gaps
plan's §6 lines and §7's notifications and push devices ticked. Migration 0130. Handler tests (no
token is `401` on every new route; discovery's `push`) and unit tests for the queue's words and
checks, `If-Match`, the settings' ranges, a notification's item and push tokens; the conformance
suite `v2_listening` covers behaviour, and `v2_walk` walks push devices (another member's id is
`404`). The rows are v1's (`play_queues`, `user_settings`, `eq_settings`,
`pending_notifications`, `device_push_tokens`), so v1, Subsonic and v2 read and write one queue,
one set of settings, one inbox. What the plan said, with these differences:

- The queue's items are `{kind: track|book|episode, id, file_id?}` (a book's file to start in);
  stored in v1's shape, an episode as its show and itself. `device_kind` and `device_name` (the
  session's, as `DevicePosition`) instead of a `device` object; `updated_at`. The `revision` is a
  column counted by a trigger over every write — v1's `PUT /playback/queue` and Subsonic's
  `savePlayQueue` included — and is also the `ETag`. `PUT` with `If-Match` (`"3"`, `3` or
  `W/"3"`; `"0"` for "there was none") writes only at that revision, in one statement, else
  `412 revision_mismatch` with `revision` in the problem; `If-Match: *` or none is v1's last
  write wins. Items must be playable by the caller (`400 invalid_item`; v1 stored any id);
  more than 1000 is `400 too_many`; the index is clamped into the list, as v1.
- `GET`/`PATCH /me/settings` is `{playback: {speed, skip_intro_ms, skip_outro_ms}, audiobook:
  {speed, skip_back_ms, skip_forward_ms}, eq: [EqCurve], updated_at}`: v1's three reads in one
  record (optimisation 8). Milliseconds are stored to the second (v1's columns). `playback`
  is writable too (v1 only read it). Out-of-range values are `400 invalid_body` with the field,
  checked before anything is stored, where v1 clamps. `eq` lists every device kind's saved curve
  (`bands_db`: six numbers); a patch names a `device_kind` or changes the calling device's.
- Notifications are cursor-paged newest first and list read ones too (`read_at`);
  `unread_only=true` is v1's list. `item` names what one is about in v2's words (`track`,
  `book`, `episode`, `podcast`, `playlist`, `file`) beside v1's `data`. `ack` takes `{ids}` (up to
  `batch_max`) or `{up_to}` (that one and every older one; `404` if not the caller's) and answers
  `{acknowledged}`. Notifications never expire (v1 never deleted them either); the index
  `(user_id, created_at DESC, id DESC)` keeps a page an index read.
- Push devices answer `{id, platform, device_name, token_suffix, created_at, last_seen_at}`;
  `POST` is `201`, or `200` for a token already the caller's; a token another account
  registered moves to the caller, as v1. `DELETE` is by the device's id (v1: by the token). They
  are not in the change feed: a device knows its own token, and the list is a settings screen
  read on demand (decided the safer way: logging a moved token would tell its old owner nothing
  useful). `modules.push` (`false`) is set in discovery only, not in v1's `features`.
- Change feed: kinds `queue` and `settings`, named by the listener's id and logged for the
  listener only, in the default snapshot (one record each) and the change feed; `notification`
  in the change feed only (a snapshot would be the whole inbox: read `GET /me/notifications`).
  Clients skip kinds they do not know (checked again: OwnAudioKit asks `/sync/changes` without
  `kinds` and keeps records by kind). `/events` needed nothing: it already wakes on the
  listener's own log rows.
- `POST /sync/push` takes `feedback` (with `value`) and `unfeedback` (phase 6 left them out). The
  queue and the settings are not in it: the queue is replaced whole and has its own
  `If-Match`, so an offline queue is a `PUT` when the device is back; settings are a patch of
  one resource, sent when the device is back. The demo account may write its own queue,
  settings, reads and push devices (v1 allowed the last two too); its `feedback` in a push is
  refused, as its favourites are.
- `v2_sync`'s reproduce check leaves out the kinds only the change feed carries (`recording`,
  `notification`): a copy holds them, a fresh snapshot cannot.
- Scopes: `PUT /me/queue` and `POST /me/notifications/ack` take `playback`, as progress;
  `PATCH /me/settings` takes `playback`; push devices take `user:write`.


v1 ops: `GET`/`PUT /playback/queue`, `GET /playback/settings`, `PUT …/audiobook-defaults`,
`GET`/`PUT …/eq`, `GET /devices/notifications`, `POST …/ack`, `POST`/`DELETE /devices/push-token`,
`GET /devices/push-tokens`.

| v2 | Permission | Request → answer |
|---|---|---|
| `GET`, `PUT /me/queue` | `member` | `{items: [{kind, id}], index, position_ms, device, revision}`; `PUT` with `If-Match: <revision>` refuses a stale write with `412`. User kind `queue` in the change log (optimisation 9). |
| `GET`, `PATCH /me/settings` | `user` | `{audiobook: {speed, skip_back_ms, skip_forward_ms, …}, eq: {…}}`; milliseconds, not v1's seconds. |
| `GET /me/notifications` | `user` | Paged, newest first, `unread_only=`. |
| `POST /me/notifications/ack` | `user` | `{ids}` or `{up_to}`. User kind `notification` in the change log. |
| `GET`, `POST /me/push-tokens`; `DELETE /me/push-tokens/{id}` | `user` | Registration only; `modules.push` says whether the server sends any (today none: clients read the inbox). |

Demo: queue, settings and acks join the demo's allowed listening writes.
Tests: two devices: one writes the queue, the other gets it from `/sync/changes` and a `library`
event; a stale `If-Match` is `412`; settings round-trip in ms; notifications page and ack
`up_to`; push token visible only to its owner.

### Phase 9 — statistics (S) — done 2026-11-17

Done, v2 revision 11: every operation in the table below; the 5 rows covered, and the gaps
plan's §7 stats line ticked. Migration 0131 (no index). Handler tests (no token is `401`) and
unit tests for the range's checks, the buckets and a play's words; the conformance suite
`v2_stats` covers what v1's `stats` suite proves. What the plan said, with these differences:

- Read from the plays (`listening_sessions`, what `POST /me/plays` and v1 record), as v1 does,
  not from the rollup: `listening_daily` counts UTC days and lags up to an hour, so it could
  neither put a play on the listener's own day nor count one just sent. Measured on 2,100,000
  plays (`CAPACITY.md`): a year grouped by month 27 ms for a listener with a million plays.
- `GET /me/stats` takes `from`, `to` (RFC 3339; 30 days up to now by default, at most ten years),
  `group` (`day`, `week` from Monday, `month`), `kind` (`track`, `book`, `episode`) and v1's
  `tz_offset_minutes`; answers `listened_ms`, `plays`, `by_kind`, `buckets` (each with its
  `by_kind`; empty ones left out), `top` (the ten most listened in the range, not per bucket —
  ten items per day of a year would be thousands of rows nobody shows), `streak_days`,
  `finished_books`. Problems `invalid_range`, `invalid_group`, `unsupported_kind`,
  `invalid_tz`. Kinds are what was played: an episode by itself, where v1's tops name the show.
- `GET /me/plays` answers `PlayRecord` — the shape `POST /me/plays` takes, with `id` the play's
  own and `item_id` the thing played, `source` (`reported` or `derived`) — cursor-paged newest
  first, without titles (v1 looked one up per row). The page query names its bound on
  `started_at` so a deep page is a seek (69 ms → 0.07 ms five years back; no new index).
- **One name for one switch** (decided the safer way, rule 1): v1 has one stored value,
  `family_members.stats_visibility` (`private`/`family_admin`), set by the member or, for an
  account they manage, by an admin. The plan named it twice (`stats_visible_to_family` in
  `/me/settings`, `stats_visible_to_admin` on the member); v2 uses `stats_visible_to_admin` in
  both places, because that is what it means — the admins see the figures, not the household.
- **`GET /family/stats` stays `family_admin`** (decided the safer way; the plan said `member`):
  everyone who shares today agreed to show their figures to the household's admins only (v1's
  `family_admin`), so opening the list to every member would widen that consent without asking.
  The admin and those who share are listed (`{user_id, listened_ms, plays, by_kind}`; names
  from `GET /family/members`); a member who does not share is absent, where v1 listed them with
  `hidden`. Same `from`, `to`, `kind`.
- `stats_visible_to_admin` in `PATCH /family/members/{id}` keeps v1's rule — an account with a
  restriction (a `deny_all` policy) or oneself, else `403 member_not_restricted`, checked before
  anything changes. The public demo account may not change it (`403`), though it may patch its
  other settings.
- The change feed: the visibility is part of the `settings` record, logged by a trigger on
  `family_members` (migration 0131) so an admin's change reaches the managed account's devices.
  Plays and statistics are not cached records: no new kind.


v1 ops: `GET /stats/me`, `GET /stats/me/history`, `PUT /stats/me/visibility`, `GET /stats/family`,
`PUT /stats/family/members/{user_id}/visibility`.

| v2 | Permission | Request → answer |
|---|---|---|
| `GET /me/stats?from&to&group=day|week|month&kind=` | `member` | Totals in ms, plays, top items per bucket, from `stats_rollup`. |
| `GET /me/plays` | `member` | Paged history (the plays `POST /me/plays` records). |
| `PATCH /me/settings` | `user` | Adds `stats_visible_to_family`. |
| `GET /family/stats` | `member` | Only members who share; admins as v1 allows. |
| `PATCH /family/members/{id}` | `family_admin` | Adds `stats_visible_to_admin`, as v1. |

Tests: the v1 `stats` suite's checks on v2 — day bucketing, a retried play counted once, privacy
(a member who does not share is absent); history pages without loss.

### Phase 10 — search across media, list filters (S) — done 2026-11-17

Done, v2 revision 12: both rows covered (`GET /library/search`, `GET /library/private`), and the
gaps plan's lines for search across media and the private filter ticked. No migration. Unit
tests for the search's types and the filters' words; the conformance suite `v2_search` covers
behaviour. What the plan said, with these differences:

- `types` also takes `contributors`: anyone credited on something visible, in any role
  (authors, narrators, hosts, artists), beside `artists` (credited as artists, as before). A
  show is found by its title or a credited host, a book by its title or a credited name, an
  episode by its title (those that start with the words first, then the newest).
- Without `types` the answer is the music, as before (additive: a shipped app asks for nothing
  new and gets nothing new); the new arrays are always present, empty unless asked for.
  `include=contributors` embeds the books' and shows' people too.
- `owner` takes only `me` (`400 invalid_owner` otherwise): nobody lists another member's items
  by owner, which would only repeat `visibility=family` with a filter on `owner_id`. Both
  filters narrow what the caller can see and never widen it; on `/albums` they keep the albums
  with such a track.
- Measured on 607,869 tracks (`CAPACITY.md`): no index. The album list needed its filtered
  form to join the albums of the kept tracks (183 ms → 11 ms for a few private albums).
- `organize`'s identify-undo check counted the reverted edits (1–2), which depended on whether
  the release's cover arrived before the undo; it now checks that each reverted edit is the
  song's, its album's or a credited contributor's and in the batch (phase 11: `items` counts
  items, not edits; a release may also change the album and the artists), and that the song is back as it was.

v1 ops (partly): `GET /library/search`, `GET /library/private`.

| v2 | Change |
|---|---|
| `GET /search` | `types` adds `books`, `podcasts`, `episodes`, `contributors` (optimisation 6). |
| `GET /tracks`, `/albums`, `/books`, `/podcasts` | `owner=me` and `visibility=private|family` filters. |

Tests: each type found and only visible ones; the filters page without loss; timings on the
scale catalog into `CAPACITY.md`.

### Phase 11 — file sync remainder (L) — done 2026-11-17

Done, v2 revision 13: everything in the table below; the 11 rows covered, and the gaps plan's
lines for companion files, deleting an uploaded show, holdings and shortcuts, organise and "use
as cover" ticked. Migration 0132. Handler tests (no token is `401` on every new route) and unit
tests for the shortcut's request, the holdings', trash's and file's words; the conformance suite
`v2_folder` covers behaviour. What the plan said, with these differences:

- **`POST /library/organise` is `member`, the caller's own items** (decided the safer way; the
  plan said `family_admin`): v1 lets anyone organise their own items and nobody anyone else's,
  and an admin-only route would take that from members without moving anything of theirs.
  `dry_run` is required — a request that forgets it is `422 invalid_body`, not a move. Answer `{dry_run,
  moves}`, each move `{kind, id, title, from, to, files}`.
- **`file` and `shortcut` are in the snapshot only when `kinds=` names them** (decided the safer
  way): the TV takes the default snapshot and would read every cover and booklet of a big
  library for nothing; the iPhone apps name their kinds. Both are in the change feed's default,
  as every kind is (a client skips kinds it does not know).
- `POST /files` takes `{upload_id, path}` (what a sync client knows) or `{upload_id, attach_to:
  {kind: album|book|track, id}, relative_path}`; `GET /files/{id}` was added beside it. The
  record `File`: `id, owner_id, visibility, path, name, content_type, size_bytes, sha256`,
  `used_as` in the create's answer only. Its upload is checked as other v2 uploads (`adopt`:
  declared size, room in the household); v1's route is unchanged.
- Holdings: `GET /me/holdings` pages the device's set (it can be a whole mirrored catalog) and
  `GET /me/holdings/{kind}/{id}` names the devices holding one item (v1 folded both into one
  route). The device is the session's `device_id`, as in progress, so a token without a refresh
  chain has one too.
- Shortcuts: `{member_id, media: music|books|podcasts, album_id|book_id|podcast_id, label}`. An
  album shortcut is stored as v1 stores it (release group, else album artist and name, taken
  from the member's tracks of that album), so v1's sync clients keep matching it, and answered
  with the `album_id` it matches — v1's shortcuts too. No migration for them beyond the change
  log trigger. Kind `shortcut` is the listener's own (`user_id`), like the queue.
- "Use as cover" is `{file_id}` on `PUT /tracks/{id}/image` and `PUT /books/{id}/image` (and on
  playlists and contributors, which share the body): the cover of that one item. v1's gesture
  set the folder's book and every track in it at once; on v2 a client names each item.
  `upload_id` became optional in that body (one of the two is required).
- The change feed: a companion file is kind `file` through a trigger on `companion_files_all`
  and its path (0126's path trigger now maps `companion_file`), so a delete, restore and purge
  through either version reach it — what phase 4 left open.
- An uploaded show in the trash: `podcast_feeds_all` behind the view, its guid's unique index
  only over live shows (a new show of the same guid may be made meanwhile; the restored one then
  comes back without its guid), the show's triggers moved and widened to `trashed_at` (v2 change
  feed with its episodes, v1's sync tree). Its episodes stay in the rows and are hidden with it;
  a stored copy trashed on its own is listed again once the show is back. Purging deletes the
  episodes (cascade) and every stored object of them, and forgets their holdings. **v1's trash
  routes neither list nor take kind `podcast`** (v1 behaviour unchanged; its clients know five
  kinds); emptying the trash through v1 and the 30-day purge include it.
- Not measured on the scale catalog: every new query reads the caller's own rows by an index
  (holdings by device, shortcuts by user, files by id and owner); the snapshot's walk of files
  goes through `audio2_can_access` per file, as v1's sync tree does — files are tens of
  thousands at most, and only clients that ask for them pay it.

v1 ops: `POST /sync/paths/organise`, `GET`/`PUT /sync/holdings`, `GET`/`POST /sync/shortcuts`,
`DELETE /sync/shortcuts/{id}`, `POST /sync/files`, `DELETE /sync/files/{id}`,
`GET /sync/files/{id}/stream`, `PUT /sync/files/{id}/visibility`,
`POST /sync/files/{id}/use-as-cover`.

| v2 | Permission | Request → answer |
|---|---|---|
| `POST /library/organise` | `family_admin` | `{dry_run}` → the moves; v1's one transaction kept (CAPACITY.md gap 4). |
| `GET`, `PUT /me/holdings` | `member` | What this device keeps; per session. |
| `GET`, `POST /sync/shortcuts`; `DELETE /sync/shortcuts/{id}` | `member` | Album shortcuts by `album_id`. |
| `POST /files` | `can_upload` | `{upload_id, attach_to: {kind, id}, relative_path}` → `File`; `Idempotency-Key`. Kind `file` in sync. |
| `DELETE /files/{id}` | `owner` | Into the trash. |
| `PUT /files/{id}/visibility` | `owner` | As other kinds. |
| `POST /playback-info` | `member` | Kind `file`, `direct` only: the download link (one signed-link mechanism, rule 1). |
| `PUT /{resource}/image` | owner | `{file_id}` — "use as cover". |
| `DELETE /podcasts/{id}` (an uploaded show) | `owner` | Into the trash with its episodes (phase 4 left it at `409 uploaded_podcast`): a `podcast` trash kind over a `podcast_feeds_all` table behind a view, as books and tracks (migration), listed and restored by `/trash`, purged with its episodes' objects. |

Tests: the v1 `filesync` suite's checks on v2 (loopback RSS feed, SQL-backed): companion files
create/visibility/delete in sync, download through `playback-info`, holdings round-trip,
shortcuts, organise dry run equals the moves made.

### Phase 12 — instance admin, setup, jobs, library folders (L) — done 2026-11-17

Done, v2 revision 14: everything in the table below; the 17 missing rows and `GET /users/{id}`
covered, so only the hosted `POST /admin/families/{id}/credit` is left (phase 13), and the gaps
plan's admin and library-folder lines ticked. The disc-number back-fill is the `disc_backfill`
job. Migration 0133 (indexes for the jobs list and for the tracks still to read). Handler tests
(no token is `401` on every new route; setup on the setup bucket) and unit tests (an account's
fields, an administrator's own rights, the lists' words); the conformance suite `v2_admin`
covers behaviour, and `run.py` makes the first administrator through `POST /setup` on a server
not set up yet — that is the positive case on CI's fresh stack, which no longer calls v1's
setup. What the plan said, with these differences:

- **An administrator keeps their own rights** (decided the safer way): `PATCH /admin/users/{id}`
  refuses to demote or block oneself and `DELETE` to delete oneself (`409 own_account`; one's own
  account is `DELETE /me`, which keeps the last admin), and `PATCH /admin/families/{id}
  {locked: true}` refuses the household the administrator is in (`409 own_family`). v1 allows
  both; on a one-household server locking the household locked everyone out, the administrator
  included, with nobody left to unlock it. So the conformance check of a locked household runs on
  a second household made through SQL (`--compose-dir`), always unlocked again by a cleanup.
- `POST /setup` checks whether the server is set up before it reads the body, so once set up it
  answers `409 setup_complete` to anything; two first requests at once make one administrator (an
  advisory lock). It answers `Me` as the plan said (v1 answered tokens); signing in is
  `POST /auth/token`.
- `GET /admin/families?state=` takes `active`, `locked` and `trashed`: v1's "families in the
  trash" (`GET /admin/families/trash`) was the trash report per household, so `trashed` lists the
  households with something in their trash, and every household carries `trash` (items, bytes,
  restores in 30 days, `flagged`). "Locked" is still what v1 made it: members, none of whom may
  sign in; unlocking lets every member in again, those blocked one by one included (v1's rule).
- The account and household lists are cursor-paged too (the plan said so of users only): on the
  hosted edition both grow with the server. `GET /library/folders` answers `{scanning,
  files_scanned, folders}` — the array inside, since the scan's state is the server's, not a
  folder's —, `member` as the plan said (v1: household admins), `media: music|books`.
- `AdminUser` says `blocked` (v1's `is_active` turned round), as `MemberRecord` does (rule 1);
  `PATCH /admin/users/{id}` takes `{display_name, instance_role, blocked}` and answers the account.
  A changed `instance_role` signs the account out everywhere (decided the safer way): the role
  travels in the access token, so on v1 a demoted administrator kept the server until the token
  expired.
- No credit figures in `/admin/stats` and `/admin/families/{id}` (v1's `credit_balance_micro`,
  `balance_micro`, `last_charge`, `entries` come from the hosted edition's hooks): phase 13 adds
  them, additively, with the credit grant.
- The disc job: `disc_backfill`, 20 files a pass, one pass at a time, the next queued on the
  worker's next tick as long as tracks are left (one look into a partial index that is empty
  once done), held while a library scan runs, as checksums are. It is in the default set because
  the FOSS compose file sets no `WORKER_JOB_TYPES`; a deployment that lists its types (the hosted
  edition's containers) must add it. Each pass's `{checked, left}` is in `GET /admin/jobs`.
- `GET /admin/jobs` is left out of `v2_walk`'s generic paging (new jobs arrive at its head while
  it is walked); `v2_admin` pages it allowing for that.


v1 ops: `POST /setup/complete`, `POST /auth/admin-create-user`, `GET /users`, `PATCH`/`DELETE
/users/{id}`, `POST /users/{id}/revoke-sessions`, partly `GET /users/{id}`, admin families (6),
`GET /admin/stats`, `GET /jobs`, `GET /jobs/{id}`, `GET /library/folders`,
`POST /library/folders/scan`.

Also from phase 6: the disc-number back-fill (v1 `POST /music/tracks/discs`, not carried over as
a route) becomes a job — a slow sweep over tracks with `disc_checked = false`, a few at a time
like the checksum sweep (each reads a file), in `WORKER_JOB_TYPES`' default set, its progress
visible in `GET /admin/jobs`. Not added in phase 6: it would read every old upload's file again
on servers upgraded from before disc numbers were kept, and the jobs it would show up in are
this phase's.

| v2 | Permission | Request → answer |
|---|---|---|
| `POST /setup` | `public`, only while `setup.complete` is false (`409 setup_complete`) | `{email, password, display_name}` → `Me`. |
| `GET`, `POST /admin/users`; `GET`, `PATCH`, `DELETE /admin/users/{id}` | `instance_admin` | Paged list with `q=`. |
| `DELETE /admin/users/{id}/sessions` | `instance_admin` | |
| `GET /admin/families?state=active|trashed`; `GET`, `PATCH`, `DELETE /admin/families/{id}` | `instance_admin` | `PATCH {locked}`. |
| `GET /admin/stats` | `instance_admin` | |
| `GET /admin/jobs`, `GET /admin/jobs/{id}` | `instance_admin` | Paged, filter `state=`, `kind=`. |
| `GET /library/folders` | `member` | Bounded array with last scan. |
| `POST /library/folders/scan` | `family_admin` | → `202`. |

Tests: the matrix matters most here: every `/admin/*` is `403` to a family admin and a member;
setup refuses once done (the compose stack is set up, so the positive case runs in a unit test
and in CI's fresh stack before the suite's own setup); a locked family cannot sign in; jobs page
without loss.

### Phase 13 — hosted edition, `audio2` (L) — done 2026-11-17

Done, v2 revision 15, in two repositories: the core's seams here, the routes and bodies in
`audio2` (committed there against this repository's checkout through an uncommitted `[patch]`;
its pin moves to the release tag in phase 14). `api-v1-v2-coverage.md` has no `missing` row left:
`scripts/check-v2-coverage.py --strict` passes, also with `--hosted-openapi`. What the plan said,
with these differences:

- **The core's seams**: `Hooks::edition_kinds` (an `EditionKind` the core plays, keeps progress
  on and sends in the change feed by its name only), `Hooks::live_events` (polled by `/events`
  with the identify runs), `Hooks::family_credit` and `Hooks::credit_total` (typed, for v2;
  v1's map-shaped hooks stay), `v2::document_with(edition)` and `http::router::finalize_v2`,
  which also lets personal access tokens use the edition's routes by their `x-scope` (they were
  refused before: the scope table knew the core's routes only).
- **Translation progress stays the household's** (decided the safer way): the position lives
  on the translation, as v1 and the shipped app have it; `/me/progress/translation/{id}` reads
  and writes that one position, with the later-wins rule, and keeps none per device
  (`progress_devices` allows the core's kinds only). So the one model is one storage, and the
  alias cannot drift from it. `include=items` does not embed translations. The snapshot does not
  carry them; the change feed does, through a hosted trigger (hosted migration 0008).
- **Money**: `_minor` (whole cents, toward zero) beside every `_micro` amount, with `currency`;
  the exact millionths stay because storage is charged by the day in fractions of a cent.
  Narration and translation quotes were whole cents with a currency already and keep their
  names. The credit grant requires `Idempotency-Key` (`400 idempotency_key_required`), takes the
  ledger's currency only (`400 unsupported_currency`) and at most 1,000,000 minor units either
  way (`400 invalid_amount`); the administrator's address is folded into the note, as on v1.
- **Paging when asked**: `GET /translation/recent` and `/translation/feeds/{id}` answer the old
  array without `limit` or `cursor` and a `{items, next}` page with either (the recent list then
  reaches past its 50 and filters visibility in SQL); `/summaries/episodes/{id}/questions`
  already answered `{items}` and adds `next` when paged.
- `GET /narration/cover-prompt` is `public`, as on v1 (a constant text).
- **Not done here**: the one real paid job on canary with test credit, and the hosted checks
  against canary, wait for the release (no deploy before it). Locally the hosted checks ran
  against a hosted build on this repository's checkout, paid work up to quote and estimate.
- The `WORKER_JOB_TYPES` lists in `audio2`'s compose files name `disc_backfill` (phase 12).


v1 ops: `POST /admin/families/{id}/credit` (missing) and the 23 routes mounted under `/api/v2`
with v1 bodies.

What must change in `audio2`:

1. **Pin** the core tag that carries phases 0–12 (`hosted/Cargo.toml`), and copy any Dockerfile
   stage changes, as usual.
2. **An OpenAPI document for the hosted v2 routes.** `hosted/` has no `utoipa` today, so the
   permission test and the access walkers never see these routes. The core gains
   `v2::document_with(edition)`; `audio2` writes `docs/api/openapi-v2-hosted.json` (core + hosted),
   with `x-permission` on every hosted operation and the same `openapi_v2_*` tests.
3. **Bodies to the rules, additively** (§1.4 item 6): problem details already come from the core
   middleware; add `_ms` fields beside seconds, amounts as integer minor units with `currency`,
   ids as UUIDs; page what grows (`GET /translation/recent`, `/translation/feeds/{id}`,
   `/summaries/episodes/{id}/questions`) with optional `limit`/`cursor` while the first page
   stays what a shipped app expects (question 5).
4. **Translations as a v2 kind.** An edition hook in the core lets `playback-info`, `/me/progress`
   and the change feed accept kind `translation`; `PUT /translation/episodes/{id}/progress` then
   writes the same progress model and stays as an alias for the shipped app.
5. **`POST /admin/families/{id}/credit`** under v2: `instance_admin`, `{amount_minor, currency,
   note}`, `Idempotency-Key` (money).
6. **Job progress through `/events`** for narration and translation jobs of the caller (as
   identify runs already are), so the apps stop polling `GET /narration/jobs/{id}`.
7. The Stripe webhook stays where it is (not carried over).
8. `hosted_feature_for` already names the v2 paths; add any new one.

Tests: hosted checks live in the core conformance suite gated on `ctx.feature("billing" |
"narration" | "translation" | "summaries")` (README rule), run by `audio2` CI against canary;
paid work is exercised up to `quote`/`estimate` only, plus one real job on canary with test
credit; the access/isolation walkers read `openapi-v2-hosted.json` when given
(`--openapi-v2`). CHANGELOG in `audio2`.

### Phase 14 — release (S)

- `scripts/check-v2-coverage.py --strict` in CI: no `missing`, no `partly`.
- `V2_STATUS = "stable"`, `openapi-v2.json` without `-draft`, the wording in CLAUDE.md,
  API_COMPATIBILITY.md §2/§4/§11, `api-v2-plan.md`, `api-v2-first.md`, the gaps plan (the
  uncommitted edits of 2026-11-17, adjusted to the date).
- The coverage doc's counts; `api-v2-gaps-plan.md` all ticked or moved to a "later" list.
- A core release (contract revision stated), then `audio2` pins it and deploys.
- Whether v1 now gets `deprecations` and `Sunset` headers is question 8.

---

## 4. The test bar

"Completely tested" means all of this, for every v2 operation, before its row in the coverage
doc says covered.

### 4.1 Per operation

| Test | Where | What it proves |
|---|---|---|
| Handler test | `cargo test`, next to the handler | The handler's own logic without a database: request validation and its problem codes, the v1 → v2 shape mapping (seconds → ms, roles, visibility words), cursor keys, permission rules that are code (self or admin, last admin). Question 4 asks whether to add `#[sqlx::test]` with PostgreSQL in the CI test job for the database half. |
| Success | conformance | The documented answer, its shape and status, on the compose stack. |
| Permission matrix | conformance, generic | Anonymous → `401`; a member → allowed, or `403`/`404` for `family_admin`/`instance_admin`/`owner`; a family admin → allowed or `403` for `instance_admin`; walks `openapi-v2.json` so a new route is covered the day it exists. |
| Other family | conformance, generic | Every v2 path carrying an id of family A answers family B with no `2xx` (the `isolation` walker on v2). Skipped on one-family installs, as today. |
| Problem shapes | conformance | Each documented `4xx` with its `code`; `application/problem+json`. |
| Paging | conformance, generic | Every operation with a `cursor` parameter, walked with `limit=2` against `limit=500` on seeded data: same items, no repeats, stable order. |
| Idempotency | conformance | Creates and paid work: the same key answers the first result and creates nothing more; the same key with another body is refused. |
| Change feed | conformance | Every write to a kind a client caches shows up in `/sync/changes`; a delete as a tombstone (rule 7). |
| v1 unchanged | conformance | The v1 suites still pass. |

### 4.2 Conformance additions (phase 0)

- `access` and `isolation` walk `openapi-v2.json` too (and `openapi-v2-hosted.json` with
  `--openapi-v2`), with v2 path parameter fills.
- A `paging` walker as above.
- **Operation coverage**: `Ctx` records each v2 call as (method, path template, status);
  `--report` lists them, and a final check fails if a v2 operation never answered `2xx` in the
  run, except those whose module is off (listed as skipped). This is the executable form of
  "untested behaviour is not part of the API".
- New checks go in `suites/v2.py` as functions per area (as today), or a `v2_<area>.py` suite when
  one grows past a few hundred lines.

### 4.3 `scripts/check-v2-coverage.py`

The mapping lives in `api-v1-v2-coverage.md` itself, so there is one table to keep (rule 12):

- Every operation in `docs/api/openapi.json` is a row; the hosted v1 routes are read from a list
  in the doc's hosted section (or `audio2`'s document when given).
- Every v2 operation a row names in backticks (`METHOD /path`) exists in `openapi-v2.json` (or
  the hosted document); a field reference (`Member.image`) must name a schema property.
- `not carried over` needs a reason in the note.
- The counts table matches the rows.
- Report mode (default) prints the missing and partly rows and exits 0; `--strict` fails on any.
  CI runs report mode from phase 0 and strict from phase 14.

### 4.4 After every phase

`cargo check`, `cargo clippy -- -D warnings`, `cargo test` (including the regenerated
`openapi-v2.json`), the console build when the frontend changed, the full conformance suite
against the compose stack (local and S3 storage, as CI's matrix), `check-api-contract.py`,
`check-v2-coverage.py`, `gitleaks detect --no-git --source .` before a push. New list queries
over catalog tables get an `EXPLAIN` on `conformance/tools/scale_catalog.py`'s 600k catalog and
a line in `CAPACITY.md`.

---

## 5. Open questions for Kornel

1. ~~Per-member policies, grants and audience~~ — answered: they stay on v1.
2. ~~Not carried over as proposed~~ — decided 2026-11-17 in line with question 1, pending
   Kornel: sharing a playlist with chosen members (`share`'s members part, `audience`, copies)
   is part of the per-member model and stays on v1; the disc-number back-fill is not a route on
   v2 and becomes a server job in phase 12. Kornel may reverse the first: it would be
   `GET`/`PUT /playlists/{id}/audience` and `POST /playlists/{id}/copies`, as the phase-6 table
   sketched.
3. ~~Rule 4~~ — answered: household-bounded lists may stay arrays; reworded in phase 2.
4. ~~Handler tests~~ — answered: `#[sqlx::test]` may be used; PostgreSQL joins the CI test job
   with the first phase that needs it (§1.4 item 2).
5. ~~Hosted bodies~~ — answered: additive changes only, deviations documented.
6. ~~The "v2 stable" edits~~ — answered: `draft` until phase 14, revision counter and contract
   check now (§1.4 item 7).
7. ~~Permanent links for catalog art~~ — answered: yes, people's pictures keep a short expiry.
8. ~~v1's deprecation~~ — answered: only once the apps have moved, not at the release.
