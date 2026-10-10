# CLAUDE.md — own-audio-foss

The agent contract for this repository. Read before changing code.

---

## 1. What this repo is

The **open-source own.audio server**: one Rust/Axum binary over Postgres and
S3-compatible storage that serves audiobooks, podcasts and music to a family,
with the React web console embedded. Licensed **AGPL-3.0-or-later**. **Public
repository since 2026-10-06**: everything here is written for strangers to
read, build and run, and GitHub Actions minutes are free here — which is why
the heavy CI (the conformance stack) stays on GitHub.

It is the **upstream** of the hosted service at own.audio. The hosted edition
lives in the private `audio2` repository as a small binary crate that
**depends on this crate** and adds billing, payments, the AI narration and
translation pipelines, the operator console and the production deployment.
There is one copy of the core, and it is this one.

State right now: **the core is imported** (2026-10-06, a snapshot of the
private codebase at its 0.1.56 with the hosted parts already split out — no
history, by design). It builds, its 202 unit tests pass, `clippy -D warnings`
is clean, `cargo deny` accepts every dependency licence, and the conformance
suite passes against the compose stack. Not yet tagged.
`docs/IMPLEMENTATION_PLAN.md` says what happens next and in what order;
`docs/SCOPE.md` says what is in and out; `docs/API_COMPATIBILITY.md` is the
versioning policy every client relies on. Update their status tables as work
lands.

---

## 2. The audio2 repository family

Ten repositories, one product. Know which one you are in.

| Repo | What it is | Stack |
|---|---|---|
| **`own-audio-foss`** (this one) | The open-source server + web console. **The API contract.** | Rust, TypeScript |
| `audio2` | The hosted edition: depends on this crate; billing, AI pipelines, operations. Private. | Rust, TypeScript |
| `audio2-sync` | Shared Rust sync core (the own.audio folder); UniFFI for the Mac Finder extension | Rust |
| `music-metadata` | Lookup service over a MusicBrainz mirror. Open source, separate deployable; optional for this server. | Rust |
| `audio2-mac` | Lead client; home of the shared Swift packages | Swift |
| `audio2-ios-book`, `-ios-podcast`, `-ios-music`, `audio2-tvos` | Apple clients, shared packages by submodule from `audio2-mac` | Swift |
| `audio2-android-book`, `-android-podcast`, `-android-music` | Android clients, `core/network` copied per repo | Kotlin |
| `audio2-win` | Windows client | C# / WinUI 3 |
| `audio2-www` | Marketing site, roadmap, waitlist. Tracks cross-repo progress. | Astro |

What is shared, and how:

- **The API contract is shared as a generated document and an executable
  suite**: `docs/api/openapi.json` (from `utoipa` annotations in this repo)
  and `conformance/`. Until Phase 3 lands, the prose client guide in `docs/`
  is authoritative, as it was in `audio2`.
- **Code is shared with the hosted edition only, as a Cargo git dependency
  pinned to a tag of this repo.** No submodule, no copy, no "port later".
- **Nothing here is shared as code with any client.** Swift is shared by
  submodule among the Apple repos; Kotlin by copy; design tokens by hand.
  Deliberate.
- **Cross-repo progress is tracked in `audio2-www`**, not here. When work
  here finishes a roadmap item, say so in your summary; don't edit that repo
  from this one.

---

## 3. The rule that matters most: the contract is additive, and it is one contract

Clients cannot see this source. Nine of them, on four platforms, run months-old
builds against today's server — and against two editions of it.

1. **Within `/api/v1`, change only additively.** New endpoint, new optional
   field, new `features` key, new enum value where the contract says unknown
   values are tolerated. Never remove, rename, retype, or change a status
   code a client handles. The full rules, the deprecation process (six
   months minimum) and the "contract revision" number are in
   `docs/API_COMPATIBILITY.md`. If what you need is not additive, it goes
   on the `/api/v2` list in that file, not into v1.
2. **A contract change and its documentation land in the same commit**: the
   handler, the OpenAPI annotations, the revision bump, the `CHANGELOG.md`
   line, the client guide where prose is needed. CI fails on a stale
   `openapi.json`.
3. **Optional features are discovered, never assumed.** Every feature that an
   operator can switch off has a key in `GET /api/v1/server` → `features`,
   which is `true` only when fully configured. A request for a feature that
   is off answers `501 { "error": "feature_unavailable", "feature": "…" }` —
   never a fake success, never a 404 (which means "missing or not visible to
   you", deliberately, and must keep meaning that).
4. **The hosted edition reaches the core through five extension points and
   nothing else**: `http::router::api_routes()` + `finalize()` (routes), the
   `Hooks` trait in `AppState` (money and dashboard extras), the job
   registry, its own `HostedConfig`, its own migrator in the `hosted`
   schema. Adding a sixth is a design discussion, not a quick fix. Never
   add a branch on "edition" inside the core.
5. **When the contract document and the code disagree, the code is right
   and the document is a bug.** Fix the document and say so in the commit.

---

## 4. Scope — what must never be in this repository

Full list in `docs/SCOPE.md`. The short version, because it is also a
licensing boundary:

- **No money.** No credit ledger logic, Stripe, storage charges, top-ups.
  The three billing tables exist in the migration sequence (continuity with
  the hosted database) and stay empty here; `grep -ri stripe backend/src`
  must return nothing.
- **No AI pipelines.** Narration and podcast translation are hosted-only
  (decided 2026-10-06, reversible later). Their tables stay dormant in the
  core sequence; their console pages stay here, hidden when
  `features.narration` / `features.translation` are false.
- **No own.audio operations.** No tunnel names, VPS addresses, R2 bucket
  names, canary workflows, or the operator console. `metadata.own.audio` is
  not a default anywhere — there is no shared metadata mirror.
- **No copyrighted test material.** `audio2/books-data/` (an e-book text
  used for narration tests) never comes here; test fixtures must be public
  domain or generated.
- **No secrets, ever, including in history.** This repo has a fresh history
  for exactly this reason (a Google Cloud key was once committed to
  `audio2`); the code is imported as a snapshot, never with `audio2`'s
  commits. `gitleaks` runs in CI; run it locally before the first commit of
  any imported tree. `.env` is gitignored and stays so.
- **No hosted-only code copied here "temporarily".** If the hosted edition
  needs something the core lacks, add an extension point (§3.4) in the core
  and the feature in `audio2`.

Things that are **in** and optional (off until configured): SSO with Google,
Apple and Microsoft; SMTP mail notifications; book identify through Google
Books; Podcast Index with a free key. Music identify through the public
MusicBrainz API is on by default. Config-gated, reported in `features`.

---

## 5. Layout

```
backend/src/
  app/        config, state, bootstrap, run      http/       router, handlers, /server
  auth/       login, JWT, refresh, SSO, device   db/         all SQL, one module per domain
  users/ families/ setup/ dashboard.rs            jobs/       worker loop + handler registry
  library/    search, delta sync, private         storage/    S3 presigning, family keys
  audiobooks/ podcasts/ music/ youtube/           playback/   progress, queue, bookmarks
  metadata/   music-metadata client, Google Books, cover-art cascade, Wikimedia
  subsonic/   OpenSubsonic at /rest               filesync/   the own.audio folder protocol
  library/scan/  (Phase 4) read-only library folders   media/  (Phase 4) server-served streams
  trash/ uploads/ stats/ devices/ mail/           hooks.rs    the Hooks trait + NoopHooks
backend/migrations/   numbered, forward-only, all 85 imported verbatim (three
                      billing tables and the generation tables stay dormant here)
backend/assets/fonts/ PT Serif for the cover watermark + its OFL.txt
frontend/             React console (served from ui/dist by the binary); hosted-only
                      pages hide themselves when GET /api/v1/server says so
i18n/                 the one catalog of UI strings (en + cs)
conformance/          black-box API suite, takes --base-url; tools/seed_test_family.py
docs/                 contract (android-client-guide, mobile-backend-api-spec),
                      policy, plan, scope, LICENSING; the shipped-feature plans
Cargo.toml            a one-member workspace so backend/Dockerfile's shape matches
                      the hosted repo's; deny.toml; .gitleaks.toml
docker-compose.yml    PostgreSQL + RustFS + server, for self-hosters (.env.example)
.github/workflows/    ci.yml (check, clippy -D warnings, test, deny, console, gitleaks,
                      conformance against the compose stack), release.yml (image on tag)
```

**All SQL lives in `db/`.** Domain modules call into it; they don't embed
queries.

---

## 6. Gotchas inherited from `audio2` (all still true after import)

- **`sqlx::migrate!()` embeds migrations at compile time.** A new migration
  needs a rebuild (`docker compose build`), not a restart.
- **`device_kind` is validated in four places that must agree**: the
  allowlists in `auth::issue_tokens` and `playback::normalize_device_kind`,
  plus `CHECK` constraints on `refresh_tokens` (0017) and
  `listening_sessions` (0020). Relaxing only the Rust side turns a
  collapsed-to-`other` value into a constraint violation and a 500 on login.
- **404 means "missing *or* not visible to you."** Never distinguish them.
- **Presigned URLs are signed against `STORAGE__PUBLIC_ENDPOINT`**, not
  `STORAGE__ENDPOINT`. In Docker both must be set; a wrong public endpoint
  fails only at playback time.
- **Uploads run with `DefaultBodyLimit::disable()`.** The ceiling is the
  reverse proxy's.
- **Audiobook upload order is decided by `relative_path`**, not send order.
- **`audiobook_books`, `music_tracks`, `music_playlists` are views** over
  `…_all` tables (the 30-day trash). A migration that adds a column must
  alter `…_all` and recreate the view; only `db::trash`, the purge job and
  reference checks may name `…_all`.
- **A new job type must be in `WORKER_JOB_TYPES` wherever it is set**
  (`docker-compose.yml`), or no container claims it.
- **PostgreSQL only, 16 and up; dev runs 17.** SQLite was considered and
  rejected (`docs/SCOPE.md` decision 10) — don't start a "just make this
  query portable" effort; there is nothing for it to lead to. Before
  shipping a migration, apply the whole sequence to a throwaway `postgres:16`
  (recipe in `docs/UPGRADING.md` once written; until then, the one in
  `audio2`'s CLAUDE.md §6).
- **Never recompute an object key to read an object**; read
  `media_objects.object_key` *and* `media_objects.backend`. Objects live in
  S3, in the local data dir, or in a read-only library folder; the layout
  changed once already.
- **The server never writes inside a library folder.** Read-only handles,
  `:ro` mounts, and the sweeps (`storage_sweep`, `trash_purge`,
  `find_unreferenced`) skip `backend = 'folder'`. A test guards this; keep
  it green. (Phase 4 — see the plan.)
- **The AWS SDK is pulled in without its legacy `rustls` feature** (hyper
  0.14 + rustls 0.21, both with open advisories). Cargo unifies features
  across the graph, so any crate in the build that depends on `aws-config`
  or `aws-sdk-s3` with default features silently brings the old stack back —
  the hosted edition's crate included. `cargo deny check advisories` is the
  guard; keep it green.
- **Subsonic**: use `subsonic::extract::SubsonicQuery`, never axum's `Query`;
  never restate `db::music::TRACK_COLS`; every response, refusals included,
  is a `subsonic-response` envelope.

---

## 7. Working rules

- **Size every change for one family with a huge catalog**
  (`docs/CAPACITY.md`, decided 2026-10-07): a family of any size (sized for
  about a dozen people, no limit enforced), up to 600,000
  songs and 1,000 audiobooks, a smaller footprint than Navidrome and
  Audiobookshelf. Catalog size may cost PostgreSQL rows and storage, never
  server memory: every list is paginated, every catalog-wide question is
  answered in SQL with an index, files are streamed, never read whole. A new
  `fetch_all` over a catalog table is a bug. Memory figures and how to measure
  them: `docs/RAM_USAGE.md`.

- **Licence headers.** Every source file starts with
  `// SPDX-License-Identifier: AGPL-3.0-or-later` (or the language's comment
  form). A CI check enforces it. The full licence text is `LICENSE`; never
  replace it with a pointer.
- **Contributions need the CLA** (`CONTRIBUTING.md`, Phase 6). Until it
  exists, no outside pull request is merged. Reason: the hosted edition
  combines this code with private code, which Kornel's own copyright allows
  and a third party's AGPL contribution would not.
- **The brand is not licensed.** `TRADEMARK.md`. The mark lives in `brand/`
  under trademark terms, not AGPL; an unmodified build may show it, a fork
  must replace it. The console says "own.audio server" and shows the
  licence.
- **Dependency licences are checked at import and at every dependency
  change.** `cargo deny check licenses` (allow-list: MIT, Apache-2.0, BSD,
  ISC, MPL-2.0, Unicode, Zlib, OFL, CC0) and `npx license-checker
  --onlyAllow …` over `frontend/` and `i18n/`. Both run in CI. A new crate
  or npm package with a licence outside the list is refused in review, not
  waved through. Any bundled font or asset ships with its own licence text
  next to it (`THIRD_PARTY_NOTICES.md` lists them). Today nothing is
  bundled: the PT Serif fonts belong to the hosted narration pipeline.
- **`music-metadata` and `audio2-sync` are private and proprietary**
  (decided 2026-10-06). This server never vendors or depends on their code
  and never links `audio2-sync`. Metadata comes through the
  `MetadataProvider` trait: the public MusicBrainz / iTunes / Podcast Index
  provider here, the private mirror in the hosted edition. **MusicBrainz's
  public API allows 1 request per second and requires an identifying
  `User-Agent`**; the token bucket and the header are not optional, and a
  build that hammers musicbrainz.org gets the whole project's IP range
  blocked.
- **Write for strangers.** README, INSTALL, error messages, config names:
  someone who has never heard of the family of repos must be able to run
  this from the docs alone. No "ask Kornel", no internal hostnames.
- **No user-facing text in code.** Console strings live in
  `i18n/strings/<area>.json` (en + cs); `cd frontend && npm run i18n` checks
  and regenerates. Czech follows `i18n/GLOSSARY.md`.
- **No lock/security framing for "private" content.** `private` means "not
  shared with the family", nothing more. No lock icons, no "secure" wording.
- **Migrations are forward-only and numbered.** Never edit a committed one.
- **Work on `main`, no feature branches**, while this is a one-person job —
  the same rule as the rest of the family, for the same reason (a branch
  drifted and cost a hand-merge). Revisit when a second person contributes.
- **The changelog is part of every change** (Kornel's rule, 2026-10-06).
  `CHANGELOG.md`, Keep-a-Changelog form, semver releases:
  - Every commit that changes what a client, operator or self-hoster can
    notice adds a line under `## [Unreleased]`, in the same commit. Pure
    refactors, test-only and CI-only changes get a line too when they change
    how the project is built or verified; typo fixes do not.
  - Sections, in this order and only when non-empty: `### Added`,
    `### Changed`, `### Deprecated`, `### Removed`, `### Fixed`,
    `### Security`. One bullet per change, present tense, starting with the
    thing (endpoint, setting, page, job) in backticks where it has a name:
    `` - `GET /api/v1/family/storage` — a family's bytes per media kind. ``
  - A contract change (new endpoint, field, enum value, `features` key)
    says so and names the new `api.revision`; a deprecation names the
    sunset date (API_COMPATIBILITY.md §5).
  - Cutting a release: rename `[Unreleased]` to `## [X.Y.Z] - YYYY-MM-DD`,
    add the line `Contract revision N.`, open a fresh empty `[Unreleased]`,
    bump `backend/Cargo.toml`, tag `vX.Y.Z`. Never edit a released block
    except to fix a factual error, and then say so in the commit.
  - Don't duplicate `git log`: the changelog explains what changed for the
    reader, not which files moved.
- **Comments explain WHY, not WHAT.** Default to none.
- `cargo check`, `cargo clippy -- -D warnings` (clean since the import —
  keep it so), `cargo test`, the console build, and `conformance/` against
  the compose stack before calling a change done. `gitleaks detect --no-git
  --source .` before every push (`.gitleaks.toml` allowlists the one test
  key and build output).
- **Don't commit or push on the user's behalf** unless asked; prepare the
  change and show it. (Global rule; nothing here overrides it.)

```bash
cp .env.example .env                 # set the three secrets; on this machine use PORT=8083,
                                     # S3_PORT=9002, S3_CONSOLE_PORT=9003 (8080/9000 belong to audio2's dev stack)
docker compose up -d                 # postgres + rustfs + server (built from source)
docker compose build server && docker compose up -d   # after a code or migration change
cd backend && cargo check && cargo clippy -- -D warnings && cargo test
cd frontend && npm run i18n && npm run build
curl -X POST localhost:8083/api/v1/setup/complete -H 'Content-Type: application/json' \
  -d '{"email":"admin@example.com","password":"…","display_name":"Admin"}'   # first admin, once
python3 conformance/run.py --base-url http://localhost:8083 --admin-email admin@example.com --admin-password '…'
```

---

## 8. Releasing

- **GitHub is the home, Docker Hub is where the image lives** (decided
  2026-10-06). Remote `origin` is `github.com/own-audio/server`
  (public since 2026-10-06); a Forgejo mirror remote (`forgejo`) exists on
  Kornel's machine. The images are `ghcr.io/own-audio/server` and `kornelko2/own-audio-server`
  (Docker Hub, Kornel's existing account), the binary `own-audio-server`,
  port 8080, data at `/data`, library folders at `/library/<name>`,
  `PUID`/`PGID` for the container user. These names are permanent from 1.0:
  every self-hoster's compose file carries them. GitHub Actions builds
  amd64 + arm64 on every tag and pushes to Docker Hub (GHCR as a mirror).
- **No Dependabot.** Dependencies are updated by hand before each release
  and verified with the conformance suite; `cargo audit` runs in CI and
  weekly. Questions go to GitHub Discussions, bugs to issues.
- Semver tags `vMAJOR.MINOR.PATCH`; the image is tagged the same plus
  `latest`. The contract revision is stated in every `CHANGELOG.md` entry.
- **Every tag gets a GitHub Release** (a tag alone shows nothing on the
  repo's page; missed until beta.4, added for beta.1–4 on 2026-10-10):
  `gh release create vX --title X --notes-file <the CHANGELOG section> --prerelease`
  (drop `--prerelease` from 1.0), notes ending with the image names and
  links to INSTALL.md / UPGRADING.md at that tag.
- A tag is what the hosted edition pins. **Core changes land here first,
  get tagged, and `audio2` bumps its pin** — never the other way round, and
  never a `[patch]` path override committed on either side.
- Before tagging: the conformance suite green against compose; the hosted
  repo's CI green against the candidate tag (it builds against the new tag
  before the pin is merged).
- Publishing is outward-facing. Claims in the README about what the server
  does must be true of this edition, not of own.audio — the marketing site's
  §3 rule applies here too.

---

## 9. Where we left off (2026-10-08)

`v1.0.0-beta.4` (contract revision 6): the demo account is read-only;
Subsonic song ids for family-shared songs fixed. `v1.0.0-beta.3` (contract revision 6): server id and addresses in
`GET /server`, smaller streams from Subsonic `stream`. `v1.0.0-beta.2` (contract revision 5): identify through the public
MusicBrainz API, podcast search through Apple's directory, getStarred and
playlist reads batched, storage reconcile by page. `v1.0.0-beta.1` (contract revision 4): OpenAPI generated and checked
(`docs/api/openapi.json`, `scripts/check-api-contract.py`), one family per
install, the server as `PUID:PGID`, SMTP only (JMAP removed), nightly
backups and `scripts/pg-upgrade.sh`, Subsonic ids as key reads, streamed
sync lists. `v1.0.0-alpha.7` fixes the streamed track list under gzip (browsers got an
empty response), path claims without indexes (first scans slowed per file)
and reads media in 64 KiB. `v1.0.0-alpha.6` (contract revision 3) adds local storage, the server's media
route (`/api/v1/media`, also for S3 behind firewalls), read-only library
folders, allocator settings and family-sized PostgreSQL, the streamed track
list, inline visibility and stored grouping keys (scale test: docs/CAPACITY.md).
`v1.0.0-alpha.5` fixes audiobook progress in the console (in-file positions, as
the API defines), names format errors in the player, answers 409 for an
undownloaded episode, and loads console pages on demand. `v1.0.0-alpha.4` schedules the hourly `stats_rollup` (smart playlists'
play counts; nothing enqueued it before). `v1.0.0-alpha.3` translates the console's account menu and a few dialogs;
`v1.0.0-alpha.2` (contract revision 2) adds the optional `demo` sign-in
shown on the console's sign-in screen and the version under the form; the
demo at https://demo.own.audio runs it, with the guest account set only in
that server's `.env` (`audio2/deploy/demo/`).

Earlier (2026-10-06, late):

Phase 1 done and tagged `v1.0.0-alpha.1` (contract revision 1; CI green
including the conformance job on a GitHub runner; the release workflow
published `ghcr.io/own-audio/server:1.0.0-alpha.1`, amd64, in 22 minutes).
**Actions minutes are a budget**: the org is on GitHub Free (2,000 min/month
for a private repo) and a full CI run costs about 25; docs-only pushes skip
the heavy jobs and superseded runs are cancelled, but push deliberately,
batch small changes, and prefer a self-hosted runner for the heavy jobs if
the repo stays private for long. `audio2` now depends on
that tag and has no core code of its own; its `hosted/Dockerfile` clones this
repo's console at the same tag. Next: Kornel adds the Forgejo secret
`OWN_AUDIO_SERVER_TOKEN` (GitHub fine-grained PAT, read-only on this repo)
so canary can build; promote; then Phase 3 (OpenAPI from code) and Phase 4
(library folders, local storage, public MusicBrainz). Open without
deadline: rotate the Google Cloud key that leaked into `audio2`'s history
(`AIzaSyD3S28JJ…`, commits `b4b65ae`, `e71caf5`) — a Phase 6 gate; lawyer;
EUIPO. RustFS passed its first full conformance run (321/321); keep watching
it. Mail is SMTP only since 2026-10-07 (JMAP removed); the install
script is gone until Phase 6 writes the new one (compose + INSTALL.md until then).
