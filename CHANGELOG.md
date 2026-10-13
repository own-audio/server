# Changelog

All notable changes to the own.audio server. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
semver. Each release states the **API contract revision** it serves
(`GET /api/v1/server` → `api.revision`), see `docs/API_COMPATIBILITY.md`.

## [Unreleased]

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
