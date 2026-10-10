# Changelog

All notable changes to the own.audio server. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
semver. Each release states the **API contract revision** it serves
(`GET /api/v1/server` → `api.revision`), see `docs/API_COMPATIBILITY.md`.

## [Unreleased]

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
