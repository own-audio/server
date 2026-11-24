# Changelog

All notable changes to the own.audio server. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
semver. Each release states the **API contract revision** it serves
(`GET /api/v1/server` → `api.revision`), see `docs/API_COMPATIBILITY.md`.

## [Unreleased]

### Added

- Sign in with Apple in a browser, for apps without Apple's SDK (Android): `GET
  /api/v2/auth/apple/start` (the app's return address, a PKCE S256 challenge and its state) sends
  the browser to Apple; Apple answers at `POST /api/v2/auth/apple/return`, which checks the id
  token (the Services ID's audience, this sign-in's nonce) and sends the app a one-time code; the
  new `urn:own-audio:params:oauth:grant-type:apple-code` grant on `POST /api/v2/auth/token`
  spends it with the PKCE verifier and signs in or makes the account as Apple's id token does.
  Discovery says so with `auth.apple_web` and the grant in `auth.grants` (API v2 revision 21).
  Needs `AUTH__APPLE__WEB_CLIENT_ID` (compose: `APPLE_WEB_CLIENT_ID`), `SERVER__BASE_URL` and the
  Return URL `<SERVER__BASE_URL>/api/v2/auth/apple/return` registered for the Services ID; app
  return addresses come from `AUTH__APPLE__APP_REDIRECT_URIS` (compose:
  `APPLE_APP_REDIRECT_URIS`; the own.audio apps' by default). Migration 0135; INSTALL.md says how.
- Console: an "AI voice" badge on a book read by an AI voice in its own language (`origin.kind`
  `narrated`) — on cards, rows, the book's page (with "Read by the AI voice …"), the player and
  Made here. A machine translation keeps its own badge; an item never gets both.

### Changed

- Console: account amounts are shown on the ledger's 4-decimal grid — credit balance, recent
  charges, paid-work quotes and their lines, charged amounts on jobs, episode translations,
  summaries and identify runs, and the daily cap ($9.8983, -$0.0017, +$5.0000), rounded half to
  even from micro-units. List prices (per GB, top-up buttons, per-hour voice prices) keep two
  decimals. The Narrate a book / Translate a book confirm step lists what the family pays for each
  part, with a new Text preparation line, adding up to the total, from the hosted edition's new
  `*_price_micro` quote fields (the providers' costs in cents from a server without them); charged
  amounts use `charged_price_micro` when sent.

## [1.0.8] - 2026-11-22

Contract revision 29. API v2 revision 20.

Books say their language and where they came from, so apps can mark a machine translation;
v2 refusals keep their figures; the console gets a choice of voice, Translate a book, Made here
and cover changes. The first start runs migration 0134 in seconds.

### Added

- Books have a `language` and an `origin` (migration 0134): `language` and `origin` on v1's
  book answers and `/library/changes` (contract revision 29) and on v2's `Book` in lists,
  `GET /books/{id}`, `/sync/snapshot` and `/sync/changes` (API v2 revision 19). `origin` is
  `null` for an ordinary book and is set only by the server — `{kind: "narrated", voice}` or
  `{kind: "machine_translated", source_language, translator, voice}` — so every client can mark
  a machine translation. `PATCH /api/v2/books/{id}` takes `language` (a BCP 47 tag; `null`
  clears it); `db::audiobooks::set_book_origin` is how an edition records a book it made.

- Web console: a small "Machine translation" badge on a machine-translated book (`origin.kind`
  `machine_translated`) in the library, on Home, on its page and in the player, and on a translated
  episode the server flags `machine_translated` (Your translations, the show's episode rows, the
  episode's translations, the player). The book's page adds "Machine-translated by Google from
  Czech to English, read by Sarah." unless its description already says it; Czech names the
  languages in the genitive. A book without `origin` or a row without the flag shows nothing.
- Web console: Create ▸ Made here, first under Create: your narrations from any device
  (`GET /audiobook-gen/jobs`, paged) — running with stage and blocks, ready with what it cost,
  Listen now and the cover actions, failed with the server's error and the charge — and the
  family's translated episodes with Play. On a server without the job list, the jobs started in
  this browser.
- Web console: the daily-cap refusal without `daily_cap_micro` says the sentence without a figure
  instead of "(—)"; the refusal's numbers are read from v2 problem members too.
- Web console: Narrate a book and Translate this episode offer the voice models a hosted server
  lists at `GET /narration/voice-tiers` (Chirp 3: HD by default; Gemini 3.8 Flash TTS and
  Flash-Lite TTS marked Preview), each with its price per hour of audio in the chosen language
  and a sample to hear, then that model's voices. The quote and the request carry `voice_tier`,
  and the summary names the model. A server without tiers shows the voice list as before and is
  sent nothing new.
- Web console: an EPUB or MOBI can keep its own cover when narrated (`cover_mode` `book`; one is
  made when the file has none).
- Web console: Translate a book under Create (`/translate-book`), the narration wizard with
  translation always on — "The book is in" and "Translate into", which must differ. Shown on a
  server with narration and translation (or `book_translation`, once a server sends it) to a
  member who may order paid work; the Add audiobook dialog's "Translate and narrate" opens it.
- Web console: a finished narration's cover can be changed on its status page, free: a new one
  (made for you or from your own brief), the book's own when the file has one, or none, with the
  number of free covers left (`POST /audiobook-gen/jobs/{id}/cover`). Each refusal
  (`not_finished`, `no_book_cover`, `cover_limit_reached`, `cover_failed`, `cover_unavailable`)
  has its own sentence.
- Web console: the confirm step of narrating or translating a book and the episode translation
  dialog carry the personal-use note (the server's `personal_use_notice` when it sends one), and
  a finished narration says it is based on the original work, for personal listening. The
  confirm step also shows the family's credit.

### Changed

- `/api/v2` problem details keep every member of the refusal they were made from as an extension
  member beside `code` (RFC 9457 §3.2): `409 daily_cap_reached` carries `daily_cap_micro` and
  `spent_today_micro`, `409 too_many_jobs` `running` and `max_running`, `402 insufficient_credit`
  `required_micro` and `available_micro`. Server errors still carry none. API v2 revision 20.

- Web console: the narration wizard shows the cover as free, and says that the price shown is the
  most you pay and that a failed job costs the part already made.
- Web console: a refused narration, book translation or episode translation is told by its code:
  not enough credit (with a link to Storage & credit), the day's limit for paid work (the limit
  the server sent), too many jobs running, an unconfirmed email address, no permission, or a
  voice model no longer offered — then the wizard reads the models again, falls back to the
  default and asks for a voice. The episode dialog shows the price note too, and a translation
  made before (`200`) says so instead of "Translation started".

## [1.0.7] - 2026-11-22

Contract revision 28. API v2 revision 18.

Undoing an identify takes everything back, the organizer's Changed means a change in effect, a
failed paid job in the console says what it cost, and `url` 2.5.9 for a security advisory. No
migrations.

### Fixed

- `url` 2.5.9 for RUSTSEC-2026-0592 (a stack overflow in `Url::origin()` on nested `blob:` URLs);
  the `webpki-roots` exception in `deny.toml` is gone, the advisory no longer applies.
- Web console: a failed narration or episode translation says what it was charged for the part
  already made (the hosted edition charges what Google had already billed), and "You have not
  been charged" only when that was nothing. The narration wizard sends its cover choice with the
  quote request, since a generated cover is part of a hosted quote now.
- `DELETE /api/v2/tracks/{id}/identity`, `POST /api/v2/identify/runs/{id}/revert` and
  `POST /api/v1/music/tracks/{id}/metadata/revert` undo everything the identify changed. The
  MusicBrainz artist id it gave the song's artist goes too (it stayed, so a later run revert
  found nothing left to undo), unless another song credited to that artist still carries it —
  then the last song undone takes it. A disc number set by an accepted proposal goes back,
  and the identify's entries in the edit history are marked undone. Undoing an identify's
  batch from the history (`POST /api/v2/edits/batches/{id}/undo`) keeps the artist's id by
  the same rule.
- `GET /api/v2/organize/tracks` and `/organize/facets`: *Changed* (`edited`) means a change
  is still in effect. A song whose identify or edit was undone, reverted or overwritten since
  is no longer counted; the history keeps every entry.

## [1.0.6] - 2026-11-22

Contract revision 28. API v2 revision 18.

Token refresh on its own rate limit in v2, readable refusals of files over the upload limit,
book file titles without track numbers. No migrations.

### Added

- `POST /api/v2/uploads`: a file over the single-upload limit is refused with `size_bytes` and
  `max_bytes` beside the problem's `detail`, so an app can show the sizes its own way. The
  `code` stays `bad_request`. Contract change: API v2 revision 18.

### Changed

- `POST /api/v2/auth/token`: the `refresh_token` and token-exchange grants count against the
  refresh limit (`SERVER__RATE_LIMIT__REFRESH_PER_MINUTE`, 120 per minute per address), as v1's
  `POST /auth/refresh` always has. Before, every grant shared sign-in's 30, so a household's
  devices refreshing behind one address could use up its members' sign-in attempts. Password,
  two-factor, device code, provider and authorization-code grants keep sign-in's limit and the
  per-account lockout. `docker-compose.yml` now passes `SERVER__RATE_LIMIT__REFRESH_PER_MINUTE`
  through to the server (it was read only when set some other way).

### Fixed

- Uploads over the single-upload limit say so in words ("The file is 6.0 GB; one upload can be
  at most 5 GB.") instead of "larger than the 5368709120-byte single-upload limit", on
  `POST /api/v2/uploads` and `POST /api/v1/uploads/presign` alike.
- A book file's title from its name drops the track number and the extension:
  `01 - One.mp3`, `01. One.mp3`, `01_One.mp3` and `1-One.mp3` are all "One" (they were
  "01   One" and the like). A number that is the whole name stays (`1984.mp3` is "1984"), as do
  hyphens inside a name. The same for books made from uploads (v1 and v2), multipart uploads and
  the files of library folders that have no title tag; existing titles are not changed.

## [1.0.5] - 2026-11-20

Contract revision 28. API v2 revision 17.

A security update for the mail: six SMTP advisories in `lettre`. No API change, no migrations.

### Changed

- Console: arriving from a TV's QR code, the sign-in page says "Connect your TV" and offers making
  an account as plainly as signing in, with a full-width button; afterwards it returns to the
  approval as before.

### Fixed

- `lettre` 0.11.24, for six SMTP advisories (RUSTSEC-2026-0560 to 0565). The server's mail uses
  implicit TLS by default, which the STARTTLS one does not reach; the others are fixed either way.
- No mail to addresses on reserved test domains (`example.com`, `.test`, `.invalid`, …). The
  provider bounced each one back to the sender's mailbox, so every test run flooded it.

## [1.0.4] - 2026-11-18

Contract revision 28. API v2 revision 17, **stable**.

1.0.4-beta.3's code, out of beta: every `/api/v1` function is in `/api/v2`, and v2 is stable
(additive only from here). v1 is served unchanged for the apps built on it. The changes since
1.0.3 are in the three betas below; `UPGRADING.md` says what an upgrade does.

### Changed

- The conformance suite runs against a server behind Cloudflare (its own User-Agent, waiting out
  rate limits, weak ETags, ranged image answers) and against the hosted edition (credit for the
  run's households, the console's own origin).

## [1.0.4-beta.3] - 2026-11-17

Contract revision 28. API v2 revision 17, **stable**. A beta, like beta.2: test it before you
rely on it.

### Fixed

- Deleting several items at once from a console on another origin (app.own.audio talking to
  api.own.audio) works: the browser refused the request after the preflight, because the
  console's `X-Trash-Batch` header was missing from the allowed CORS headers.

## [1.0.4-beta.2] - 2026-11-17

Contract revision 28. API v2 revision 17, **stable**. A beta, like beta.1: test it before you
rely on it.

### Added

- A `beta` image tag: each beta moves it (`latest` and `1.0` stay on the last stable), and the
  *retag image* workflow moves it by hand.
- A human check on the email confirmation page: a console built with `VITE_TURNSTILE_SITEKEY`
  shows Cloudflare Turnstile before confirming, and sends its token as the optional `human_check`
  on `POST /api/v1/auth/email/verify` and `POST /api/v2/auth/email-verification`. The address is
  confirmed with or without it; the new `Hooks::email_verified_by_link` passes the token and the
  caller's address to the edition, which may grant less without a passed check (the hosted
  edition's welcome credit). The console's Content-Security-Policy allows
  `challenges.cloudflare.com`. Contract revision 28, v2 revision 17.

### Changed

- The console's Tailwind packages are build-time dependencies, out of the production audit
  (picomatch GHSA-fc5q-39xc-vjpf has no fixed version); chrono's unmaintained notice
  (RUSTSEC-2026-0543) is ignored with its reason, as no vulnerability is known.

### Security

- The image's FFmpeg carries only audio components: no video or image decoder is built, so a
  crafted upload cannot reach the ones recent FFmpeg CVEs hit (CVE-2026-30997 in the AV1
  decoder, which 8.1.3 does not fix; tiff, rasc, lcldec, rscc). Files with a video stream or
  an embedded cover play, scan, fingerprint and measure as before. The image is ~23 MB smaller.

## [1.0.4-beta.1] - 2026-11-17

Contract revision 27. API v2 revision 16, **stable**.

**A beta.** Large parts of the server changed (every v1 function moved to `/api/v2`); test it
before you rely on it, and expect things that do not work yet. It is not `latest`.

Every `/api/v1` function is in `/api/v2` (`docs/api-v2-completion-plan.md`, phases 0–14), or
recorded in `docs/api-v1-v2-coverage.md` as not carried over with its reason. v1 is served
unchanged for the apps built on it, without deprecation headers until they have moved.

### Changed

- `/api/v2` is stable: `GET /.well-known/own-audio` → `api[]` says `status: "stable"` for v2,
  and `openapi-v2.json` is version `2.15` (was `2.15-draft`). Nothing else in the contract
  changes with it; it stays additive only, as it has been since revision 1.
- CI runs `scripts/check-v2-coverage.py --strict`: a v1 operation without a v2 counterpart or a
  reason fails the build.
- `/api/v2` counts its revisions: discovery reports revision 1 (`openapi-v2.json` version
  `2.1-draft`, was `2.0-draft`), up by one with every addition. It stays `draft` until the
  release that completes `docs/api-v2-completion-plan.md`, when it becomes stable; the apps
  already ship on it, so v2 changes are additive only and `scripts/check-api-contract.py`
  checks v2 as well as v1.
- `/api/v2/media` — the server's signed media route for v2; v2 links no longer point at
  `/api/v1`. Every link the server signs itself (local storage, library folders, S3 behind
  `STORAGE__PROXY`: v2 upload targets, v1 stream answers) now points at `/api/v2/media`; links
  handed out before keep working on `/api/v1/media`, which takes the same signature. v2
  revision 2.
- `GET /api/v2/stream` sends an original the server keeps itself (`200`, or `206` for a
  `Range`) instead of redirecting to the media route: one round trip less before the first
  byte. Object storage keeps the redirect. v2 revision 2.
- `image` links to catalog art (covers, artist and author pictures, podcast art) no longer
  expire: `GET /api/v2/images/{id}?s=…`, without `e`. Links with an expiry keep working until
  it passes. v2 revision 2.
- `Family.image` and `Member.image` are signed image links (`/api/v2/images/{id}`, good for one
  to two weeks) instead of `/api/v1` routes that want the access token. v2 revision 2.
- A new profile or household picture is a new stored object rather than an overwrite of the
  old one, so an image link always shows the picture it was made for; the replaced one is
  removed by the storage sweep.

- The job worker starts a refresh or a download a request queued at once, rather than at its
  next idle poll (up to 30 s later).
- `docs/api-v2-rules.md` rule 4: lists that grow with the catalog are cursor-paged; lists
  bounded by the household may stay arrays.

- The change feed (`/api/v2/sync/changes`) also carries: a move or rename in the own.audio
  folder (one change of the item, with its new `path`); a per-item grant (a change of that item
  for that member only); a member's media policy (a `reset` for that member only); a checksum
  computed after the upload; and what an item makes visible when it appears, is shared,
  unshared, trashed or moved — its album, release group, recording, genres and contributors,
  and a show's episodes. Before, a member could receive a shared track without its album, or a
  shared show without its episodes. Migration 0126.
- A new member joining the household is a `reset` for the newcomer only, and for everyone only
  when they bring a library along; before, every new account sent every device in the household
  back to a full snapshot.
- Continue listening reads a listener's progress newest first through new indexes
  (`(user_id, updated_at)` on track and episode progress): 0.7 ms instead of 33 ms for 15,000
  unfinished songs in a 600,000-track catalog.
- `POST /api/v2/books` and an uploaded episode queue the checksum of their files at once, as a
  track upload does, instead of waiting for the periodic sweep.

- Console wording follows `i18n/GLOSSARY.md` and matches the apps: American English throughout
  (catalog, favorites, canceled, recognize); Czech "obálka" for a cover, never "obal"; the
  permanent-delete dialogs say "Smazat" in their text as in their titles. The glossary records
  the 2026-11-16 decisions: Follow / Unfollow for podcasts, "Just me", "Storage & credit",
  "Muted music".
- Conformance runs on a server with many households (the hosted edition): a member a suite
  needs in a household admin's household joins it through an invitation (`Ctx.make_member`) in
  `entities`, `v2_admin`, `v2_podcasts` and `v2_sync`, instead of assuming everyone shares the
  server's one household. `filesync` and `gpodder` skip their feed checks, with the reason, when
  the server cannot fetch a feed from the machine running the suite (a server outside Docker
  does not know `host.docker.internal`).
- Conformance checks v2 operations no suite answered `2xx` before: `PATCH /api/v2/family`,
  `POST /api/v2/family/join`, `DELETE /api/v2/me/sessions`, `GET /api/v2/library/quarantine` and
  `DELETE /api/v2/library/quarantine/{track_id}`, `PUT /api/v2/albums/{id}/release-group`,
  `POST /api/v2/albums/{id}/merge` and `PUT /api/v2/podcasts/{id}/credits`.

### Added

- `POST /api/v2/auth/register` — sign-up with a password, with or without an invitation;
  answers the account (`Me`), signing in stays `POST /auth/token`. Problem codes
  `registration_closed`, `email_taken`, `weak_password`, `invalid_email`, `invalid_invite`.
  v2 revision 3.
- `POST /api/v2/auth/password-reset` and `…/password-reset/confirm`, `PUT /api/v2/me/password`
  — the forgotten-password mail and its link, and changing the password (other devices signed
  out), with v1's password rule and the sign-in rate bucket. A server without mail answers
  `501 feature_unavailable` instead of pretending it sent something. v2 revision 3.
- `POST /api/v2/auth/email-verification` (the mailed link's landing call) and
  `POST /api/v2/me/email-verification` (another link: `202`, `409 already_verified`, `501`
  without mail). v2 revision 3.
- `GET /api/v2/me/security-events` — the account's security trail, cursor-paged, the address
  cut to its network. v2 revision 3.
- `POST`, `PUT`, `DELETE /api/v2/me/two-factor` — set up, confirm (recovery codes) and turn off
  two-factor sign-in; `two_factor` in `GET /me`. v2 revision 3.
- `PATCH /api/v2/me` (display name, `recommendations_enabled`, `discovery_languages`),
  `DELETE /api/v2/me` (confirmed with the password when the account has one; `409 last_admin`),
  `PUT`/`DELETE /api/v2/me/image` (an upload id or the image itself; anyone may set their
  picture). `Me` gains `image`, `recommendations_enabled`, `discovery_languages`. v2 revision 3.
- `GET`/`POST /api/v2/me/subsonic-key` — the key for third-party Subsonic players, and a new
  one. v2 revision 3.
- `POST /api/v2/auth/token` takes two more grants: `authorization_code` with PKCE for the
  desktop apps' Google and Microsoft sign-in (loopback redirect only), and RFC 8693 token
  exchange (a refresh token for a second, independent device — the Mac's Finder extension).
  The credential routes (password, two-factor, Subsonic key, deleting the account) need a
  signed-in session; a personal access token gets `403`. v2 revision 3.
- `/.well-known/own-audio` adds `auth.grants`, `auth.clients` (the providers' desktop and web
  client ids), `auth.email_verification`, `setup.complete` and, on a public demo, `demo`.
  v2 revision 3.
- Handler tests through the v2 router without a database (problem codes, the grants' checks,
  the sign-in bucket on the password and code routes, discovery), and the code grant's exchange
  against a stub provider. Conformance: the `v2_account` suite.
- `scripts/check-v2-coverage.py` — checks `docs/api-v1-v2-coverage.md` against both OpenAPI
  documents (every v1 operation has a row, every v2 operation and field a row names exists,
  the counts add up) and lists what is still missing; run in CI. `--strict` fails on open rows.
- Conformance: the `v2_walk` suite walks `openapi-v2.json` — the permission matrix, another
  member's (or household's) ids on every operation, problem details on every refusal, and
  every cursor-paged list in each of its orders, two at a time against one page. After a run,
  `run.py` lists the v2 operations that never answered 2xx (`--v2-coverage-strict` to fail).
- `POST /api/v2/podcasts` follows a feed (`{feed_url, visibility?}`): fetched at once, a show
  the caller has already answers `200`, `Idempotency-Key` as on every create; any member may
  follow (an uploaded show still needs the right to add to the library). `DELETE
  /api/v2/podcasts/{id}` unfollows (the owner, or a household admin for a shared show), and the
  change feed sends the removal of the show and of each of its episodes. Problem codes
  `invalid_feed_url`, `youtube_not_supported`, `feed_unreadable`, `uploaded_podcast`. v2
  revision 4.
- `POST /api/v2/podcasts/{id}/refresh` (and `POST /api/v2/podcasts/refresh` for every show) —
  queued, `202`; the show's and episodes' art is stored on the way (v1's `sync-images`), and new
  episodes arrive through `/sync/changes`. v2 revision 4.
- `PATCH /api/v2/podcasts/{id}` (`auto_store`), `POST /api/v2/podcasts/{id}/store` (the back
  catalogue, or the newest `latest`; `preview` counts), `PUT`/`DELETE
  /api/v2/episodes/{id}/stored` (keep one episode on the server, queued; or drop the server's
  copy into the trash). `Podcast` gains `source` and `auto_store`. v2 revision 4.
- `GET /api/v2/me/podcasts/unplayed` — how many episodes of each show the caller has not
  finished, cursor-paged, one grouped query per page. v2 revision 4.
- `GET /api/v2/episodes/{id}/transcript` — the publisher's (or the uploaded) transcript read by
  the server into timed segments, with the speaker where WebVTT voices or Podcast Index JSON
  name one; `404 no_transcript`. v2 revision 4.
- The podcast directory on v2: `GET /api/v2/podcasts/directory/search`, `…/categories`,
  `GET /api/v2/podcasts/directory?category=` (cursor-paged), `…/similar?podcast_id=|feed_url=`
  (one route for v1's two), `…/preview?feed_url=` (the show and its newest episodes with their
  files, nothing stored). In the caller's discovery languages; what the caller has carries its
  `podcast_id`. A server without the catalogue or the search answers `501
  feature_unavailable` (`modules.podcast_discovery`, `modules.podcast_search`), a directory
  that does not answer `502 directory_unavailable`. v2 revision 4.
- Conformance: the `v2_podcasts` suite (follow, unfollow, refresh, storing, unplayed,
  transcripts, the directory, their change-feed entries) against a feed it serves itself;
  `conformance/compose.feeds.yml` lets a test stack fetch it (and `filesync`'s and `gpodder`'s)
  on Linux, `conformance/compose.mail.yml` gives a test stack a mail catcher; CI's conformance
  job uses both.
- `GET /api/v2/sync/changes?kinds=` — read only some kinds (`track,book`, or one kind of state
  such as `progress.episode`), filtered by the server; a `reset` always comes through. v2
  revision 5.
- `GET /api/v2/sync/snapshot?fields=ids` — the snapshot as `{type: id, kind, id}` lines without
  records, for a client checking that what it holds is still there (about 60 bytes an item;
  replaces v1's `/sync/tree/ids`). v2 revision 5.
- The `member` kind in `/api/v2/sync/snapshot` and `/sync/changes` (`MemberRecord`: `user_id`,
  `display_name`, `label`, `image`, `left_at`): everyone in the household and everyone who left
  it, so an owner id always has a name. Someone who leaves keeps the name the household knew;
  a deleted account keeps none. A rename or a new picture is a change of the record. v2
  revision 5.
- `path` on `Track`, `Book` and `Episode` (where the file or folder sits in the owner's own.audio
  folder), `sha256` on `Track` and `Episode`, `relative_path` and `sha256` on `BookFile`; in the
  create answers (`POST /tracks`, `POST /books`, `POST /podcasts/{id}/episodes`) too. v2
  revision 5.
- `GET /api/v2/me/progress` takes several kinds (`kind=track,book,episode`, one list, most
  recent first), `unfinished=true`, and `include=items,podcasts,albums,contributors` with the
  records embedded once in `included` (`tracks`, `books`, `episodes`, `podcasts`): continue
  listening in one request (v1's `/library/continue`). v2 revision 5.
- Conformance: the `v2_sync` suite — a copy built from a snapshot and the change feed equals a
  fresh snapshot, for the owner and for another member, after uploads, a rename, sharing, a
  grant and a deletion; `kinds=` paging without loss; the ids-only snapshot against the full one;
  member records through a rename and an account deletion; continue listening. `filesync` runs
  on a server with one household, skipping its outsider checks there.
- `DELETE /api/v2/tracks/{id}` and `DELETE /api/v2/books/{id}` — into the 30-day trash (the
  owner, or a household admin for what is shared, as v1); the change feed sends the removal,
  with the album when it was the album's only song. v2 revision 6.
- `POST /api/v2/trash` — delete up to `limits.batch_max` (500) tracks, books, playlists,
  episodes' stored copies and companion files as one restorable batch, each item answering on
  its own (`ok`, `duplicate`, `not_found`, `forbidden`, `invalid_kind`); `Idempotency-Key`
  answers a retry with the first answer. v2 revision 6.
- The trash on v2: `GET /api/v2/trash` (cursor-paged, newest first; `scope=family` for a
  household admin; what a restore would charge), `POST /api/v2/trash/{kind}/{id}/restore`,
  `POST /api/v2/trash/batches/{batch}/restore` (a v1 `X-Trash-Batch` batch too),
  `DELETE /api/v2/trash/{kind}/{id}` (delete one for good) and `DELETE /api/v2/trash` (empty it,
  page by page). Restores reach the change feed as the item again. v2 revision 6.
- Conformance: the `v2_trash` suite (delete, batches, restore, purge with its stored object,
  who may, idempotency, paging, problem shapes, the change feed). The `trash` suite runs on a
  server with one household, so a full run with `--compose-dir` passes there; CI's conformance
  job now runs with it, so the SQL-backed suites (`trash`, `filesync`) and checks run in CI.
  `filesync` and `v2_walk` upload a real tone (`Ctx.tone()`) for their songs: the fingerprint
  job quarantined their random bytes part-way through the suite, depending on timing.
- `PATCH /api/v2/books/{id}` — correct a book's title, description, publisher, year and ISBN
  (`null` clears one); the owner's, as v1. v2 revision 7.
- A book's files on v2: `POST /api/v2/books/{id}/files` (many uploads at once, play order by
  their paths, a path the book has answers its file, `Idempotency-Key`),
  `PATCH /api/v2/books/{id}/files/{file_id}` (rename), `DELETE …/files/{file_id}` (`409
  last_file`: a book keeps one), `PUT …/files/order` (one ordered list, as a playlist's; `400
  invalid_file`). Renaming, removing and reordering are also a household admin's on a shared
  book, as v1. v2 revision 7.
- `POST /api/v2/books/{id}/identity/search` and `PUT /api/v2/books/{id}/identity` — identify a
  book from the household's book source, named as the tracks' identity routes; v1's outbound
  bucket. A source that does not answer is `502 source_unavailable`, a spent Google quota or a
  refused key `503 source_refused`, an id no search gave `400 invalid_volume_id`. v2 revision 7.
- `PUT`/`DELETE /api/v2/books/{id}/image` — a book's cover from an upload, or none; a new
  picture is a new object and link. v2 revision 7.
- Book tags on v2: `PUT /api/v2/books/{id}/tags` (the whole list), `Book.tags` in a full book
  record, `GET /api/v2/books?tag=` and `GET /api/v2/book-tags` (cursor-paged, with how many books
  the caller can see carry each). v2 revision 7.
- Series on v2: `GET`/`POST /api/v2/series`, `GET`/`PATCH`/`DELETE /api/v2/series/{id}` and
  `PUT /api/v2/series/{id}/books` (the ordered list in one call; places such as `1.5`). A series
  is its owner's, private or shared with the household (`visibility`), and shows each reader only
  the books they can see. v2 revision 7.
- Collections on v2: `GET`/`POST /api/v2/collections`, `GET`/`PATCH`/`DELETE
  /api/v2/collections/{id}`, `GET`/`POST /api/v2/collections/{id}/books` (paged; add many) and
  `DELETE /api/v2/collections/{id}/books/{book_id}` — the playlist shape for books, with
  `visibility`. v2 revision 7.
- Kinds `series` and `collection` in `/api/v2/sync/snapshot` and `/sync/changes`: a series, a
  collection, a book's place in a series, a collection's books and a book's tags write to the
  change log, and so does what a book makes visible (the series and collections it is in).
  Replacing a series' books is one change row of the series; renaming or sharing a series sends
  its books again. Migration 0127. v2 revision 7.
- Contributors on v2: `POST /api/v2/contributors` (a name the household has answers that one,
  `200`), `DELETE /api/v2/contributors/{id}` (a household admin's; `409 in_use` while anything
  credits them), `PUT`/`DELETE /api/v2/contributors/{id}/image` (a photo from an upload, kept for
  good; or forget it so the server looks again), `bio` in `PATCH /api/v2/contributors/{id}`.
  `Contributor` gains `bio` and `image_credit` (source, author, licence and links of a looked-up
  photo; v1's `image-info` in the record). v2 revision 7.
- Handler tests for every phase-5 route without a token, the outbound bucket on the identity
  routes, and an identity search against a stub Open Library. Conformance: the `v2_books` suite;
  `v2_walk` walks series and collections too (someone else's, paging).
- Track feedback on v2: `GET /api/v2/me/feedback` (cursor-paged, `value=dislike|ban`) and
  `PUT`/`DELETE /api/v2/me/feedback/track/{id}` (`{value: "dislike" | "ban"}`) — a listener's
  "not for me", which only their own automatic selection skips. Kind `feedback.track` in the
  snapshot and the change feed, for that listener only. v2 revision 8.
- `PUT /api/v2/tracks/{id}/lyrics` — the owner writes a track's lyrics, as text (plain or LRC)
  or timed lines; `null` takes them away and the file's own are read again. v2 revision 8.
- `GET /api/v2/tracks/{id}/file-tags` (what the file says, read now, written nowhere) and
  `POST /api/v2/tracks/{id}/rescan` (read them into the library again: `409 identified`,
  `422 no_tags`) — the owner's. v2 revision 8.
- `GET /api/v2/tracks/duplicates` — the caller's own tracks that are the same file, a group per
  SHA-256, cursor-paged and found in SQL; `include=tracks`. v2 revision 8.
- `POST /api/v2/smart-playlists/intent` — a sentence to a smart-playlist rule, the server's
  fallback for a device without its own model; `422 not_understood`. The demo account may ask.
  v2 revision 8.
- `PUT`/`DELETE /api/v2/tracks/{id}/image` (a track's cover from an upload, in the edit history)
  and `PUT`/`DELETE /api/v2/playlists/{id}/image` (a playlist's own cover; without one it shows
  its first track's). v2 revision 8.
- `kept` in `PATCH /api/v2/playlists/{id}` keeps a generated playlist (or lets it go again);
  `Playlist` gains `generated` and `kept`. v2 revision 8.
- Kinds `playlist` and `smart_playlist` in `/api/v2/sync/snapshot` and `/sync/changes`: a
  playlist's name, sharing, cover and entries, its trip through the trash (deleted, restored,
  purged), and a track it holds being shared, unshared, trashed or given a new cover or length
  reach the change feed as a change of the playlist; a deleted one is a tombstone. Entries stay a
  paged read. Migration 0128 (also an index on playlist entries by track, so purging a track no
  longer scans every entry). v2 revision 8.
- Handler tests for every phase-6 route without a token; unit tests for lyrics as timed lines,
  the intent's checks and the new kinds. Conformance: the `v2_music` suite; `v2_walk` walks
  feedback, lyrics and the new image routes.
- `PUT`/`DELETE /api/v2/family/image` (the household's photo, household admins) and
  `PUT`/`DELETE /api/v2/family/members/{user_id}/image` (a member's picture, by themselves or a
  household admin): `{upload_id}` or the image itself, at most 5 MB, as `/me/image`. v2
  revision 9.
- `blocked` in `PATCH /api/v2/family/members/{user_id}`: blocking signs the member out of every
  device at once and refuses their sign-in; `false` lets them in again. Not oneself (`400
  cannot_block_self`), not a server administrator (`403 cannot_block_instance_admin`). v2
  revision 9.
- `DELETE /api/v2/family/members/{user_id}` lets a member leave the household themselves, as
  v1 does; the last admin stays while others are in it (`409 last_admin`), and on a server with
  one household nobody leaves it (`409 one_household`). v2 revision 9.
- `POST /api/v2/family/members` makes an account for someone without a mailbox (a child, a
  grandparent) with its age bracket and rights, and answers the member and its `claim`
  invitation; `Idempotency-Key` as on every create. `POST /api/v2/family/invites/{id}/regenerate`
  gives an invitation a new code (`409 invite_used` when it is spent). `Invite` gains
  `member_user_id`. v2 revision 9.
- `POST /api/v2/family/join/preview` (`{code}`, without signing in, the device-code rate limit)
  — the household, its photo, the inviter and, for a claim code, the account; a used or expired
  code is `410 invite_used` / `invite_expired`. `POST /api/v2/auth/claim` (`{code, password}`,
  the sign-in rate limit) sets a claimed account's first password and answers `Me`; signing in
  is `POST /auth/token`. v2 revision 9.
- `GET`/`PATCH /api/v2/family/settings` — library audio analysis and the book source in one
  resource; members read, household admins change. v2 revision 9.
- `GET /api/v2/family/reports` (household admins; `state=open|resolved|all`, cursor-paged),
  `POST /api/v2/family/reports` (any member, on what they can see) and
  `PATCH /api/v2/family/reports/{id}` (`{resolved, note?}`: close with a note, or open again).
  Migration 0129 adds the note and an index for every state. v2 revision 9.
- Kind `family` in `/api/v2/sync/snapshot` and `/sync/changes`: the household as `GET /family`
  answers it, when its name or photo changes, and for one member when their own role or rights
  do. `MemberRecord` gains `blocked`, and blocking or letting someone in again reaches the change
  feed. Migration 0129. v2 revision 9.
- Handler tests for every phase-7 route without a token, the claim's checks before the database
  and the rate-limit buckets of the preview and the claim; unit tests for leaving and removing
  (the last admin, one household, unclaimed accounts) and the reports' words. Conformance: the
  `v2_household` suite.
- `GET`/`PUT /api/v2/me/queue` — the cross-device play queue in v2's words (`{kind, id,
  file_id?}` items, `index`, `position_ms`, the device that wrote it) and a `revision` counted
  over every write, v1's and Subsonic's included (migration 0130). `PUT` with
  `If-Match: "<revision>"` refuses a write made against an older queue (`412
  revision_mismatch`, with the current `revision`); without it the last write wins, as on v1.
  Items must be playable by the caller (`400 invalid_item`). v2 revision 10.
- `GET`/`PATCH /api/v2/me/settings` — the player's speed and intro/outro skips, the audiobook
  speed and skip steps (milliseconds, kept to the second) and each device kind's equaliser in
  one resource; out-of-range values are refused (`400 invalid_body`) rather than clamped as on
  v1. v2 revision 10.
- `GET /api/v2/me/notifications` (newest first, cursor-paged, `unread_only`; `item` names what
  a notification is about in v2's words, `read_at` when it was read) and
  `POST /api/v2/me/notifications/ack` (`{ids}` or `{up_to}`: everything up to one). v2
  revision 10.
- `GET`/`POST /api/v2/me/push-tokens`, `DELETE /api/v2/me/push-tokens/{id}` — push
  registration (a token another account registered moves to the caller); `modules.push` in
  discovery says whether the server sends any (`false` here: clients read the inbox and the
  change feed). v2 revision 10.
- Kinds `queue`, `settings` (the listener's, named by their id; in the snapshot too) and
  `notification` (change feed only) in `/api/v2/sync/changes`, so another device learns of a
  new queue, changed settings or a new or read notification from the feed and a `library`
  event on `/events` instead of polling. Migration 0130 adds the triggers and an index of each
  listener's notifications by time. v2 revision 10.
- `POST /api/v2/sync/push` takes `feedback` (`value`: `dislike` or `ban`) and `unfeedback`, so
  a dislike made offline waits for nothing. v2 revision 10.
- The demo account may write its own queue, settings, notification reads and push devices on
  v2, as it may on v1.
- Handler tests for every phase-8 route without a token and for discovery's `push`; unit tests
  for the queue's words and checks, `If-Match`, the settings' ranges, notification items and
  push tokens. Conformance: the `v2_listening` suite; `v2_walk` walks push devices.
- `GET /api/v2/me/stats` — a listener's listening over a range (`from`, `to`), in buckets of a
  day, week or month in their own time (`tz_offset_minutes`), per kind (`kind=`), with the ten
  most listened items, the streak and the books finished; milliseconds. `GET /api/v2/me/plays`
  — the plays recorded, newest first, cursor-paged. v2 revision 11.
- `GET /api/v2/family/stats` — the household's admins see their own figures and those of every
  member who shares them; the others are absent. `stats_visible_to_admin` in
  `/api/v2/me/settings` (the listener's choice, in the change feed with the settings, migration
  0131) and in `PATCH /api/v2/family/members/{user_id}` (an admin, only for an account they
  manage: `403 member_not_restricted`). v2 revision 11.
- Handler tests for the statistics routes without a token; unit tests for the range, the
  buckets and a play's words. Conformance: the `v2_stats` suite. `CAPACITY.md`: the history and
  the statistics on 2,100,000 plays.
- `GET /api/v2/search` searches every medium: `types=` takes `books` (by title or a credited
  name), `podcasts` (by title or a host), `episodes` (by title, newest first) and
  `contributors` (anyone credited: authors, narrators, hosts, artists) beside `artists`,
  `albums` and `tracks`; one request, ranked one way, only what the caller can see;
  `include=contributors` embeds the books' and shows' people too. Without `types` the answer
  is the music, as before. v2 revision 12.
- `owner=me` and `visibility=private|family` on `GET /api/v2/tracks`, `/albums`, `/books` and
  `/podcasts`: the caller's own items, the private or the shared ones, cursor-paged (v1's
  `GET /library/private` is `owner=me&visibility=private`); `400 invalid_owner`,
  `invalid_visibility`. v2 revision 12.
- Unit tests for the search's types and the filters' words. Conformance: the `v2_search` suite
  (and `v2`'s search checks know the new types);
  `organize`'s identify undo checks what it reverted (at least one change, each of the song in
  the batch) instead of a count that depended on when the release's cover arrived.
  `CAPACITY.md`: search and the filters on 600,000 tracks.
- Companion files on `/api/v2`: `POST /api/v2/files` keeps an image, booklet, lyrics or cue
  sheet from an upload (`POST /uploads` kind `companion`) at its path in the own.audio folder,
  or next to an album, book or track (`attach_to`, `relative_path`), with v1's cover and lyrics
  rules and `Idempotency-Key`; `GET`/`DELETE /api/v2/files/{id}` (into the trash, kind `file`),
  `PUT /api/v2/files/{id}/visibility`; kind `file` in `/sync/changes` and, when `kinds=` names
  it, in `/sync/snapshot` (not in the default snapshot, so players take none). A move, a share,
  the trash and a restore of a file reach the change feed (migration 0132). Downloaded through
  `POST /api/v2/playback-info` with kind `file` (always the original, reason
  `companion_file`). v2 revision 13.
- `PUT /api/v2/tracks/{id}/image`, `/books/{id}/image`, `/playlists/{id}/image` and
  `/contributors/{id}/image` take `{file_id}` instead of `{upload_id}`: one of the caller's
  companion images as the cover ("use as cover"). v2 revision 13.
- The own.audio folder on `/api/v2`: `GET`/`POST /api/v2/sync/shortcuts` and `DELETE
  /api/v2/sync/shortcuts/{id}` (an album by its `album_id`, a book, a show, or a member's whole
  `media`; kind `shortcut` in the change feed, and in the snapshot when asked for);
  `GET`/`PUT /api/v2/me/holdings` (a device's offline set, paged) and `GET
  /api/v2/me/holdings/{kind}/{id}` (the devices that hold an item); `POST /api/v2/library/organise`
  (`{kind, dry_run, ids?}`: the caller's own tracks or books to their default places, a dry run
  listing exactly the moves). v2 revision 13.
- `DELETE /api/v2/podcasts/{id}` puts an uploaded show in the trash with its episodes for 30
  days (kind `podcast` in `/api/v2/trash`, restored and purged there with its stored files)
  instead of answering `409 uploaded_podcast`; a followed show is still unfollowed. `podcast_feeds`
  is now a view over `podcast_feeds_all` that hides the trash, as books and tracks are
  (migration 0132); v1's trash routes neither list nor take a show. v2 revision 13.
- Handler tests for the new routes without a token; unit tests for the shortcut's request, the
  holdings' and trash's words, a file's path. Conformance: the `v2_folder` suite; `v2_podcasts`
  and `v2_trash` follow the uploaded show into the trash; `organize`'s identify-undo check
  compares the undo's `items` with the items it put back (the song, and its album and artists
  when the release changed them), not with the number of edits.
- Running the server on `/api/v2`: `POST /api/v2/setup` makes the first administrator while
  discovery says `setup.complete: false` (answers `Me`; `409 setup_complete` afterwards, the
  setup bucket as v1); `GET`/`POST /api/v2/admin/users` (paged in the order accounts were made,
  `q=` a part of the address or name; a new account with `Idempotency-Key`, `409 email_taken`,
  `invalid_role`), `GET`/`PATCH`/`DELETE /api/v2/admin/users/{id}` (`{display_name,
  instance_role, blocked}`; blocking it or changing its role signs it out: v1 left a demoted administrator's tokens working until they expired) and `DELETE
  /api/v2/admin/users/{id}/sessions`; `GET /api/v2/admin/families?state=active|locked|trashed`
  (paged, with `locked` and the trash figures), `GET`/`PATCH {locked}`/`DELETE
  /api/v2/admin/families/{id}` (`409 family_not_empty`); `GET /api/v2/admin/stats` (listening in
  milliseconds); `GET /api/v2/admin/jobs` (paged newest first, `kind=`, `state=`) and `GET
  /api/v2/admin/jobs/{id}`. All `instance_admin`. An administrator cannot demote, block or delete
  their own account there (`409 own_account`), nor lock the household they are in (`409
  own_family`: on a one-household server that locked everyone out, v1 still allows it). v2
  revision 14.
- `GET /api/v2/library/folders` (any member: the folders, `media: music|books`, their last scan)
  and `POST /api/v2/library/folders/scan` (household admins, `202`). v2 revision 14.
- `disc_backfill` job: the worker reads the disc number of tracks uploaded before it was kept, 20
  files a pass, one pass after the other until none are left, each pass's `{checked, left}` in
  `GET /api/v2/admin/jobs`; held while a library scan runs. On by default (no
  `WORKER_JOB_TYPES`); a deployment that lists its job types adds `disc_backfill`. v1's `POST
  /api/v1/music/tracks/discs` is unchanged. Migration 0133: indexes for the jobs list and the
  tracks still to read.
- Handler tests: the new routes refuse without a token, setup is on the setup bucket; unit tests
  for an account's fields, an administrator's own rights, the lists' words. Conformance: the
  `v2_admin` suite (the permission matrix against a member and a household admin, accounts,
  households — a second one locked and opened through SQL —, figures, jobs paged while new ones
  arrive, the disc job on a track marked through SQL); `run.py` makes the first administrator
  through `POST /api/v2/setup` on a server not set up yet, and CI leaves that to it;
  `v2_walk` pages the administrator's lists with the administrator's token.
- An edition's own kinds in `/api/v2` (the hosted edition's `translation`, a podcast episode
  translated and narrated for the household): `POST /api/v2/playback-info` takes kind
  `translation`, `/api/v2/me/progress/translation/{id}` keeps where the listener is (the later
  position wins; `kind=translation` in `GET /api/v2/me/progress`, alone or with other kinds), and
  the change feed sends `translation` and `progress.translation` (`kinds=` takes both). Where no
  edition adds the kind it stays `400 unsupported_kind` / `invalid_kind`. v2 revision 15.
- `GET /api/v2/events`: on the hosted edition, `narration` and `translation` events for the
  caller's own narration and translation jobs as they move on, so an app need not poll them. v2
  revision 15.
- `credit` in `GET /api/v2/admin/families/{id}` (the balance in whole minor units and in
  millionths with its `currency`, the last storage charge, the latest entries) and in `GET
  /api/v2/admin/stats` (every household's together), where the server charges (the hosted
  edition); absent elsewhere. A server without billing answers the hosted `POST
  /api/v2/admin/families/{id}/credit` with `501 feature_unavailable` (`billing`). v2 revision 15.
- For the hosted edition: `Hooks::edition_kinds` (`EditionKind`), `Hooks::live_events`,
  `Hooks::family_credit` and `Hooks::credit_total`; `v2::document_with` (the v2 document with an
  edition's own documented operations, as the hosted `openapi-v2-hosted.json`), `v2::to_json` and
  `v2::undeclared_permissions`; `http::router::finalize_v2`, which mounts an edition's
  documented `/api/v2` routes so personal access tokens reach them by their `x-scope`.
- Conformance: the `v2_hosted` suite, each part gated on the module the server reports — the
  credit in minor units, the administrator's credit grant (`Idempotency-Key` required, retried
  once, refused to a household admin), narration's quote and estimate, translations played,
  resumed through `/me/progress` and the apps' own route, in the change feed and on `/events`
  (a translation made through SQL with `--compose-dir`), questions paged; on a server without
  billing, the grant's `501`. `run.py --openapi-v2 <file>` walks another v2 document (the
  hosted edition's) and counts its operations; on a server where every account founds its own
  household, `v2_walk`'s member joins the admin's household first.
- Console: back from a credit top-up (`/billing?topup=success`), a thank-you dialog with the
  amount added, the new balance and, at the library's current size, the date the credit lasts
  until. It waits up to 20 s for the payment to reach the ledger, since the payment provider's
  webhook can lag the redirect. `?topup=cancelled` says nothing was charged.
- Console: `credit_topup` notifications (hosted servers) in the user's language, from their
  `data`.
- `frontend/e2e/release-smoke.mjs` — a release check of the console in a headless browser against a
  fresh server: setup, music, podcasts, an audiobook, a family invite by mail, the trash, settings
  and sign-out, with every console error and 4xx/5xx reported. `release-smoke-hosted.mjs` does
  the hosted-only pages (sign-up, credit, narration and translation up to their price).
- `family_name` (optional) in `POST /api/v1/setup/complete` and `POST /api/v2/setup`: the
  household's name. Contract revision 27, v2 revision 16. The console's setup wizard asks for
  it, filled in with the admin's surname.

### Fixed

- `DELETE /api/v2/contributors/{id}/image` forgot the picture but nothing looked for a new one
  until a v1 client asked; it now looks again in the background (an artist's photo for someone
  credited on music, an author's for someone only on books), and what it finds reaches the
  change feed with its `image_credit`.
- `PATCH` and `DELETE /api/v2/family/members/{user_id}` answered `401` for someone not in the
  household, which a client takes for a signed-out session; they answer `404`, as documented.
- `PUT /api/v1/family/members/{user_id}` with nothing to change answered `204` for someone in
  another household; it answers `401` (no such member in this household), as it already did
  when the body changed something. Nothing was ever written to another household's member.
- `conformance/tools/scale_catalog.py` adds a second batch of tracks after the first
  (the object keys of a second run collided with the first's).
- `Book.series` named every series a book is in, another member's private one included; it
  names those the reader can see.
- Deleting an account that follows a podcast failed (`500`): the gPodder subscription log
  wrote an entry for the account being deleted. Migration 0126.
- A member moving from one household to another in one step reached only the household they
  left through the change feed; the change log keeps one row per household now.
- The console fetches the music library again when it opens a copy saved on the device less
  than 30 seconds earlier; a reload right after a visit showed the old list (songs another
  family member had just shared were missing).
- The console's *Translate this episode* asks for a price only once its languages are loaded;
  before, an English show's first quote went out for English and came back 400.
- The conformance check of Subsonic bookmarks on episodes passes against an episode under 102 s
  (a 42 s bookmark there is in the last minute, so deleting it means finished) and against one
  with no position yet (the 404 was put back as a position).
- Acoustic fingerprints with an ffmpeg that has no Chromaprint (Homebrew's, for one, when the
  server runs outside its image): the server says so once in its log and fingerprints nothing,
  as with `FINGERPRINT__MODE=off`. Every upload was quarantined as "does not decode" and left
  the library. The image's ffmpeg has Chromaprint; nothing changes there.
- The first household is no longer named after its admin's whole name, which made invites read
  "Jana Novák invited you to Jana Novák": without a `family_name` it is the name's last word
  (a surname), as every invite says "the {name} family".
- The console's Trash page says restoring charges the storage only on a server with billing
  (`features.billing`).
- Storage & credit, the home page's storage card, the top-up thanks and the top-up notification
  say "More than 10 years" instead of a runway of a million days and a run-out date in the
  year 6590 (a small library on a server with billing).

### Security

- An access token of a deleted account stops working at once (`401`), on v1 and v2: the
  session check now treats a token without its session row as revoked. Before, it passed the
  check until it expired and the request failed further on (`500` on routes that read the
  household).

## [1.0.3] - 2026-11-08

Contract revision 26.

### Fixed

- Mails show the own.audio logo as the website does (the full mark with its arcs and family),
  not the old simplified one.

## [1.0.2] - 2026-11-08

Contract revision 26.

1.0.1's code; its image did not build, so 1.0.1 has no image — use this one.

### Fixed

- The image build gets Chromaprint from GitHub's tag archive and retries every source download:
  GitHub's release downloads answered 504 for hours on 2026-11-08 and failed the 1.0.1 build.

## [1.0.1] - 2026-11-08

Contract revision 26.

### Changed

- **Mails in the reader's language, with the logo.** Password reset, email confirmation, family
  invites, new-sign-in and locked-sign-in notices are written in English or Czech, picked from
  the `Accept-Language` of the request that caused them; anything else gets English. The web
  console now sends the language chosen in its settings; the iPhone apps already send the phone's. Every mail
  uses one layout with the own.audio logo, carried inside the message (no image to load, so it
  shows on a server without a public address too).

### Fixed

- The family invite mail still said "audio2"; it says own.audio.

## [1.0.0] - 2026-11-04

Contract revision 26.

The first release. The code is 1.0.0-beta.54's; what changes is the promise: from here on the
API contract only grows (see `docs/API_COMPATIBILITY.md`), migrations stay forward-only, and
the image is also tagged `1.0` and `latest`. Clients built against revision 26 — own.audio Music
1.0.0 among them — run against this server.

### Added

- **`install.sh`**: one guided setup from the command line —
  `curl -fsSL https://raw.githubusercontent.com/own-audio/server/main/install.sh | bash`.
  Checks Docker, asks for the folder, port, music and audiobook folders and the address, makes
  the secrets, writes `.env` and a `docker-compose.override.yml` with the folders mounted
  read-only, pulls the newest release, starts it, waits for `/health` and prints what to open.
  Run again to upgrade. Answers can be given as `OWN_AUDIO_*` variables with `--yes`.
- The web console's first-run setup goes on after the first account with **Your library** (the
  connected folders and how far their first scan is) and **Apps** (the server's address as a QR
  code, for the phone apps), instead of three tips.

### Fixed

- `docker compose` no longer warns about `S3_SECRET_KEY` being unset on a local-storage install.

## [1.0.0-beta.54] - 2026-11-03

Contract revision 26.

### Fixed

- The quarantine's decode, stub, cut-off and silence checks reach a library fingerprinted by an
  older server: the fingerprint job now records when it examined a file (migration 0125), and the
  sweep takes every file examined before beta.53 again, 200 every two minutes in the background.
  Without this, only files no reader could open were ever quarantined on an existing library.
- Web console: `follow-redirects` advisory GHSA-mrwp-f3q8-3g9p (a Node-only dependency of axios, never run in the browser), which had failed CI's `npm audit` since beta.51.

## [1.0.0-beta.53] - 2026-11-03

Contract revision 26.

### Added

- **Quarantine for broken files** (migration 0124). A file no reader can open, one that does not
  decode, a stub of a second or two, a download cut off early (the headers promise a song, the
  decode gives a fraction of it), or one silent for its first two minutes (a DRM-protected rip
  decodes to exact zeros) is set aside by the library scan or the fingerprint job: out of every
  list and player, as a missing file is, but kept with the reason. A family admin lists them at
  `GET /api/v2/library/quarantine` and lets one back with `DELETE /api/v2/library/quarantine/{track_id}`;
  a file that changes on disk and reads again comes back by itself. The fingerprint job measures
  loudness in the same decode (`volumedetect`), at no extra cost.

## [1.0.0-beta.52] - 2026-11-03

Contract revision 26.

### Fixed

- Transcodes (a smaller stream on mobile data, or MP3/AAC of a format a player can't decode) read
  the original where it is — the file on this server's disk, or S3 by a link — instead of through
  a pipe. An M4A whose index (`moov`) follows its audio could not be read through the pipe, so its
  transcode was an empty answer the player reported as "could not play"; and a start part-way in
  (`start_ms`, Subsonic `timeOffset`) decoded everything before it first, which an hour into an
  audiobook took longer than a player waits for the first byte. Both are seeks now. A transcode
  that produces nothing (a damaged file) falls back to the original, with ffmpeg's reason in the
  log, instead of an empty 200.
- **A library scan checks each file's name against its contents.** The headers say what a file
  is (`mp3`, `mp4`, `adts`, `flac`, `ogg`, `wav`, `aiff`, ASF for WMA…); where that contradicts
  the name — a WMA named `.mp3`, an MP3 named `.wav`, from an old rip — the content type is
  corrected to what the file is, and the file is logged, as is every file no reader can open at
  all (a stub, a damaged download). The same check runs on an upload. Files scanned before
  1.0.0-beta.51 are read once on the next scan. Test: "a file that is not what its name says gets
  the content type of what it is".
- **The length of an MP3 or WAV is measured where its headers are doubtful.** An MP3 whose
  bitrate varies and that has no Xing header was taken at its first frame's rate for the whole
  file (two 1990s rips in a test library ran a fifth short, the progress bar reaching its end
  with a minute of song left); a WAV holding MP3 was its data size over a nominal rate. Where
  ffprobe's own estimate disagrees with the headers by more than a tenth, or the WAV is not PCM,
  the scan decodes the file and counts the audio. A WAV's codec is read from its format tag, so a
  WAV holding MPEG audio (or one named `.mp3`) is transcoded rather than sent to a player as PCM.
  Tests: "a doubtful length is measured from the audio", "a WAV holding mp3 says so".
- `POST /playback-info` sends a transcode for a file whose headers say it is not what its name
  says — a WMA named `.mp3`, from an old rip — instead of the original with a content type the
  player trusts and then cannot decode. Transcodes are at most 48 kHz and stereo: a 24-bit/96 kHz
  rip or a 5.1 mix gains nothing from more in a stream made to be small, and not every player's
  AAC decoder takes 96 kHz. (Files a scan read before 1.0.0-beta.51 have no codec on record
  until their folder is scanned again.)
- `POST /playback-info` judges "over the bitrate limit" by the file's own bitrate where its
  headers were read, and only otherwise by size over length, which counts covers and tags too: a
  320 kbps MP3 with a large cover no longer gets a 320 kbps transcode of itself.

- Deleting a podcast episode's Subsonic bookmark within its last minute (or past 90 %) marks the
  episode played instead of sending it back to the start. That is how Music Assistant reports a
  finished episode — it deletes the bookmark and says nothing else — so episodes it played to the
  end showed as unplayed in the own.audio apps.

## [1.0.0-beta.51] - 2026-10-31

Contract revision 26.

### Added

- OpenSubsonic API keys: a Subsonic app can sign in with the key alone (`apiKey`), as the
  OpenSubsonic `apiKeyAuthentication` extension describes, instead of an email plus the key as a
  password; `getOpenSubsonicExtensions` lists it. A key sent together with `u`, `p`, `t` or `s`
  answers error 43, an unknown one 44. Keys get a SHA-256 for the lookup (migration 0123), filled
  in for existing keys when the server starts; the key itself stays encrypted.
- Podcast episodes are Subsonic bookmarks: `createBookmark` on an episode saves its listening
  position, `getBookmarks` lists the episodes in progress, and `deleteBookmark` forgets an
  unfinished place. They are the same positions the own.audio apps resume from, so a Subsonic
  client that resumes podcasts through bookmarks (Music Assistant does) agrees with them. Until
  now an episode's bookmark was refused as "not found".

- What a song's file is and how good: the server reads its codec, sample rate, bit depth,
  channels and bitrate when the file arrives (migration 0122) and `POST /api/v2/playback-info`
  returns them as `source`, also when it sends a conversion. Files already in a library folder are
  read on the next scan; earlier uploads have only their type and an average bitrate. The web
  console's Now Playing screen shows it: "FLAC · 24-bit / 96 kHz", "MP2 · 128 kbps · 44.1 kHz →
  playing as MP3 · 192 kbps, converted by the server".

### Fixed

- `getAlbumInfo2` answered under `albumInfo2`; the protocol (and Navidrome) name it `albumInfo`,
  as for `getAlbumInfo`. Music Assistant stopped on the missing key: its OpenSubsonic library
  synced no songs and no song would play.

### Changed

- The albums column on an artist's page in the web console can be dragged wider or narrower, like
  the list beside it; the width is remembered. Double-click the edge for the default.

## [1.0.0-beta.50] - 2026-10-30

Contract revision 26.

### Fixed

- An artist's page in the web console lists their own albums under "Albums" and compilations they
  are on under "Appears on", as the client guide asks. Mixed together, an artist the list called
  "1 album" showed five.
- A click on a song in the web console's Songs list plays it straight away. Playing queued the whole
  list and asked the server for a link to every song first: in a 10,000-song library, 10,000
  requests before anything played. Now only the song that starts gets a link; the next one is
  fetched while it plays.
- The web console plays a song Safari can't decode as an MP3 the server makes, instead of
  stopping with "this browser can't play the file's format". The case found was an MPEG Layer II
  file named `.mp3`: stored as `audio/mpeg`, so it went out as the original. A song that still
  can't be played now leaves the player paused, not showing Pause over audio that never comes.

## [1.0.0-beta.49] - 2026-10-28

Contract revision 26.

### Added

- `GET /api/v1/stats/me` says whether the family admin may see the statistics
  (`stats_visibility`: `private` or `family_admin`), so the apps can show the switch that
  `PUT /stats/me/visibility` sets (contract revision 26).

### Changed

- `POST /api/v1/auth/refresh` (and the `refresh_token` grant of `POST /api/v2/auth/token`) accepts one
  retry of a refresh whose answer was lost: presenting the token just rotated, while the token that
  rotation produced is still unused, answers with a new pair instead of signing the device out. A
  second retry, or the replaced token, still revokes the chain; a signed-out device stays signed
  out. Phones that lost signal or were suspended mid-refresh were being signed out (migration 0121,
  contract revision 26).

## [1.0.0-beta.48] - 2026-10-28

Contract revision 25.

### Added

- Identify an audiobook from Open Library, Apple's audiobook catalogue or Google Books. A family
  admin chooses under Family → Audiobook details, and may enter the family's own Google Books key
  there (stored encrypted, never shown again). New installs and existing families start on Open
  Library: keyless Google Books turns most servers away. `features.book_source_choice`,
  `GET`/`PUT /api/v1/family/settings/book-source`, `source` on every candidate (migration 0120,
  contract revision 25). The hosted edition keeps Google Books with its own key.

## [1.0.0-beta.47] - 2026-10-28

Contract revision 24.

### Fixed

- Console: "Add an audiobook" no longer offers to narrate or translate an ebook on a server without
  them (`features.narration` / `features.translation`), the open-source edition among them.

## [1.0.0-beta.46] - 2026-10-28

Contract revision 24.

### Changed

- The server image is a third of its size: 296 MB instead of 924 MB on disk (arm64), 71 MB to
  download instead of 239 MB. FFmpeg 8.1.3 with Chromaprint 1.6.2, and aubio 0.4.9's beat tracker,
  are built in `backend/Dockerfile` instead of installed from Debian, whose `ffmpeg` and
  `aubio-tools` brought LLVM, mesa, a Python with NumPy and more. Every FFmpeg decoder, demuxer,
  muxer and filter is still there (FFmpeg 8.1 instead of Debian's 5.1); gone are only formats that
  came with extra Debian libraries and no server code uses: Codec2, game-music and tracker modules,
  DASH. Fingerprints are the same as before, byte for byte. FFmpeg's security fixes now come with a
  version bump in the Dockerfile; the image scan sees only Debian's packages.
- Tempo (`bpm`): ffmpeg decodes the track to a WAV in the temp folder and aubio's `aubiotrack` finds
  the beats, averaged as `aubio tempo` averaged them. The same files give the same BPM as before.

### Fixed

- The tempo of WMA files: aubio's own decoder read only half of a WMA's audio, so its BPM was off (a
  test song measured 113 instead of 128). Songs measured before keep their stored value.

## [1.0.0-beta.45] - 2026-10-28

Contract revision 24.

### Fixed

- Uploads from the console at a server's home address. Links the server hands out — uploads and
  files on its own media route, covers, v2 stream links — were all built from `SERVER__BASE_URL`;
  with that set to a public name (a tunnel), the console opened at `http://192.168.x.x` sent its
  uploads to the public name, another origin, and the browser refused them ("The upload didn't
  finish"); over a tunnel they would also have met its request-size limit. Links now follow the
  address the request came in on when it is one of the server's own (`SERVER__BASE_URL` or
  `SERVER__ADDRESSES`); any other `Host` leaves them on `SERVER__BASE_URL` as before.

## [1.0.0-beta.44] - 2026-10-27

Contract revision 24.

### Added

- The web console takes the media proxy when the object store's host cannot be reached: a song,
  book file or episode that fails to load switches once to `fallback_url` (or the v2 link with
  `route=proxy`) at the same position, and the rest of the browser session goes straight there.
  Downloads for offline listening do the same.

## [1.0.0-beta.43] - 2026-10-27

Contract revision 24.

### Added

- A media proxy for networks that block the object store's host (company firewalls often block
  `*.r2.cloudflarestorage.com`). With `STORAGE__MEDIA_PROXY_URL` and `STORAGE__MEDIA_PROXY_SECRET`
  set, v1 stream answers carry a `fallback_url` and the v2 stream link takes `route=proxy`; both
  lead to the proxy, which checks the server's signature (HMAC-SHA256 over `GET\n{key}\n{expires}`,
  its own key) and reads the bucket. Everyone else still gets the store's presigned link, so the
  proxy carries only the exceptions.

## [1.0.0-beta.42] - 2026-10-24

Contract revision 23.

### Security

- RUSTSEC-2026-0387 (crypto-bigint): the S3 client no longer builds SigV4a signing, which only AWS
  Multi-Region Access Points need; R2, RustFS, Garage and single-region AWS sign with SigV4 as
  before. beta.41's release stopped on this advisory and has no image; use beta.42.

## [1.0.0-beta.41] - 2026-10-24

Contract revision 23.

### Added

- Acoustic fingerprints: every song gets a Chromaprint fingerprint, made by the image's ffmpeg from
  its first two minutes, in the form AcoustID takes (migration 0119, job `fingerprint_track`). New
  and changed songs are fingerprinted within minutes of the scan that finds them, a few at a time,
  never while a scan runs; ~0.3 s a song on a NAS. A song under 10 seconds or one that does not
  decode is recorded with the reason — a cut-off download is found that way. `FINGERPRINT__MODE`:
  `import` (default), `unidentified` (only songs no identify has matched, for object storage), or
  `off`. Nothing leaves the server; identify does not use them yet. On S3, ffmpeg reads only the
  first two minutes, through a link signed for the store's internal address (`STORAGE__ENDPOINT`),
  which the server can reach where a browser's address may not be.

## [1.0.0-beta.40] - 2026-10-23

Contract revision 23.

### Added

- **Merge albums** in the console: an album with others of its name by the same artist offers
  "Merge N albums" (family admins), on its page and in artist → albums → songs. They become the one
  with the most songs, with stars and ratings, and later scans keep them together
  (`POST /api/v2/albums/{id}/merge`; contract unchanged). Songs identified one by one can land on
  different editions of one release, and each edition was an album of its own: on the NAS, Dookie
  became three albums and Insomniac four.

## [1.0.0-beta.39] - 2026-10-23

Contract revision 23.

### Security

- `libexpat1` 2.5.0-1+deb12u5 in the image (CVE-2025-59375, CVE-2026-77214). The image's
  `apt-get upgrade` sat in a cached build layer and had not run since that layer was made, so
  beta.37 and beta.38 shipped the older package; CI and releases now pass the day as `APT_DAY`, and
  the system packages are at most a day behind Debian's security updates.

### Added

- The console marks a song identified in MusicBrainz with a small check badge at the end of its
  row ("Identified in MusicBrainz" as its tooltip and for screen readers).
- **Edit album** for selected songs: one album name, and the album artist if filled in, for all of
  them at once — a folder whose files spell their album three ways, or songs that belong together.
  Only your own songs change, as one batch of the edit history, undone from the confirmation. It
  uses `PATCH /api/v2/tracks`; contract unchanged.
- Identify runs cluster before they look up, as MusicBrainz Picard does: songs of one folder whose
  album names match (case, "Disc 2"/"CD1", bracketed notes aside) are one album even across album
  entities, a disc's own subfolder counts as its album's, and a song without an album tag joins
  its folder's only album. Uploads stay grouped by album entity, so two copies never merge. On the
  NAS library, 188 of 1,213 songs that were looked up alone now go with their album.
- Artist → albums → songs: an artist with more than one album gets a column of its albums ("All
  songs" first) beside the songs, chips above them on a phone. The album picked is in the address
  (`?album=`), so a link or a reload keeps it.

## [1.0.0-beta.38] - 2026-10-23

Contract revision 23.

### Security

- `tokio` 1.54.1 (RUSTSEC-2026-0386, `AsyncFd::new` and related constructors violate I/O safety)
  and `ctutils` 0.4.5 (RUSTSEC-2026-0385, non-constant-time comparisons). beta.37's CI stopped on
  them before its conformance ran; this release carries beta.37's changes as well.

## [1.0.0-beta.37] - 2026-10-23

Contract revision 23.

### Fixed

- A MusicBrainz that keeps answering "slow down" is a `503 source_busy` the caller may retry, not
  a `500`: the message the client gave up with lacked the status the error mapping looks for.
  `POST /api/v2/organize/releases` failed a conformance run on exactly that.

### Changed

- `GET /api/v1/music/tracks` carries a weak `ETag` and answers `304` while nothing the caller can
  see has changed (revision 23, `docs/API_COMPATIBILITY.md` §6b). The version is a fingerprint the
  database makes in ~30 ms from the rows' transaction stamps, instead of building and compressing
  the whole list: with 10,311 songs on the NAS, 7.5 MB and ~0.6 s.
- Answers are compressed with gzip's fastest level. The default level took ~0.35 s of the NAS's
  CPU for the 10,000-song list, longer than sending it uncompressed on a LAN.
- Conformance: a `caching` suite checks the track list's version, its 304, a new version after a
  change, and that one person's version is not another's.

## [1.0.0-beta.36] - 2026-10-21

Contract revision 22.

### Added

- The console plays concerts: a **Concerts** tab in Music (servers with `track_chapters`), the
  player naming the song of the show playing — in the bar, Now Playing and the media keys — next
  and previous moving between its songs, a list of the songs to jump to one, and for the owner
  an editor for them (rename, set a start to where the player is, add one there, delete; saved
  with `PUT /api/v2/tracks/{id}/chapters`). A concert opens where it was left, from the
  position the server keeps for every device. "This is a concert" / "Not a concert" in an
  album's menu. Chapters are read through v2. Browser check: `frontend/e2e/concerts.mjs`.

## [1.0.0-beta.35] - 2026-10-21

Contract revision 22.

### Added

- Concerts: a whole concert as one track with a chapter mark where each song starts
  (docs/concerts-plan.md). The server reads the marks from the file (M4A, M4B, MP3) when a track
  is uploaded, and in library folders for tracks of 20 minutes or more (migration 0118). A
  concert is an album whose release group is `live`: `release_kind` on `POST /api/v2/tracks`,
  `PATCH /api/v2/release-groups/{id}`, and `kind=live` on the album lists of v1 and v2.
  Chapters: `chapter_count` on v2 tracks, `chapters` on `GET /api/v2/tracks/{id}`,
  `GET /api/v1/music/tracks/{id}/chapters`; the owner corrects them with
  `PUT /api/v2/tracks/{id}/chapters`, and corrected chapters are never read from the file again.
  Discovered as `features.track_chapters`.

### Fixed

- A track uploaded through v2 (`POST /api/v2/tracks`) has its length: the server measures the
  file when the client does not say it, as it already did for library folders. Such tracks had
  no `duration_ms`, so players showed no length and could not seek or resume.

## [1.0.0-beta.34] - 2026-10-21

Contract revision 21.

### Fixed

- A book in a library folder gets the chapters marked inside its files (M4B, M4A, MP3 with chapter
  frames), each starting where its mark does in the whole book's time. A single-file book showed
  as one chapter of up to 24 hours: on the NAS test, "Atomic Habits" carries 24 marks, "The
  Complete Robot" 43. Books already indexed are read at the next scan, once (migration 0117);
  a book whose files carry fewer than two marks keeps its files as its parts. Contract unchanged
  (revision 21): `GET /audiobooks/{id}/chapters` and the v2 book's `chapters` simply fill in.
- The console no longer asks for the progress of a book you have not started: it takes it from
  the list of started books, so opening a new book is not a 404 in the browser's console.
- Conformance: the fixtures gain an M4B book with three chapter marks; the `library` suite checks
  its chapters and that a book without marks has none.

## [1.0.0-beta.33] - 2026-10-21

Contract revision 21.

### Fixed

- Library scans skip the folders a NAS keeps for itself inside shared folders (Synology's `@eaDir`,
  `#recycle`, `#snapshot`, QNAP's `@Recycle`): they hold thumbnails and index data, and part of
  `@eaDir` is unreadable, which logged a warning on every scan.
- `GET /api/v1/setup/status`, which the console asks on every page load, no longer writes, reads
  and deletes a test file each time: the storage check is kept for five minutes. On a busy NAS it
  took 1–2 s; on object storage it was three requests per page view.
- The console's player shows the length of a song the server converts on the fly (WMA): such a
  stream has no length the browser can read, so the bar showed none.
- The library scan starts as soon as setup creates the first admin, instead of at the scanner's
  next look up to 30 s later.
- Conformance: the library fixtures gain a WMA song and a Synology `@eaDir` folder; the `library`
  suite checks the WMA's tags and length and that the folder is not scanned.

### Changed

- The console opens the music pages from the device's last copy of the library and refreshes it in
  the background: with 9,351 songs the first song shows after 0.8 s instead of 2.8 s. The copy is
  kept per account in the offline database and deleted on sign-out.
- Long song lists in the console keep only the rows on screen. Scrolling all 9,351 songs held 6,400
  rows and 565 MB and blocked the browser for 5 s; now 81 rows, 25 MB and no blocking. Each row
  says its place in the list (`aria-posinset`) for screen readers; the browser's own find only
  sees the rows on screen.
- A track's cover answers with an `ETag` and `Cache-Control: private, no-cache`: the browser keeps
  it and asks again, and an unchanged cover is a `304` that never reads the file. Contract
  unchanged (revision 21).

## [1.0.0-beta.32] - 2026-10-21

Contract revision 21.

### Security

- `rustls-webpki` 0.103.16 and `webpki-roots` 1.0.10 (RUSTSEC-2026-0382, RUSTSEC-2026-0381): TLS
  certificate checks could accept distrusted intermediate certificates. The server makes TLS
  requests to MusicBrainz, podcast feeds, cover art and sign-in providers. `webpki-roots` 0.26.11,
  which sqlx 0.8 still pulls in, only re-exports version 1 and is listed as an exception.

## [1.0.0-beta.31] - 2026-10-21

Contract revision 21.

### Fixed

- A cover embedded in the audio files is stored once per picture, not once per track. Every track
  of an album carries the same art, so the per-track copies were mostly duplicates: 1,702 stored
  covers for 379 different pictures in the first 2,000 tracks of a NAS library
  (`docs/SYNOLOGY.md`). Covers already stored stay as they are.
- File checksums wait while a library folder is being scanned. Each one reads the whole file, so
  during a first scan every file was read twice from the same disks; they run once the scan is
  done.
- The web console plays every format in the library: it tells the server what the browser plays
  (`POST /api/v2/playback-info`) and gets an MP3 for the rest. WMA did not play in any browser,
  and neither did FLAC in older Safari. Contract unchanged (revision 21).
- Applying an identified album no longer takes about 4 seconds a song. Each song asked
  MusicBrainz for the album's release again and downloaded its cover again; releases are now kept
  for a quarter of an hour, and the cover is stored once per release and reused by the album's
  other songs. Against musicbrainz.org that leaves one request (about a second) a song.
- WMA files in a library folder get their tags and length: the tag reader cannot open WMA, so
  they were added with a title made from the file name and no length. When it cannot open a file,
  `ffprobe` reads it instead. Files added before are read again when they change.
- The console tells albums with the same name apart: it selects, highlights and lists an album by
  its id. A folder tagged partly with MusicBrainz release ids becomes two albums (open, see
  `docs/library-scale-plan.md`), and the console showed both as selected with their songs together.
- Console pages opened directly (`/music`, `/settings`, …) answer with status 200 instead of 404.
- The console shows your profile picture: in Settings, with a preview of a newly chosen one, and on
  the account button in the sidebar. It was saved before, but shown only on the Family page, so
  uploading one looked as if nothing had happened.
- The console stays light with a large library. With 9,351 songs the music pages held 340,000 DOM
  nodes and 390 MB of memory and froze the browser for seconds: the song list and the "all songs"
  column rendered every song at once. Long lists now render a part at a time as you scroll (28 MB,
  13,000 nodes), and the filter no longer redoes the whole list on every keystroke.
- Covers in the console load when they come into view, a few at a time, once per address. Scrolling
  a large library asked for every cover together: the browser refused 20,000 requests and the
  server's rate limit 440 more.
- Settings no longer downloads the whole library (6.6 MB) to show muted songs when there are none.
- A selected album or song is marked for screen readers (`aria-current`), not by colour alone.
- Members no longer get an error on the identify page (library folders are for family admins), and
  the trash page no longer asks a server without billing for prices.
- `frontend/e2e/library-scale.mjs`: the console in a browser against a large library — load times,
  bytes, long tasks, memory, playback, accessibility.
- Conformance: the identify searches that ask MusicBrainz wait out a `503 source_busy` (after 3, 6,
  12 and 24 s, as the answer advises) and record a skip if it stays busy, instead of failing the
  run. Without a private metadata service the server asks the public musicbrainz.org, which takes
  about one request a second per address, and CI runs two conformance jobs from one runner — the
  beta.30 run failed on exactly that. Any other status still fails.

## [1.0.0-beta.30] - 2026-10-18

### Changed

- The console keeps your questions about an episode and their answers (hosted, for you alone), with
  a button to clear them. Contract unchanged (revision 21).

## [1.0.0-beta.29] - 2026-10-18

### Changed

- The console's summaries come in three lengths (short, medium, long), and each summary and answer
  shows what it cost. Questions can be asked without a summary first. Hosted only; contract unchanged
  (revision 21).

## [1.0.0-beta.28] - 2026-10-18

### Added

- The hosted summaries on `/api/v2/summaries/…` too (contract revision 21): a server without them
  answers those paths and `/api/v1/podcast-ai/…` with `501 feature_unavailable`. The console calls
  the v2 paths, and its error messages read v2's `detail` as well as v1's `error`.

## [1.0.0-beta.27] - 2026-10-18

### Added

- `features.summaries` (contract revision 20), `false` in this edition: the hosted edition's
  episode summaries and questions. The console shows "Summarize" with a question box under an
  episode with a transcript when a server reports it.

### Changed

- An expanded episode in the console shows its transcript button (and Summarize) above the
  description, and long descriptions are cut to four lines with "More".

## [1.0.0-beta.26] - 2026-10-18

### Added

- An episode's transcript to read (contract revision 19):
  `GET /api/v1/podcasts/{id}/episodes/{ep_id}/transcript` answers the publisher's
  `<podcast:transcript>` or the file uploaded with the episode as timed lines (WebVTT, SRT,
  Podcast Index JSON) or plain text; YouTube-style rolling captions read once. The console shows it
  under an expanded episode ("Show transcript").

## [1.0.0-beta.25] - 2026-10-18

### Added

- Uploaded podcasts (contract revision 18, `features.uploaded_podcasts`): a podcast whose
  episodes its owner uploads instead of a feed the server fetches — lectures, a family's own
  recordings, a show bought as files. `POST /api/v2/podcasts` and
  `POST /api/v2/podcasts/{id}/episodes` take the files from `POST /api/v2/uploads` (new kinds
  `episode`, `episode_transcript`, `podcast_cover`, `episode_image`); a `guid` makes both safe to
  retry. The show is shared with the family or private like any other, its episodes are stored
  from the start, play, sync and notify as usual, and an uploaded transcript (WebVTT, SRT, text,
  JSON) is served through `transcript_url`. Migration 0116 (`transcript_object_id`).

## [1.0.0-beta.24] - 2026-10-18

### Removed

- YouTube channels as podcast feeds (contract revision 17). The server no longer lists or
  downloads anything from YouTube, and the image no longer contains `yt-dlp` (or the
  Python it pulled in). `POST /podcasts/subscribe` with a YouTube address answers
  `400 youtube_not_supported`. YouTube feeds followed earlier stay, with their episodes and
  stored copies, but are never refreshed again; storing one of their episodes, auto-store
  and store-all answer `409 youtube_not_supported`. This skips the six-month deprecation in
  `docs/API_COMPATIBILITY.md` §5 on purpose: the feature never worked in a released image
  (the bundled `yt-dlp` 2023.03.04 could no longer read YouTube), so no client depends on it.

## [1.0.0-beta.23] - 2026-10-17

### Added

- gPodder sync (contract revision 16, `features.gpodder`): podcast apps that sync with a
  gPodder server — AntennaPod, Kasts, gPodder, own.audio Podcasts — sign in with your email and
  the app key from *Settings → Other apps*, and keep your shows and positions in step with
  this server and the own.audio apps. gpodder.net's API v2 at `/api/2` (`docs/gpodder.md`).

### Changed

- *Settings → Other music apps* is *Other apps*: the same key signs in gPodder podcast apps.
- The organizer is called *Identify music* in the console, opens on the songs not yet
  identified, and the sidebar lights only its own entry while it is open, not *Music* as well.

## [1.0.0-beta.22] - 2026-10-15

### Added

- The organizer's table plays a song: a play button beside each row's checkbox starts that
  song on its own, and on the song playing it pauses and resumes. Hearing two "possible
  duplicates" is how you tell them apart. A click anywhere on a row selects it (shift for a
  range), and the checkboxes wear the console's accent.

### Fixed

- The organizer's table shows the cover an identify brought: the server fetches it in the
  background after answering, and the table looked only once, too early; now it looks again a
  few seconds later.
- The organizer's facets and the duplicates filter answer in milliseconds again: beta.21's
  duplicates test compared every song with every other, over a second on 600 songs; it is now
  three hash joins over the caller's songs.

### Security

- rustls 0.23.46: RUSTSEC-2026-0346 to 0349 (session values' `Debug` output, ECH rejection,
  Brotli certificate decompression, key-exchange downgrade). CI's advisory check had been
  failing on them since beta.17.

## [1.0.0-beta.21] - 2026-10-15

### Fixed

- The organizer's *Possible duplicates* filter. It took any two songs with the same title and
  artist for a pair, so a studio take beside a live one was a "duplicate", while two copies
  whose titles differed in an apostrophe or a comma were not. Now a pair is the same file, the
  same MusicBrainz recording, or the same title and artist (spelling and punctuation aside)
  within three seconds of the same length and not identified as two different recordings; the
  list comes pair by pair instead of album by album, and the filter shows its count like the
  others (`counts.duplicates` in `GET /api/v2/organize/facets`).

## [1.0.0-beta.20] - 2026-10-14

### Changed

- Fewer round trips to object storage, which on a remote store (R2, S3) are most of a
  request's time: a resized cover is read in one request instead of a check and a read
  (`Storage::get_optional`), the covers served lately are kept in memory (32 MB, the oldest
  go first; an image never changes, so nothing kept goes stale), and `playback-info` takes a
  file's size from the database rather than asking the store, where the upload recorded it.

### Fixed

- Conformance suite: the organizer suite skips on a server with none of the admin's songs
  instead of failing; with the fixture library, the mobile suite checks the album its songs
  went to, and the organizer accepts the release's cover arriving in the undone batch.

## [1.0.0-beta.19] - 2026-10-14

### Changed
- **An identify brings the release's cover.** Accepting a run's proposal (one, an album's, every
  sure one, a picked recording) and `PUT /api/v2/tracks/{id}/identity` put the release's cover on
  the songs, in the background, replacing what they had when a cover is found; one fetch per
  release, kept under the release so an undo shows the earlier cover again, and in the
  identify's batch of the history. A compilation's song kept on its own album keeps its cover.
  `PUT …/identity` takes `fetch_cover: false` to skip it. Until now only the web console's single
  identify fetched a cover, and only when asked.
- Covers fall back to the release group's front on Cover Art Archive when the release has none of
  its own, which many reissues and digital editions do not.
- The console shows each match's cover — proposals, editions, recordings and releases, in Identify
  and in the organizer.

## [1.0.0-beta.18] - 2026-10-14

### Added
- The organizer's artist card: with an album artist picked and no song selected, its name,
  sort name and MusicBrainz artist (by link) to correct, kept in the history; and merging it
  into another spelling of the same artist (a family admin's, not undoable).

## [1.0.0-beta.17] - 2026-10-14

### Fixed
- 1.0.0-beta.16's image did not build (the console's type check); the same release, built.

## [1.0.0-beta.16] - 2026-10-14

### Added
- **The metadata organizer** (revision 15, docs/metadata-organizer-plan.md), in the web
  console under Create → Organize music, `features.organizer`: your songs as a table to filter
  (not identified, no cover, no year, no genre, changed, possible duplicates, by folder, album
  artist, album, words) and sort, edited in place or many at once, saved as one batch;
  MusicBrainz beside it — a song searched in your own words and compared field by field, or a
  set of songs matched to a release with which file is which track, applied in one batch with
  the cover and, if wanted, the genre. Files are never rewritten.
- **An edit history with undo** (`metadata_edits`): every change someone makes to a song, an
  album or a contributor — by hand, in bulk, by identify, by a run, by an undo — with the
  fields before and after. `GET /api/v2/edits`, `GET /api/v2/edits/batches`,
  `POST /api/v2/edits/{id}/undo`, `POST /api/v2/edits/batches/{id}/undo`: all or nothing,
  `409 changed_since` when a field changed again since. Scans reading files are not edits.
- `GET /api/v2/organize/tracks`, `GET /api/v2/organize/facets`, `PATCH /api/v2/tracks` (many
  songs, one batch), `POST /api/v2/organize/releases`, `POST /api/v2/organize/apply-release`.
- `PATCH /api/v2/contributors/{id}` takes `musicbrainz_artist` (an id or link; `409 mbid_taken`
  when another contributor already is that artist — merge them instead).

### Changed
- MusicBrainz asking to slow down answers `503 source_busy` with `Retry-After`, not a 500.

## [1.0.0-beta.15] - 2026-10-14

### Added
- An album's editions searched again with the owner's words (revision 14), for an album its
  tags name wrong: `POST /api/v2/identify/groups/{id}/search` with `album` and/or `artist`
  replaces the group's editions and puts its pending proposals on the best one; an owner's own
  picks stay. The web console's "Other edition" has the fields.

## [1.0.0-beta.14] - 2026-10-13

### Added
- Search in the owner's own words (revision 13), for tags that are what is wrong — "Track 02",
  a typo, the artist in the title. A run keeps what each single-track lookup also found:
  `GET /api/v2/identify/proposals/{id}/alternatives`; `POST …/{id}/search` searches again with
  a title, artist or album and keeps the results; `PUT …/{id}/recording` makes a recording the
  proposal, by id or musicbrainz.org link, looked up by id, and accepts it with `accept`.
  Proposals say `chosen_by_owner` and how many `alternatives` they have; another edition of
  the album leaves an owner's choice alone.
- Any of your tracks, outside a run: `POST /api/v2/tracks/{id}/identity/search` and
  `PUT /api/v2/tracks/{id}/identity`; `DELETE` undoes it as before. Free in every edition.
- The web console's identify page: "Other recording" on a proposal, with the alternatives, a
  search and a pasted link.

## [1.0.0-beta.13] - 2026-10-13

### Added
- Smart playlists on `/api/v2` (docs/api-v2-plan.md, V11): `/smart-playlists` to list, read,
  save, change (name, description, rule, sharing) and delete them; `/smart-playlists/presets`;
  `POST /smart-playlists/resolve` to try a rule without saving it and
  `POST /smart-playlists/{id}/resolve` to play one, each worked out from the caller's own
  history; `POST /smart-playlists/{id}/freeze` to make one into an ordinary playlist. The rule
  is v1's. The read-only demo account may try and play them.

## [1.0.0-beta.12] - 2026-10-12

### Added
- `GET /api/v2/events` also sends `identify` events (revision 12): a run of the
  caller's moved on — `run_id`, `status`, `groups_done`, `groups_total`. Clients can
  stop polling a run.
- `DELETE /api/v2/tracks/{id}/identity` (revision 12): undo one track's identify, as
  v1's `…/metadata/revert` does; `409 not_identified` / `nothing_recorded`.

### Changed
- Undoing a track's identify, by either API, makes its run's proposal pending again.
- Identify against musicbrainz.org finds the album a library holds, not a box set
  or a bootleg of it: song searches ask for official studio albums first, the
  releases to look up are chosen by kind, track count and year, and when the best
  match is a remaster box the original release of the same recordings is looked
  up too. A song whose compilation is not in MusicBrainz is searched without it.
- Web console: keys on a run's page — j / k next and previous, a accept, x reject,
  Shift A the whole album.

## [1.0.0-beta.11] - 2026-10-12

### Added
- Web console: Music → Identify library. Start a run for the whole library or
  one folder, accept the sure matches at once, review the rest song by song or
  album by album, pick another edition, undo a run. Songs matched to another
  artist are left out of "accept album".
- `POST /api/v2/identify/quote` (revision 11): how many tracks a run over a scope
  would look up and the most it could charge — 0 here, where identify is free.
  Starting a run can now answer `402` in an edition that charges for it.
- Edition hooks for identify: `identify_quote`, `identify_run_start` (may refuse a
  run) and `identify_run_end` (called when a run finishes, fails or is deleted,
  while its proposals still exist).

### Removed
- `MUSICBRAINZ__BASE_URL`, `MUSICBRAINZ__API_KEY` and `MUSICBRAINZ__MIN_INTERVAL_MS`
  (beta.9): identify asks only the public MusicBrainz API, at its own pace. A
  server that still sets them starts as before; they are ignored.

### Fixed
- The read-only demo account could not write anything through the REST API, not
  even what listening writes (progress, playback sessions): the check saw the path
  without its `/api/v1` prefix and allowed nothing. On `/api/v2` it now allows the
  same — `POST /playback-info` (a read), progress, plays and `/sync/push`, where
  the demo account's favourites and ratings are refused one by one.

## [1.0.0-beta.10] - 2026-10-11

### Added
- What an app needs from `/api/v2` to stop using the Subsonic API for music
  (docs/api-v2-plan.md, V10):
  - `/playlists`: list, read, make, rename, describe, share, delete (to the trash),
    and their entries — each with its own id, as a track may be in a playlist
    twice — added, removed and reordered. A reader sees, and counts, only the
    entries whose tracks they can see. Not in the change feed yet.
  - `GET /search`: artists, albums and tracks in one answer; a track matches by
    its title, its album or a credited name.
  - `GET /tracks/random`, from one genre if asked.
  - `GET /me/played`: the caller's own most played and most recently played
    tracks or albums, with their play counts.
  - `GET /tracks/{id}/lyrics`: line by line, with times when the lyrics have them.

### Fixed
- A play reported on v2 with an `ended_reason` other than `completed`,
  `skipped`, `stopped` or `replaced` failed to be recorded; the reason is now
  dropped instead (the documentation suggested `finished`).

## [1.0.0-beta.9] - 2026-10-11

### Added
- Identify runs (`features.identify_library`, contract revision 10; endpoints in the
  draft `/api/v2/identify/*`): match your library, a library folder, an album, a
  playlist, an artist or a selection against the MusicBrainz source in the background
  (migration 0112). Results are proposals — `sure`, `review` with the reasons
  (part of an album, different disc layout, two files on one track, another album
  matching as well, a compilation's song on its original album, a weak match) or
  `none` — accepted all at once for the sure ones, one by one for the rest, on
  another edition when you pick one (no new lookup), or keeping a compilation's own
  album. A run can be reverted as one. Runs take turns across households, one album
  per lookup, and wait with a growing delay when the source is busy; against
  musicbrainz.org that is about one request a second, so a large library takes hours.
  Nothing is written to files.
- Identify can ask any server that speaks the MusicBrainz API instead of
  musicbrainz.org: `MUSICBRAINZ__BASE_URL` (your own MusicBrainz mirror, or a
  `music-metadata` instance), `MUSICBRAINZ__API_KEY` for one that wants a key, and
  `MUSICBRAINZ__MIN_INTERVAL_MS` to lift the one-request-a-second pace for your own
  server. musicbrainz.org itself keeps its pace and is never sent a key. The compose
  file passes them through (`MUSICBRAINZ_BASE_URL`, `MUSICBRAINZ_API_KEY`,
  `MUSICBRAINZ_MIN_INTERVAL_MS`, `MUSICBRAINZ_CONTACT`, `MUSICBRAINZ_ENABLED` in `.env`).
- Album identify counts a release named like the album as one more hit, so the album
  itself is not outranked by every live record holding the searched songs.

### Added
- Files that carry their own MusicBrainz ids (as MusicBrainz Picard saves them: Vorbis
  `MUSICBRAINZ_TRACKID`, ID3 `UFID`, MP4 `MusicBrainz Track Id`) arrive identified, with no
  lookup, on upload, folder scan and rescan; the ids follow the file when it changes.
- `POST /api/v1/music/tracks/{id}/metadata/revert` undoes an identify: the track's title,
  artist, album, album artist, genre, track number and MusicBrainz ids go back to what they
  were before its first identify (migration 0111 keeps them). Tracks identified earlier stay
  identified but cannot be reverted: their previous values were never kept. Contract
  revision 9.

### Changed
- `…/metadata/rescan` and the folder scan refuse to re-read the file only for a track this
  server matched through MusicBrainz, no longer for one whose ids came from the file itself.

## [1.0.0-beta.8] - 2026-10-11

### Fixed
- `GET /api/v2/sync/snapshot` was cut after its first line for a client that
  accepts gzip (every app): the stream panicked when the compression layer
  polled it after its end.

## [1.0.0-beta.7] - 2026-10-11

### Added
- `/api/v2`, in draft beside v1 (`docs/api-v2-plan.md`): its own OpenAPI
  document (`docs/api/openapi-v2.json`), errors as RFC 9457 problem details
  with a stable `code`, cursor-paged lists, every operation declaring who may
  call it (`x-permission`, checked by the conformance suite), `GET /me`. Draft
  means it may still change in any way until our apps ship on it.
- v2 sign-in in the OAuth shape: `POST /api/v2/auth/token` for every grant
  (password, refresh, device code, a provider's id token, a two-factor code;
  `403 mfa_required` starts the last), `POST /auth/revoke` (RFC 7009),
  `POST /auth/device` (RFC 8628, with `verification_uri_complete` for a QR
  code), sessions under `/me/sessions` (signing a device out ends its access
  tokens at once), and personal access tokens under `/me/tokens`: scoped,
  shown once, stored hashed, good on `/api/v2` only (migration 0105).
- The music catalogue on v2: `/contributors` (with `/artists`, `/authors`,
  `/narrators` and `/hosts` as views of them), `/albums`, `/release-groups`,
  `/recordings`, `/genres`, `/tracks`. Records carry ids and credits, not
  names; `include=` sends referenced records once; lists page by cursor in
  index order (migration 0106), about 30 ms a page at 600,000 tracks.
- Books and podcasts on v2: `/books` (one with its files, chapters, series
  and people), `/podcasts`, `/podcasts/{id}/episodes`, `/episodes/{id}`
  (migration 0107). Credits list by role, then position.
- Corrections on v2: `PUT /tracks|books|podcasts|albums/{id}/credits` sets
  an item's credits by hand — a role set this way stays whatever the tags
  later say, which is how two people of one name are told apart; `PATCH
  /contributors/{id}` and `/albums/{id}`, merge and split of contributors,
  merge and regrouping of albums. Merging and splitting answer `403` before
  reading the body (v1's too).
- A listener's own state on v2, one model for every medium: `/me/favourites`,
  `/me/ratings`, `/me/progress` (a book's position counts across its files;
  the later position wins, a device clock ahead of the server's is clamped,
  and each device's last position is kept — migration 0108), `/me/plays`
  (idempotent by the client's id), `/me/bookmarks`, and `POST /sync/push`
  for what a device changed offline, answered change by change. Stored in
  the tables v1 writes, so both versions see the same state.
- Sync on v2: `GET /sync/snapshot` streams everything the caller can see as
  NDJSON between a `begin` and an `end` line carrying the cursor (resumable
  with `after`), and `GET /sync/changes` reads a change log fed by triggers
  (migration 0109, kept 180 days): each changed item's current record or its
  removal, the listener's own state included, one row per item per
  transaction. A household move sends `reset`; an expired cursor is `410`.
- Playback on v2: `POST /playback-info` takes what the client plays and
  answers `direct` or `transcode` (MP3 or AAC, by the Subsonic layer's
  planner and ffmpeg pool) with a link signed for one item, format, bitrate,
  user and key id, good for six hours. `GET /stream` checks the user may
  still play it, then redirects to storage for the original or streams the
  transcode. No access token in any URL.
- Artwork on v2: records carry `image`, a link to `GET /images/{id}` signed
  for the image and the week, not the user, so lists stay cacheable; the
  client adds `w` (64 to 1024), the server scales once and keeps the result
  (`derived/…`, which the storage reconcile leaves alone), answered
  `immutable`. Replaces `has_cover`/`has_image` in the draft records.
- The household on v2: `/family` (and renaming it), `/family/members` (roles
  `admin` / `member`, rights, age bracket, label; removing), `/family/invites`,
  `/family/join`, `/family/storage`, and `PUT /tracks|books|podcasts/{id}/visibility`
  with `private` or `family`. The rules are v1's, called rather than copied.
- `GET /api/v2/events` (Server-Sent Events): a `library` event whenever the
  caller's household or own state changes, its id the change log's position
  (`Last-Event-ID` resumes), a comment every 30 seconds; the records come
  from `/sync/changes`.
- Adding to the library on v2: `POST /uploads` (the size is required, so a
  file that cannot fit is refused before a byte is sent — security hardening
  plan C1), then `POST /tracks` or `POST /books` from the uploads.
  `Idempotency-Key` on every v2 write: a repeat answers what the first did
  (kept 24 hours, migration 0110), another request under the same key is
  `422`. Discovery states the upload, batch, idempotency and change-log limits.
- `http::router::finalize_with` lets an edition add routes under `/api/v2`,
  where they get v2's problem details, rate limit and `Idempotency-Key`. A
  server without the hosted edition answers its v2 paths (`/family/billing`,
  `/narration`, `/translation`) with `501 feature_unavailable`, as on v1.
- `GET /.well-known/own-audio`: the server's id, version, APIs, sign-in
  methods, modules, addresses and limits, without signing in.
- Music entities: contributors, credits, release groups, albums, recordings
  and genres, per household, following MusicBrainz (migration 0100,
  `docs/data-model-plan.md`). The server derives them from the tags and keeps
  them in step on every write; the tag strings on a track stay as read.
  Artists are split on `;` and on the guest markers (`feat.`, `ft.`,
  `featuring`) — never on `&` or `,`, which belong to names like "Simon &
  Garfunkel". Tracks in `/api/v1/music` carry `album_id` and `recording_id`;
  tracks on one album share its id however the tag is spelled. Linking an
  existing library of 600,000 tracks takes about a minute at the first start.
  Contract revision 8.
- `GET /music/artists` and `GET /music/albums` read the entities: each artist
  and album carries `id`, an album its `artist_id`, and albums filter by
  `artist_id` as well as by name. Artists are whoever is credited, so "A; B"
  lists two and a `feat.` guest is not an artist of the track; albums spelled
  in different case are one album.
- Corrections on artists and albums, fixed once on the entity and kept when
  the same tags are read again (migration 0104):
  - `PATCH /music/artists/{id}` renames; the old spelling still matches, and
    a name another artist already has is a 409 (merge them instead).
  - `POST /music/artists/{id}/merge` makes two artists one; their albums of
    the same title become one too.
  - `POST /music/artists/{id}/split` turns "Hall & Oates" into its two
    people: every credit, and every later scan of the same tags, credits both.
  - `PATCH /music/albums/{id}` renames, `POST /music/albums/{id}/merge` joins
    one release tagged two ways, `PUT /music/albums/{id}/release-group` groups
    editions; albums carry `release_group_id`.
  Merging and splitting are a family admin's.

### Changed
- Subsonic artist and album ids are the entities' ids (migration 0101). They
  change once, with this release: a Subsonic client re-syncs its library, and
  album and artist stars, ratings and cached ids from before are carried over
  by the server, not by the client. Albums are grouped by album artist, so a
  compilation is one album instead of one per track artist, and `byGenre`
  finds an album through any of a track's genres ("Rock; Pop").
- Stars, ratings and photos of artists and albums belong to the entities:
  per household, kept across retags and spelling. `GET/POST/DELETE
  /music/artists/image*` take `artist_id` as well as `artist` (the name).
- Books credit the same contributors as music (migration 0102): authors and
  narrators come from a book's `author` / `narrator` text (`;` or ` & `
  between names — "Terry Pratchett & Neil Gaiman" is two authors) and links
  made by hand are kept. Someone who narrates and sings is one contributor.
  `audiobook_authors` is gone; `/audiobooks/authors` lists, edits and links
  the household's contributors, with photos shared with the music side.
  Roles are `author`, `co_author`, `narrator`, `translator`, `editor`,
  `illustrator`.
  Renaming an author to a name another contributor has is a 409, as for artists.
- Podcasts credit their hosts the same way, from the feed's author line
  (migration 0103); a refresh that brings the same author changes nothing.

### Security
- `/audiobooks/authors` was one list for the whole server: every household
  saw every other's authors and tags, could edit or delete any author by id,
  and could link authors to or set tags on any book by id. Authors and tags
  are per household now; reading a book's credits or tags needs the book to
  be visible, changing them needs its owner, and deleting a contributor
  needs a family admin.
  Entities nothing refers to any more are removed by the daily sweep.

## [1.0.0-beta.6] - 2026-10-11

Contract revision 7. Beta.5 with its version number set: the beta.5 release
did not bump it, so its image reports `1.0.0-beta.4` in `GET /api/v1/server`
and the compose file still pulled beta.4. Nothing else changed.

### Fixed
- `docker-compose.yml`, `.env.example` and `INSTALL.md` name this image, and
  the server reports its own version.

## [1.0.0-beta.5] - 2026-10-11

Contract revision 7. The security release: phases 1–3 of the hardening plan,
with the apps' side still to come (a client must handle the two-factor
`202`, see `docs/API_COMPATIBILITY.md` §7, before anyone turns it on).

### Security
- Access tokens live one hour by default, not seven days (`AUTH__ACCESS_TTL_SECS`);
  apps refresh as before, and a stolen token is worth an hour.
- Browsers may call `/api` only from the console's origins
  (`SERVER__CORS_ORIGINS`, default: the console's and the server's own); the
  server used to allow any origin. `/rest` stays open for Subsonic web clients.
- A signed-in caller without the right gets `403` on admin routes and on
  other users' records, not `401`; clients no longer refresh a good token and
  sign out over a permission.
- Requests the server makes on a user's behalf (podcast feeds, episode
  audio, feed artwork) go through one guard: `http` and `https` only, public
  addresses only (checked after DNS and on every redirect, the connection
  pinned to the checked addresses), bodies under a size cap and a stall
  timeout. A feed on a private or local address is refused with `400`.
  `SERVER__OUTBOUND__ALLOW_PRIVATE=true` lifts the address check for a server
  whose feeds live on its own network.
- Episode audio is streamed to disk under `PODCASTS__MAX_EPISODE_BYTES`
  (default 512 MiB) instead of being read into memory whatever its size.
- Presigned uploads and storing podcast episodes need the `can_upload` right,
  like every other upload; the Subsonic `downloadPodcastEpisode` too. An
  uploaded object over the size limit is removed at `complete` instead of
  registered. A presigned upload that never completes is deleted by the daily
  storage sweep after 24 hours (migration 0094).
- `POST /auth/register` is rate-limited like sign-in.
- A new password must be at least 12 characters (was 8); nothing else is
  required of it, and existing passwords keep working. Setup, register,
  invite claim, password change and admin-create all apply it; the console's
  hints say so.
- Wrong passwords in a row lock sign-in for that email for a growing while
  (30 s after five, doubling to 16 minutes), whether or not the email has an
  account: `429` with `error: account_locked` and `Retry-After`. The owner
  is told by mail at the first lock when mail is configured. A right
  password clears it. Migration 0095. The published demo account is exempt.
- Email addresses are verified. An account born from an invite mailed to
  that address, from a sign-in provider that vouches for it, or typed in by
  an admin counts as proven; one from open registration (or a link invite)
  gets a confirmation link, good for 24 hours, and `me.email_verified` is
  false until it is used (`POST /auth/email/verify`, `POST /auth/email/resend`;
  the console shows a banner). Inviting people needs a proven address
  (`403 email_unverified`). A server without mail or a console address takes
  addresses as given — nothing to verify against. Everyone who exists at
  upgrade is taken as verified (migration 0097). The edition seam gains
  `Hooks::email_verified`, where the hosted edition grants its welcome credit
  from now on, so a stream of fresh registrations earns nothing.
- CI scans the server image for fixed critical and high CVEs, audits the
  console's shipped dependencies, and fails if a password or a bearer token
  appears in the server log of a full conformance run.
- A security trail per account (`GET /auth/security-events`, the last 100):
  sign-ins and failures, locks, two-factor steps and changes, password
  changes and resets, email confirmation, sessions signed out or revoked,
  accounts created, changed or deleted by an admin — with the address and
  user agent, never a secret. Kept a year; the daily sweep drops older rows.
  The console's Settings page shows it as "Recent activity". Migration 0099.
- Browser-facing headers on every response: a `Content-Security-Policy` for
  the console (its own files and the Google and Apple sign-in scripts, no
  inline scripts, no framing; media and images from wherever this install
  keeps them; fetches to this origin, the store and the providers),
  `X-Content-Type-Options: nosniff`, `X-Frame-Options: DENY`,
  `Referrer-Policy: strict-origin-when-cross-origin`, a `Permissions-Policy`
  that leaves the camera for the console's QR scanner, and
  `Strict-Transport-Security` when `SERVER__BASE_URL` is https.
- Request limits on the whole API: 1200 a minute per signed-in device (by
  token) or address over `/api/v1` and `/rest`, 60 for searches, 30 for the
  routes that make the server fetch from elsewhere (subscribe, refresh,
  identify, metadata lookups), and the provider sign-ins under the sign-in
  limit (`SERVER__RATE_LIMIT__API_PER_MINUTE`, `…SEARCH…`, `…OUTBOUND…`).
  `429` with `Retry-After`, as before.
- Two-factor sign-in with an authenticator app (TOTP, RFC 6238) and eight
  one-time recovery codes, per user, off by default: `GET /auth/totp`,
  `POST /auth/totp/setup`, `POST /auth/totp/enable`, `DELETE /auth/totp`
  (needs a code). With it on, `POST /auth/login` and the provider sign-ins
  answer `202` with an `mfa_token` instead of tokens, and
  `POST /auth/totp/verify` with the code finishes the sign-in. A code works
  once; the secret is encrypted at rest. The console's Settings page sets it
  up (QR code) and the sign-in page asks for the code. `features.auth.totp`.
  Migration 0098.
- A sign-in from a device unlike any the account used in the last 90 days
  (by kind and name) is announced to the owner by mail, when mail is
  configured; the demo account is exempt.
- Subsonic API keys are encrypted in the database (AES-256-GCM under a key
  derived from `AUTH__SESSION_SECRET`); the protocol needs them readable, so
  they could not be hashed like passwords. Existing keys are encrypted at the
  first start. Changing the secret now also means regenerating Subsonic keys.
- Instance-admin routes decide on the caller before reading the body
  (`InstanceAdmin` extractor): a plain member gets `403` with nothing else,
  where `POST /auth/admin-create-user` used to answer a validation error
  first, and `GET /jobs` answered `401`. The conformance suite now walks
  `docs/api/openapi.json` (`suites/access.py`): every protected operation
  must answer `401` without a token and every admin one `403` to a member.
- A per-family storage limit, `STORAGE__FAMILY_QUOTA_BYTES` (unset: none).
  Over it, `POST /uploads/presign`, `POST /uploads/complete` and storing a
  podcast episode answer `402` with the reason; the Subsonic
  `downloadPodcastEpisode` reports it in its error message. The same check is
  the seam an edition uses for its own rule (`Hooks::storage_allowance`).

### Added
- "Forgot your password?" — `POST /auth/password/forgot` mails a single-use
  link, good for 30 minutes, when the email has an account with a password
  (the answer is `200` whatever the email); `POST /auth/password/reset` sets
  the new password behind it and signs every device out. Offered as
  `features.auth.password_reset` when mail and `SERVER__APP_BASE_URL` are
  set; the console shows the link only then. Migration 0096. Contract
  revision 7.
- `docker-compose.yml` passes `APPLE_SIGN_IN`/`APPLE_CLIENT_IDS` and
  `GOOGLE_SIGN_IN`/`GOOGLE_CLIENT_IDS` through, so a self-hosted server can let
  own.audio Music sign in with Apple or Google (INSTALL.md lists the ids).

### Changed
- Library folders: a file that disappears from a folder hides its song (or,
  when none of its files are left, its book) everywhere, the Subsonic API
  included, instead of leaving an item that fails to play. When the file is
  back, the same item returns with its stars, playlists and history.
  Migration 0093.

### Fixed
- Web console: a page could fail to open with "'text/html' is not a valid
  JavaScript MIME type" until the next deploy. A host that answers missing
  paths with the app's page (Cloudflare Pages, any SPA fallback) returned HTML
  for a build file asked for while a deploy was switching over, and the
  service worker cached it under that file's name. It now keeps only real
  files and drops such a copy; its cache is renewed when its own code changes.
  It also asks past the browser's HTTP cache, where a CDN that gives `.js`
  paths hours of cache kept the fallback page too, and the worker's own
  update check no longer goes through that cache.
- Web console sign-in: the "Continue with Apple" button had no visible label
  (its text took the background colour), and in the dark theme Google's
  button sat in a white box. Both are now the same height and shape.
- A first scan of a large library folder got slower with every file: the
  check that a new item's file-sync path is free read every path the owner
  had (40 ms a file at 44,000 files). It is three index lookups now.

## [1.0.0-beta.4] - 2026-10-10

Contract revision 6.

### Changed
- A public demo's shared account (`SERVER__DEMO__EMAIL`) is read-only: its
  password is published, so anyone could otherwise change it, rotate the
  Subsonic key or delete the music and lock everyone else out. Listening,
  progress, the play queue, scrobbles, searches and smart-playlist previews
  still work; other writes answer `403` (Subsonic: error 50).
  `SERVER__DEMO__READ_ONLY=false` lifts it while a seed script runs.

### Fixed
- Subsonic: songs shared from another family member's library carried album
  and artist ids made for their owner, not for the listener, so "go to album"
  and "go to artist" failed on them in every Subsonic app.

## [1.0.0-beta.3] - 2026-10-10

Contract revision 6.

### Added
- `GET /api/v1/server` says which server this is and where it can be
  reached: `id`, a UUID made once at install, and `addresses`, every address
  the server answers on with its `scope` (`lan`, `vpn`, `public`). The list is
  `SERVER__BASE_URL` plus the new `SERVER__ADDRESSES` (comma-separated: the
  home network, a Tailscale address, a second public name). A client can then
  switch between them as the phone moves between networks. Migration 0092.
- Subsonic `stream` makes a smaller stream on request: `maxBitRate` (kbps)
  and/or `format` (`mp3`, `aac`; `raw` is the original), with `timeOffset` to
  start part-way. The original is piped through ffmpeg and sent as it is
  made, nothing written to disk. A limit the original already meets sends the
  original; so does every case when ffmpeg is busy — at most
  `SUBSONIC__MAX_TRANSCODES` (default 4) run at once. `download` always sends
  the original.

## [1.0.0-beta.2] - 2026-10-08

Contract revision 5.

### Added
- Identify through the public MusicBrainz API when no metadata service is
  set: track candidates, which album a group of tracks is, and the details
  applied, with the metadata service's album matching. Asked only when
  someone presses identify, at most once a second, with a User-Agent naming
  the server (`MUSICBRAINZ__CONTACT`); `MUSICBRAINZ__ENABLED=false` turns it
  off. `features.music_identify` is now `true` on a default install.
  Podcast discovery still needs the metadata service.
- Podcast search without the metadata service, through Apple's public iTunes
  Search API: the term goes to Apple, so `ITUNES__ENABLED=false` turns it
  off. `features.podcast_search` says whether search works (contract
  revision 5); categories, browse and similar shows stay with
  `features.podcast_discovery`.
  Apple lists one show several times; each feed is answered once.

### Fixed
- Subsonic `getStarred`: the starred tracks in one query instead of one
  each, starred albums and artists through the grouping index instead of two
  passes over the catalogue.
- A smart playlist's tracks and a playlist's tracks are read in one query,
  not one per track. `getRandomSongs` shuffles ids only and reads whole rows
  for the few it picks.
- Storage reconcile pages through the store's listing and checks each page
  against the database, instead of holding every key of both in memory.
- The web console offers podcast categories and similar shows only when the
  server has them, and search only when it can answer; before, a server
  without the metadata service showed them and answered with errors.

## [1.0.0-beta.1] - 2026-10-08

Contract revision 4. The first beta: the API contract is executable
(`docs/api/openapi.json`) and checked against every later release.

### Added
- Mail over SMTP submission (`SMTP_HOST`, `SMTP_USER`, `SMTP_PASSWORD`,
  `MAIL_FROM` in `.env`; `MAIL__SMTP_*` underneath): implicit TLS on 465 by
  default, STARTTLS on 587, or a password-less relay on your own network.
  No password is sent over a connection that is not encrypted.

### Removed
- The JMAP mail sender (`MAIL__JMAP_*`). Mail goes over SMTP only; an
  install that set the JMAP variables sends no mail until `MAIL__SMTP_*`
  (or `SMTP_*` in `.env`) is set.
- `features.one_family` in `GET /api/v1/server` (revision 4).
- `docs/api/openapi.json`: the whole `/api/v1` contract as OpenAPI 3.1, 271
  operations, generated from the handlers (`utoipa`). Routes are registered
  through `utoipa-axum`'s `routes!`, so a documented route is a real one.
  CI fails when the file is not what the code describes, when an operation
  disappeared since the last release, or when it changed without a revision
  bump (`scripts/check-api-contract.py`). It replaces the hand-written
  endpoint spec in `docs/mobile-backend-api-spec.md`.
- `SECURITY.md`, `CONTRIBUTING.md` and `docs/EDITIONS.md`.
- Nightly database backups: a `backup` service in the compose file dumps
  PostgreSQL at 03:00 UTC into `./backups`, keeping 7 daily and 4 weekly
  dumps (`BACKUP_DIR`, `BACKUP_HOUR`). It uses the database's own image, so
  the dump tool always matches it. `BACKUP.md` shows the restore, and CI
  restores a dump into an emptied database and runs the suite against it.
- `scripts/pg-upgrade.sh 17` moves the compose stack to a new PostgreSQL
  major: dump, the new major on a fresh data directory beside the old one,
  restore, every table's row count compared, and only then the server
  started. The old data stays until you remove it. Tried from 16 to 17 with
  the suite passing afterwards. `UPGRADING.md` explains when it is needed.

### Changed
- The server refuses to start with an empty, placeholder (`CHANGE_ME`) or
  shorter than 16 characters `SESSION_SECRET`, which signs every sign-in and
  media link; under 32 characters it logs a warning.
- `features.mail` is `true` only when a mail server is actually set, not
  when the variables exist but are blank.
- One install is one family. The first admin founds it; every later account
  (invite, admin-created, open registration, single sign-on) joins it as a
  member instead of getting a family of its own, and removing a member or
  leaving answers `409` instead of creating a second family. Installs that
  already have several families keep them. The hosted edition is unchanged:
  the `Hooks::one_family` default is `false`, and only the open-source
  binary answers `true`.
- The server no longer runs as root. The image starts as root only to hand
  `/data/media` to `PUID:PGID` (default 1000:1000, once, if its owner
  differs), then runs the server as that user. Library folders must be
  readable by it.

### Fixed
- Subsonic album and artist ids are found by a key read (`subsonic_ids`,
  migration 0091) instead of aggregating the whole catalog per request:
  `getAlbum`, `getArtist`, `getMusicDirectory`, `getCoverArt` for album and
  artist tiles, stars and ratings, album and artist info. An id the server
  has not handed out yet costs one pass, which records every id at once;
  one pass at a time, so an album grid's fifty covers do not start fifty.
  `getCoverArt` tries playlist and podcast ids first, which never needed it.
- `GET /library/changes` (a full sync is every track) and
  `GET /sync/tree/ids` stream their lists instead of building them in
  memory; same responses.
- Reading a track's tags, its lyrics and the audio analysis stream the file
  to disk instead of holding it whole (a FLAC is 100–300 MB).
- The checksum and audio-analysis backfills enqueue the next batch as soon as
  the previous one has drained, instead of 200 items an hour.
- Web console on iPhone and iPad: the volume slider did nothing, because
  Safari there ignores a page's volume (only the hardware buttons change it).
  The slider is hidden where the browser cannot set volume, and mute now uses
  the element's `muted`, which works everywhere.

## [1.0.0-alpha.7] - 2026-10-07

Contract revision 3 (unchanged).

### Added
- `conformance/tools/loadtest.py`: N listeners browsing, streaming with
  ranges and reporting progress at once; `--heavy` adds the catalog-wide
  calls on every loop.

### Changed
- Media files stream in 64 KiB reads instead of 4 KiB, a sixteenth of the
  trips through Tokio's blocking pool.
- `conformance/tools/loadtest.py` streams from the address under test (media
  links carry the public address) and shares one sign-in across listeners.

### Fixed
- `GET /music/tracks` (streamed since 1.0.0-alpha.6) broke for every client
  that asked for gzip — browsers included: the stream panicked when the
  compression layer polled it after its end, so the response arrived empty.
  The stream is fused now, and the conformance suite fetches the main lists
  with `Accept-Encoding: gzip`.
- Claiming a file-sync path compared the new path with every path the owner
  already had (no index could serve the "inside another folder" test), so a
  first library scan slowed down with every file. The test is now three index
  lookups (migration 0090 adds a prefix index); measured on a Raspberry Pi 4
  during a 10,000-file scan.

## [1.0.0-alpha.6] - 2026-10-07

Contract revision 3.

### Added
- `docs/RASPBERRY_PI.md`: the server on a Raspberry Pi 3 or newer (64-bit
  OS, a USB disk, library folders, local storage, a Cloudflare Tunnel).
- `conformance/tools/scale_catalog.py`: fills a test database with a
  generated catalog (600,000 tracks and 1,000 books in about half a minute)
  and measures memory and latency of the browse calls. Results in
  `docs/CAPACITY.md`.
- Library folders: a file whose tags change is read again on the next scan;
  a moved or renamed file (same size and time, nothing left at the old path)
  keeps its item, so progress, stars and playlists stay.
- Conformance suite `library` (scan, tags, natural chapter order, streaming
  from a folder, removing hides) on generated fixtures
  (`conformance/tools/make_library_fixtures.sh`,
  `conformance/compose.library.yml`); CI runs it in the local-storage job.
- API contract revision 3: tracks and books carry `source` (`upload` or
  `folder`) and `read_only`; `GET /library/folders` and
  `POST /library/folders/scan`; stream URLs may point at the server's own
  `/api/v1/media` route. The conformance suite checks them.
- **Read-only library folders** (issue #1): `LIBRARY__MUSIC`,
  `LIBRARY__AUDIOBOOKS` or `LIBRARY__FOLDERS` point the server at existing
  collections, mounted read-only and indexed in place. Music by tags (and a
  `cover.jpg` beside the files), audiobooks as `Author/Title/` folders with
  chapters in natural order and titles from the tags. The scanner walks one
  directory at a time and skips files whose size and time are unchanged; it
  runs at start, hourly, from `POST /api/v1/library/folders/scan` and from
  Subsonic `startScan`. Removing a folder item hides it and leaves the file.
  `features.library_folders` is `true` when folders are configured.
- **Local storage** (`STORAGE__KIND=local`, the compose default): media is
  plain files under `/data/media`, no object store needed. RustFS moved behind
  the compose profile `s3`.
- **The server's own media route**, `GET | HEAD | PUT /api/v1/media`, with
  links signed by the server (key, method, expiry; 4 hours), range requests
  for seeking, and bodies streamed in chunks. It stands in for presigned URLs
  with local storage, and with S3 when `STORAGE__PROXY=true` — for networks
  whose firewall blocks the store's host. Clients see the same opaque,
  expiring URLs either way and need no change.
- CI runs the conformance suite against both storage kinds.
- Plan: everything ships together as 1.1.0; a Raspberry Pi 3 demo with local
  storage only, published through a Cloudflare Tunnel.
- Plan, Phase 6 item 4, and scope decision 14: PostgreSQL stays the only
  database, and before 1.0 the server makes it painless — family-sized
  settings by default, nightly automatic backups with a tested restore, and
  a one-command PostgreSQL major upgrade.
- `docs/RAM_USAGE.md`: why Navidrome and Audiobookshelf cannot be measured on
  PostgreSQL (both are SQLite-only, checked in their repositories).

### Changed
- The visibility check in list queries is written inline instead of calling
  `audio2_can_access` per row: the policy is read once per query and grants
  as one set. At 600,000 tracks a family's album list takes 0.2 s in
  PostgreSQL instead of 1.6–1.9 s, genres 0.1 s instead of 1.1 s. Same rules,
  checked by the families conformance suite.
- Album artist, primary artist and album are stored as generated columns
  (migration 0089) with indexes, instead of a regular expression evaluated per
  row per query. The migration rewrites `music_tracks_all` once, which takes
  a few minutes on a very large catalog.
- `GET /music/tracks` streams the list from the database instead of
  building it in memory: at 600,000 tracks the server peaks at 23 MiB instead
  of 805 MiB. Same JSON array, no client change.
- The image sets `MALLOC_ARENA_MAX=2`, `MALLOC_MMAP_THRESHOLD_=131072` and
  `MALLOC_TRIM_THRESHOLD_=131072`: the server's peak under the test suite
  falls from 272 MiB to 27 MiB at the same speed.
- The compose file runs PostgreSQL sized for one family (`shared_buffers`
  32 MB, 30 connections) and pins its major version. Server and database
  together: 41 MiB idle, 111 MiB at peak.
- The server's database pool is 10 connections by default and configurable
  with `DATABASE_MAX_CONNECTIONS` (it was a fixed 20). A busy deployment —
  the hosted edition — should set it explicitly.

### Fixed
- The manual tag rescan of a track streamed the file into memory whole; it
  now streams it to a temporary file.

## [1.0.0-alpha.5] - 2026-10-07

Contract revision 2 (unchanged).

### Added
- `docs/CAPACITY.md`: who the server is for — one family, with no limit on
  its members,
  catalogs up to 600,000 songs and 1,000 audiobooks, a smaller footprint
  than Navidrome and Audiobookshelf — and an audit of the places where
  today's code still grows with the catalog (tracked in issue #2).
- `docs/RAM_USAGE.md`: the memory investigation. The server needs about
  20 MiB at rest; the 270 MiB seen after a burst is glibc keeping freed
  memory, and three `MALLOC_*` settings bring the peak to 28 MiB. Includes a
  comparison with Navidrome and Audiobookshelf, what the settings cost, and
  how to compare a PostgreSQL stack with SQLite servers fairly (whole stack
  against both of them together).
- Scope decisions 11–13 (one family per install, catalog size, footprint
  goal) and a "Scale and footprint" section in the implementation plan.
- README: who the server is for.
- GitHub issues #1 (read-only library folders) and #2 (scale to a 600k-song
  catalog).
- `brand/github-avatar-512.png` and `brand/github-social-preview.png` for
  the GitHub organisation and the repository's link preview.
- A Docker Hub overview (`DOCKERHUB.md`), published by its own workflow
  whenever it changes; OCI labels on the image and annotations on the multi-arch
  index, so GHCR and Docker Hub show a description, the source and the licence.

### Changed
- The web console loads each page on demand instead of as one 1.2 MB
  script: the sign-in screen needs three small files, and the build no longer
  warns about oversized chunks. A tab left open across a deploy reloads once
  if a page it asks for is gone.
- `npm audit fix` in the console: axios, react-router, vite, postcss and
  their dependencies moved to patched versions within their majors; the audit
  reports nothing now.
- CI runs clippy on tests too (`--all-targets`); the two lints it found in a
  test module are fixed.
- A `.dockerignore` keeps `target/`, `node_modules/` and local `.env` files
  out of the image build context; local builds no longer copy a Mac's
  `node_modules` over the Linux ones.
- CI and release workflows use the Node 24 majors of every action
  (checkout v7, setup-node v7, upload-artifact v7, download-artifact v8,
  Docker's login v4, setup-buildx v4, build-push v7, metadata v6), which
  clears GitHub's Node 20 deprecation warnings. A cancelled conformance run no
  longer warns about a missing report.
- README and INSTALL: both architectures are pulled, the demo's content is
  described as it is now, and features still being built (library folders,
  local storage, SMTP mail, music identify, podcast discovery) are marked as
  coming instead of listed as available. The native apps are described as in
  development.

### Fixed
- The web console saved audiobook progress as a position in the whole book,
  while the API (and the native apps) store the position inside the playing
  file. A book started in the browser resumed too far in on a phone and showed
  more than 100 % on the shelf; one started on a phone resumed at the start of
  its file in the browser. The console now saves and resumes the in-file
  position, and still reads positions it saved the old way.
- Streaming a podcast episode that is not downloaded yet answers
  `409 episode_not_downloaded` instead of a `500` that logged an internal
  error.
- The console's player said "the link expired, or the file isn't on the
  server" when the file was fine but the browser cannot play its format —
  Safari and Ogg Vorbis podcasts. It now checks whether the file loads and
  says the format is the problem.
- The release workflow pasted the image metadata into a shell string; once
  the repository description contained an apostrophe, `v1.0.0-alpha.4` built
  both images but created no tag. The metadata now goes through the
  environment, and the step fails when it tags nothing.

## [1.0.0-alpha.4] - 2026-10-07

Contract revision 2 (unchanged).

### Fixed
- Smart playlists now see real listening. The worker rebuilds the listening
  rollup and the per-track play counts every hour over the last year; before,
  nothing scheduled that job, so "not played lately", "what the family plays"
  and the play-weighted shuffle never reflected what anyone listened to.

## [1.0.0-alpha.3] - 2026-10-07

Contract revision 2 (unchanged).

### Fixed
- The web console's account menu and a few dialogs and sign-in errors stayed in English when the console was set to Czech; they are translated now.

## [1.0.0-alpha.2] - 2026-10-07

Contract revision 2.

### Added
- `GET /api/v1/server` carries an optional `demo` object (`email`, `password`)
  when `SERVER__DEMO__EMAIL` and `SERVER__DEMO__PASSWORD` are set. The
  console's sign-in screen then shows a "This is a demo" box with the shared
  account and a button that fills it in. The values exist only in the
  server's environment, never in the image or the console bundle.
- The sign-in screen shows the server's version under the form.
- The release workflow builds `linux/arm64` natively next to `linux/amd64`
  and publishes one multi-arch tag to GHCR and Docker Hub
  (`kornelko2/own-audio-server`, from the `DOCKERHUB_IMAGE` repository
  variable). It can be re-run for an existing tag (`workflow_dispatch`).
  Attestation manifests are off, so the package shows exactly two platforms.
- A public demo of this edition at https://demo.own.audio (guest account in
  the README), reset nightly.

### Changed
- The repository is public (2026-10-06); the compose stack defaults to the
  published `ghcr.io/own-audio/server` image.
- CI spends fewer minutes: the conformance job reuses Docker layer cache
  between runs, documentation-only pushes skip the heavy jobs, and a newer
  push cancels the run it supersedes.

### Removed
- The inherited `install.sh` (it set up the old Garage stack under the old
  name); `docker compose up -d` with `.env` is the install path, see `INSTALL.md`.

## [1.0.0-alpha.1] - 2026-10-06

Contract revision 1. The first build of the open-source server; not for
production use yet — the install guide, the SMTP mailer and library folders
are still to come (see `docs/IMPLEMENTATION_PLAN.md`).

### Added
- The server, imported from the private codebase behind own.audio as a
  snapshot (no history) at its version 0.1.56 — audiobooks, podcasts, music,
  families with parental controls, playback sync, the own.audio folder sync
  protocol, an OpenSubsonic surface, and the web console. Every source file
  carries an SPDX header; PT Serif ships with its OFL text
  (`THIRD_PARTY_NOTICES.md`).
- A compose stack for self-hosters: PostgreSQL 16, RustFS 1.0.1 as the
  object store, the server built from source; `.env.example`, `INSTALL.md`.
- CI: check, `clippy -D warnings`, tests, `cargo deny` licence allow-list,
  console lint/test/build with an npm licence check, gitleaks, and the
  conformance suite against the compose stack; a release workflow that
  publishes `ownaudio/server` (amd64 + arm64) on a `v*` tag.
- `rust-toolchain.toml` pins Rust 1.97 for CI, local and the image alike;
  CI installs that exact version. The release workflow publishes to GHCR
  (and Docker Hub once its token exists), amd64 only for now.
- `GET /api/v1/server`: edition, version, API revision and the `features`
  map every client gates on. Contract revision **1**.
- `GET /api/v1/family/storage`: a family's bytes per media kind.
- Per-IP rate limits on login, refresh, device codes, join codes and setup
  (`SERVER__RATE_LIMIT__*`), answering `429 rate_limited` with `Retry-After`.
- `501 feature_unavailable` for routes of features this edition does not
  offer (billing, narration, translation).
- Conformance suite (`conformance/`, 330 checks) runnable against any server,
  with `--compose-dir`, `--database` and `--database-user` for the SQL-backed
  checks; billing-only expectations are gated on `features.billing`.

### Changed
- Clippy debt from the private codebase paid at import (`-D warnings` is
  clean); `backend/.env.example` uses obvious placeholders for the S3 keys.

### Security
- `quick-xml` 0.36 → 0.41 (RUSTSEC-2026-0194, -0195: quadratic duplicate-attribute
  check and unbounded namespace allocation in the Subsonic XML envelope).
- The AWS SDK no longer pulls its legacy TLS client: `hyper` 0.14 / `h2` 0.3
  (RUSTSEC-2026-0258) and `rustls` 0.21 / `rustls-webpki` 0.101
  (RUSTSEC-2026-0098, -0099, -0104) are gone from the dependency graph;
  `aws-config` and `aws-sdk-s3` use the SDK's modern HTTPS client.
- `cargo deny check advisories` runs in CI; two unmaintained transitive
  crates (`paste`, `ttf-parser`) are accepted with reasons in `deny.toml`.
- `Cargo.lock` is committed, so a build is reproducible and audits mean
  something.

### Removed (relative to the hosted codebase)
- Billing, payments, the narration and translation pipelines, the operator
  console — hosted-only, see `docs/SCOPE.md`.
