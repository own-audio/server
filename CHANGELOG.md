# Changelog

All notable changes to the own.audio server. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
semver. Each release states the **API contract revision** it serves
(`GET /api/v1/server` → `api.revision`), see `docs/API_COMPATIBILITY.md`.

## [Unreleased]

### Added
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
- The image sets `MALLOC_ARENA_MAX=2`, `MALLOC_MMAP_THRESHOLD_=131072` and
  `MALLOC_TRIM_THRESHOLD_=131072`: the server's peak under the test suite
  falls from 272 MiB to 27 MiB at the same speed.
- The compose file runs PostgreSQL sized for one family (`shared_buffers`
  32 MB, 30 connections) and pins its major version. Server and database
  together: 41 MiB idle, 111 MiB at peak.
- The server's database pool is 10 connections by default and configurable
  with `DATABASE_MAX_CONNECTIONS` (it was a fixed 20). A busy deployment —
  the hosted edition — should set it explicitly.

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
