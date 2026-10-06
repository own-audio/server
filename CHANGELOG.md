# Changelog

All notable changes to the own.audio server. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow
semver. Each release states the **API contract revision** it serves
(`GET /api/v1/server` → `api.revision`), see `docs/API_COMPATIBILITY.md`.

## [Unreleased]

### Added
- The release workflow builds `linux/arm64` natively next to `linux/amd64`
  and publishes one multi-arch tag; it pushes to Docker Hub (`ownaudio/server`)
  as soon as the account's token is configured, and can be re-run for an
  existing tag (`workflow_dispatch`).

### Changed
- The repository is public (2026-10-06); the compose stack defaults to the
  published `ghcr.io/own-audio/server:1.0.0-alpha.1` image (amd64), with
  `docker compose build server` for arm64.

### Removed
- The inherited `install.sh` (it set up the old Garage stack under the old
  name); `docker compose up -d` with `.env` is the install path, see `INSTALL.md`.

### Changed
- CI spends fewer minutes: the conformance job reuses Docker layer cache
  between runs, documentation-only pushes skip the heavy jobs, and a newer
  push cancels the run it supersedes.

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
