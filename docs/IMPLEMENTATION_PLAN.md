# Implementation plan — the open-source own.audio server

Written 2026-10-06 from a survey of `audio2` (backend 0.1.56, 85 migrations,
~50k lines of Rust), the nine clients, and the earlier MindMapVault
FOSS/SaaS split. Status of each phase is tracked at the bottom; update it as
work lands.

---

## 0. What we are doing, in one paragraph

The `audio2` backend becomes an open-source server under AGPL-3.0, in this
repository, serving audiobooks, podcasts and music from one binary with the
web console embedded. The hosted service at own.audio keeps running the same
code as a thin private binary that **depends on this crate** and adds what
only a hosted service needs: payments, the credit ledger, the storage charge,
the operator console and the production deployment. Every client works
against both because the API is one API, versioned once, and clients ask the
server what it offers instead of assuming.

---

## 1. Decisions taken by the survey, and the ones still open

### Decided by the facts

| Decision | Choice | Why |
|---|---|---|
| Relationship between the repos | **This repo is upstream; the hosted edition depends on it as a Cargo git dependency pinned to a tag.** Not a copy, not a submodule, not a periodically-merged fork. | MindMapVault did a copy-based split (saas imported from server, parity by hand). Their own audit a few months later: 29 shared frontend files and 18 backend files drifted, the server edition shipped a share UI with zero share routes, a 413 fix was never ported. One codebase, one truth. The `audio2` crate was already written as a library with `app::bootstrap()` / `http::router::api_routes()` / `finalize()` "so the SaaS binary can compose core + SaaS routes" — the seam exists, it just was never used. |
| Git history | **Fresh history. A clean snapshot of the tree, not a fork of the `audio2` history.** | A live Google Cloud API key (`AIzaSyD3S28JJ…`) was committed on 2026-07-30 in two commits (`b4b65ae`, `e71caf5`, files `.env`, `backend/.env`, `env.copy.txt`). It is gone from HEAD but present in every clone of the history. **Rotate that key in the Google Cloud console regardless of anything else in this plan.** Publishing the history would publish the key. |
| Licence | **AGPL-3.0-or-later**, full licence text in `LICENSE`, SPDX headers in source. | Network use is the only use that matters for a server; AGPL §13 is what makes a modified hosted copy publish its changes. "or-later" matches `mindmapvault-server`. A ten-line pointer file instead of the full text was already a mistake once (GitHub showed no licence at all). |
| Contributions | **Contributor licence agreement required** (a short "you grant Kornel Maráz the right to use your contribution under any licence" document, signed by PR comment / a CLA bot). | The hosted binary combines this AGPL code with private code. Kornel's own code can be combined freely; a third party's AGPL contribution cannot, without a grant — it would make the whole hosted service AGPL and oblige publishing the billing layer. Until the CLA exists, no outside PRs are merged. |
| Brand | The name **own.audio** and the logo are **not** licensed. `TRADEMARK.md` says so. | AGPL does not stop anyone from running the server, commercially included. It stops them from doing so under this name, and from keeping their modifications private. That is the realistic protection; say it plainly in the README so nobody expects more. |
| Crate name | Keep `audio2` for the Rust crate and binary **for now**; the product name is "own.audio server". | The port is mechanical if names stay. A rename is a later, separate, cosmetic change. Module docs and the Subsonic `serverVersion` string use the product name. |
| Migrations | **All 85 existing migrations are imported verbatim** and stay the core sequence, including the three billing tables they create. New hosted-only tables go in a `hosted` Postgres schema with their own `_sqlx_migrations` (by running the hosted migrator on a connection with `search_path = hosted`; sqlx 0.8.6 has no per-migrator table name). | The production database must be able to switch to the new binary without a baseline dance, and a database created by either edition must run under either. Squashing is a 2.0 decision. The dormant tables are documented, not hidden. |
| Web console | **In this repo**, including the billing page's code, which hides itself when `features.billing` is false. The separate operator `admin/` console is hosted-only and does not come. | A server with no UI is not usable for a self-hoster (first-admin setup lives in the console). The billing UI is a few files (`src/api/billing.ts`, `pages/billing/BillingPage.tsx`, two home widgets, a sidebar entry) and nothing in it is secret. Maintaining a second console is the drift MindMapVault had. |
| i18n catalog | Comes along (`i18n/`). | The console needs it; it is meant to become every client's source anyway. |
| API versioning | Path major (`/api/v1`) + an integer **contract revision** + `GET /api/v1/server` capabilities, additive-only rules, 6-month deprecations, OpenAPI generated from code, a conformance suite run against both editions. | Written out in `docs/API_COMPATIBILITY.md`. The current state is a hard-coded `/api/v1` prefix, a 1,571-line prose guide, no version endpoint, and compatibility handled by "older servers omit X" footnotes. |

### Decided by Kornel, 2026-10-06 (full list in `docs/SCOPE.md`)

1. **Narration and podcast translation are hosted-only.** Reversible later
   (a tag bump opens them), not reversible the other way. The console pages
   stay here, capability-gated. Phase 2 grows by about a week for the extra
   seam.
2. **SSO (Google, Apple, Microsoft) stays in**, config-gated, off by default,
   as it already is.
3. **`music-metadata` stays private; the open-source server talks to public
   directories directly.** A `MetadataProvider` trait with two
   implementations: the private mirror service (hosted edition, configured by
   `METADATA__BASE_URL` + key) and a public provider (this edition:
   `musicbrainz.org` at 1 request/s with the mandatory identifying
   `User-Agent`, Cover Art Archive as today, iTunes Search for podcasts,
   Podcast Index with a free key). Built in Phase 4 next to the scanner,
   which is where identify matters most. Revised from "open the repo" the
   same day.
4. **Mail is SMTP only**, optional, for notifications. The JMAP code is
   replaced, not kept.
5. **Docker Hub for the image, GitHub for the public repository and the
   image builds** (Kornel, 2026-10-06). Names, decided the same day:
   GitHub organisation `own-audio`, repository `own-audio/server`; Docker
   Hub `ownaudio/server`; binary `own-audio-server`; port 8080; data dir
   `/data`; library mounts `/library/<name>`; `PUID`/`PGID`. The
   organisation and the repository exist since 2026-10-06:
   `https://github.com/own-audio/server.git` is remote `origin` (private
   until publication); Forgejo is remote `forgejo`.
7. **Library folders are in scope and designed in before 1.0** (Kornel,
   2026-10-06): self-hosters' files stay on their read-only shares and are
   indexed in place; see Phase 4.
6. **`audio2` stays the hosted repo** and shrinks; renaming it is cosmetic.

---

## 1a. Pre-flight — settle before Phase 1 starts

Found while checking the tree on 2026-10-06. None is large; all are cheaper
now than after the first public push.

**Legal and ownership**

- [ ] **Rotate the leaked Google Cloud key** (§1) — Kornel, 2026-10-06:
      **before the repo goes public, as a Phase 6 gate**, not now. The
      `audio2` history stays private, so exposure is limited to people with
      Forgejo access. Listed in Phase 6 step 5.
- [x] **Five commits (2026-03-29 to 2026-04-09) carry a MANN+HUMMEL address**
      (`6eafb01`, `47e4d83`, `d6a3eb8`, `63a08e6`, `6201e7c`). Kornel's
      call, 2026-10-06: the open-source repo is built from a fresh history,
      so no employer address appears in it. (Git authorship and copyright
      are separate questions; the code was written on personal time and the
      pieces have been rewritten since — recorded here so the question is
      answered if it is ever asked.)
- [ ] **Bundled fonts**: `backend/assets/fonts/PTSerif-*.ttf` are embedded
      by the core too (`metadata/watermark.rs` draws the cover watermark with
      PT Serif Regular), not only by the hosted narration covers — found
      during S6. So the OFL 1.1 text (PT Serif: Copyright 2010 ParaType Ltd,
      Reserved Font Names "PT Sans", "PT Serif", "ParaType") ships at
      `backend/assets/fonts/OFL.txt` and `THIRD_PARTY_NOTICES.md` lists it.
      Done at the import (Phase 1 step 3).
- [ ] **`cargo deny` with a licence allow-list** (MIT, Apache-2.0, BSD,
      ISC, MPL-2.0, Unicode, Zlib, OFL, CC0) over `Cargo.lock` (6,161 lines)
      and `license-checker` over the two `package-lock.json` files, **at the
      import** (Phase 1 step 3), and in CI from then on — Kornel, 2026-10-06;
      rule in `CLAUDE.md`. One GPL-incompatible or proprietary dependency
      found after publication is a public retraction.
- [x] **`music-metadata` and `audio2-sync` stay private, proprietary**
      (Kornel, 2026-10-06; supersedes decision 3's "open the repo"). Add the
      same proprietary `LICENSE` text `audio2` uses to both. Consequence for
      the open-source server: see the metadata discussion point below —
      without a public mirror or a public repo, music identify and podcast
      discovery are hosted-only in practice.
- [x] **Trademark notice and the mark in the repo** (`TRADEMARK.md`,
      `brand/`): unmodified builds may show them, forks replace them.
- [ ] **Trademark registration**: an EUIPO word mark for "own.audio"
      (~€850, one class) is the cheap way to make the notice enforceable.
      Kornel, 2026-10-06: not now, no deadline.
- [ ] A lawyer reads `LICENSE`, `docs/LICENSING.md`, the CLA text and
      `TRADEMARK.md` once. Kornel, 2026-10-06: **not now, no deadline** —
      an open item, not a Phase 6 gate. `docs/LICENSING.md` keeps its "not
      legal advice" line.

**Never copied into this repository**

- `books-data/` — a copyrighted Czech e-book text used for narration tests.
- `.env`, `backend/.env`, `env.copy.txt`, `docs/test-accounts*.md`,
  `deploy/production/`, `.docker-certs/`, `design_artboards/`,
  `design_doc.json`, anything from `freevps-server-docu`.
- Brand lockups beyond the mark (OG images, email logo, blog heroes) stay
  in `audio2-www`; only the mark is in `brand/`, under trademark terms.

**Security — the core becomes readable by everyone, including people
probing api.own.audio**

- [x] **No rate limiting exists on `/auth/login`, `/auth/refresh`,
      `/auth/device/*`, `/join/*`.** Agreed 2026-10-06: `tower_governor`
      per IP and route, configurable (`SERVER__RATE_LIMIT__*`), defaults
      such as 10 login attempts per minute per IP, `429` with
      `Retry-After`. Phase 2. Additive; the hosted edition gets it too.
- [ ] **`CorsLayer::permissive()`** → config allow-list (planned Phase 3;
      must land before Phase 6).
- [x] A **security pass over the auth and families code** with the
      "attacker has the source" assumption: token handling, join-code
      entropy and brute-force surface, the 404-vs-401 leak noted in the
      client guide, presigned-URL scope, path handling in `filesync` and
      the new scanner. Agreed 2026-10-06; half a week in Phase 2, output
      is a fix list worked off before Phase 6. `cargo audit` in CI from
      the first commit, plus a weekly scheduled run.
- [x] `SECURITY.md` with a private disclosure address (`security@own.audio`,
      forwarding like `privacy@`) before the repo is public. Agreed
      2026-10-06; alias is Kornel's to create at the mail provider.
- [ ] Confirm the defaults that matter for a stranger's install:
      `AUTH__REGISTRATION_OPEN=false`, `AUTH__DEV_SEED_ADMIN=false`, a
      generated `AUTH__SESSION_SECRET`, non-root container user.
- [ ] State **"no telemetry, no update checks"** in the README; the code
      has none, and self-hosters look for the sentence.

**Sequencing change**

- [x] **Consolidate the conformance suite before the seam, not after.**
      Agreed 2026-10-06. The
      backend has 232 unit tests and zero database tests; the eight Python
      scripts are the only end-to-end safety net, and Phase 2 refactors the
      most entangled code (billing reaches into families, trash, dashboard,
      the worker). Phase 3 step 3 moves to the start of Phase 2: run the
      consolidated suite against canary before the first seam commit and
      after every one. Phase 2 is then ≈ 3.5 weeks and Phase 3 ≈ 1 week.

**Names that become permanent at 1.0** (a self-hoster's compose file carries
them forever; the crate name does not matter, these do)

- [x] Docker Hub `ownaudio/server` (account still to create); GitHub
      `own-audio/server` exists, private. Kornel, 2026-10-06.
- [x] Binary `own-audio-server`; env var names as already in use; port
      8080; `/data` for `STORAGE__KIND=local`; `/library/<name>` for
      library folders; `PUID`/`PGID`. Kornel, 2026-10-06.
- [x] **PostgreSQL only**, minimum **16** (production runs 16.14), dev on 17;
      stated in the README at import. SQLite rejected 2026-10-06 (SCOPE
      decision 10).

**Expectation setting**

- [x] README says what the project is: one person, no SLA, issues welcome,
      PRs after the CLA, releases when ready. Agreed 2026-10-06.
- [x] `audio2-www`: "self-hostable" and "open source" are claimed on the
      site **only after Phase 6**; the roadmap page gets an "Open-source
      server" item at that point; pricing copy gains "or run it yourself
      for free" then, not before (that repo's CLAUDE.md §3). Agreed
      2026-10-06; nothing on the site until publication.
- [x] Maintenance automation, decided 2026-10-06: **no Dependabot** —
      dependencies are updated by hand before each release (`cargo update`,
      `npm update`, then the conformance suite). `cargo audit` weekly and on
      push stays (agreed under security). Image build and publish on tag.
      GitHub Discussions for questions, issues for bugs.

---

## 2. What moves where

### Comes to this repository (open source)

```
backend/            the whole crate, minus billing/, audiobook_gen/,
                    podcast_translate/ and their db/ modules (see seam below)
  migrations/       all 85, verbatim
frontend/           the web console (billing page capability-gated)
i18n/               the string catalog and its scripts
docs/               the client guide, the endpoint spec, the plans that
                    describe shipped behaviour (file sync, trash, families,
                    permissions, album-artist, music signals, identify) —
                    rewritten where they mention own.audio operations
scripts/            the Python test scripts → become conformance/
docker-compose.yml, deploy/garage.toml, install.sh, INSTALL.md
```

### Stays in `audio2` (private, becomes the hosted edition)

```
backend/src/billing/           Stripe Checkout, webhook, success/cancel pages
db/billing.rs                  the credit ledger (moves into the hosted crate)
the storage_billing job        daily charge, credit_low alerts
backend/src/audiobook_gen/     narration pipeline (Google Translate, TTS, Gemini, ffmpeg)
backend/src/podcast_translate/ episode translation; db/audiobook_gen.rs, db/podcast_translate.rs
jobs gen_pipeline, assemble_audiobook, podcast_translate, assemble_podcast_translation;
the assembler container; mail/narration.rs; GOOGLE_CLOUD__{TRANSLATE,TTS,GEMINI}_API_KEY
docs/podcast-translation-plan.md, docs/audiobook-generation-plan.md
/family/billing*, /admin/families credit adjustment, dashboard billing widgets
admin/                         the operator console
deploy/production/, .forgejo/workflows/   VPS, Cloudflare Tunnel, R2, canary, promote
frontend/functions/            Cloudflare Pages functions for the hosted console build
docs/production-deployment.md, test-accounts*.md, family-billing-plan.md,
sso-payments-plan.md, billing-settings-widgets-plan.md
the waitlist "first 100 get $5" tie-in (todo), the 3-month pause (todo)
```

### Never copied anywhere public

`.env`, `backend/.env`, `env.copy.txt`, `docs/test-accounts.local.md`,
anything from `freevps-server-docu`. CI in this repo runs `gitleaks` on every
push; Phase 1 runs it over the whole imported tree before the first commit.

---

## 3. The seam

The hosted edition needs five ways into the core (billing and the AI
pipelines both use them; nothing is specific to either). Each is one small, named
extension point; nothing else about the core knows the hosted edition exists.

1. **Routes.** `http::router::api_routes()` stops nesting `/billing`,
   `/audiobook-gen` and `/podcast-translate`; `finalize()` stops merging the
   Stripe pages. The hosted binary does
   `finalize(api_routes().merge(hosted::routes()), state)`. The `/family/billing*`
   handlers move out of `families/mod.rs` into the hosted crate (they are
   28 call sites there today, all billing). The core answers the three
   removed prefixes with `501 feature_unavailable` so an old client gets a
   stable answer instead of `not_found`.
2. **Hooks.** An `Arc<dyn Hooks>` in `AppState`, `NoopHooks` in this repo:

   ```rust
   #[async_trait]
   pub trait Hooks: Send + Sync {
       async fn member_joined(&self, family: FamilyId, user: UserId) {}      // welcome grant
       async fn authorize_spend(&self, family: FamilyId, kind: SpendKind, estimate_micro: i64)
           -> Result<SpendToken, SpendRefused> { Ok(SpendToken::free()) }      // narration, translation, trash restore
       async fn commit_spend(&self, token: SpendToken, actual_micro: i64) {}
       fn features(&self) -> HostedFeatures { HostedFeatures::none() }        // billing, payments → /server
       fn dashboard_extras(&self) -> ... { none }                             // admin dashboard billing block
   }
   ```

   The call sites today: `db/families.rs:165` (welcome grant on
   membership), `trash/mod.rs:210,293` (restore charge and balance check),
   `dashboard.rs` (billing totals). The narration charges in
   `audiobook_gen/mod.rs:663` and `podcast_translate/mod.rs:676` move to the
   hosted crate with their modules. After the seam, `grep -ri "billing\|gen_pipeline\|podcast_translate" backend/src`
   in this repo returns only migrations and the capability flags.
3. **Jobs.** The worker's string-dispatched `execute_job` gains a registry:
   `jobs::register("storage_billing", handler)`, and the same for the four
   generation/translation job types. The core's own job types stay as they
   are. `WORKER_JOB_TYPES` keeps working unchanged; a type nobody registered
   is logged and skipped, never claimed.
4. **Config.** `BillingConfig`, `StripeConfig` and the three Google AI keys
   leave `AppConfig` (`GOOGLE_CLOUD__BOOKS_API_KEY` stays — book identify is
   in scope). The hosted binary loads its own `HostedConfig` from the same
   environment (`STRIPE__*`, `BILLING__*`, `GOOGLE_CLOUD__*`) next to
   `AppConfig::load()`. `METADATA__BASE_URL` loses its own.audio default.
5. **Migrations.** `db::migrate(&pool)` runs the core sequence. The hosted
   binary then runs `hosted::migrate(&pool)` in the `hosted` schema. The core
   never names a `hosted.*` table.

Everything else — SSO providers, metadata lookup, mail, file sync,
Subsonic, YouTube import, book identify — stays in the core, switched by
configuration as it is today, and **reported truthfully by `/api/v1/server`**.

---

## 4. Phases

Durations assume one person, part-time alongside client work. Each phase ends
with the hosted service still running (Phase 2 is the only one that touches
production) and with `cargo check`, `cargo clippy`, the console build and the
conformance suite green.

### Phase 0 — Repo, licence, policy (this session)

- [x] `LICENSE` (full AGPL-3.0 text), `.gitignore`
- [x] `docs/API_COMPATIBILITY.md` — the versioning policy
- [x] `docs/IMPLEMENTATION_PLAN.md` — this file
- [x] `CLAUDE.md`, `README.md`, `TRADEMARK.md`
- [ ] **Rotate the leaked Google Cloud key** (Kornel, Google Cloud console)
- [x] Decide the open scope questions (`docs/SCOPE.md`); GitHub org / Docker Hub namespace still open
- [x] `docs/LICENSING.md` — why AGPL, what was rejected

### Phase 1 — Clean import (≈ 1 week)

Goal: this repo builds and passes the smoke test on its own, with no hosted
code in it, and the tree is secret-free.

1. Copy the tree listed in §2 from `audio2` at a tagged commit
   (`foss-import-base`). Record that commit hash in the import commit message
   — it is the one link between the two histories.
2. Delete what stays hosted-only (§2). The build will break where billing is
   referenced; **do not fix by stubbing** — leave it broken until Phase 2's
   seam, or do Phase 2's seam first in `audio2` and import after (preferred,
   see Phase 2 step 1).
3. `gitleaks detect --no-git` over the tree. Grep for `own.audio` operational
   references (`api.own.audio`, `metadata.own.audio`, tunnel names, the VPS
   IP) and keep only the ones that are legitimately defaults or examples.
4. `main.rs` uses the library crate (`use audio2::app;`) instead of
   re-declaring every module with `#![allow(dead_code)]`. This is what makes
   the binary and the hosted binary build the same code.
5. Add SPDX headers (`// SPDX-License-Identifier: AGPL-3.0-or-later`) with a
   script; a CI check keeps them present.
6. `docker compose up -d` from a clean checkout → `install.sh` → first admin
   through the console → `conformance/` passes. Fix the stale README/INSTALL
   claims found in the survey ("first registered user becomes admin" is
   wrong — it is `POST /setup/complete`; `STORAGE__DATA_DIR` does not exist;
   there is no Tauri desktop app).
7. CI (Forgejo Actions here, mirrored to GitHub later if the repo goes
   there): `cargo check`, `cargo clippy -D warnings`, `cargo test`, console
   build + `npm run i18n` check, `gitleaks`, compose up + conformance.

Exit: first tagged pre-release `v1.0.0-alpha.1`. Nothing public yet.

### Phase 2 — The seam, and production on the two-crate build (≈ 3.5 weeks)

Goal: `audio2` production runs a binary built from `hosted/` + this crate.

0. **Conformance first** (moved up from Phase 3) — **done 2026-10-06**:
   `conformance/` in this repo (`run.py`, `core.py`, seven suites, 316
   checks). Green against the local stack (316 ok) and against canary
   (142 ok, trash and filesync skipped there: they need SQL access and a
   server that can reach the test machine). Keep it green through every
   step below; run it before and after each seam commit.
1. **Build the seam in `audio2` first**, on `main`, while it is still one
   repo: introduce `Hooks`, move billing call sites behind it, split routes,
   move config, add the job registry, add per-route rate limiting
   (`tower_governor`) and do the security pass (both pre-flight, agreed). Production keeps running the single
   binary throughout (the hooks are wired to the real billing). This is
   refactoring with the full test surface available, and it means Phase 1's
   import is of code that already has the seam.
2. In `audio2`, create `hosted/` (binary crate `audio2-hosted`) that
   depends on `audio2 = { git = "https://jo.marazfamily.eu/kornelko/own-audio-foss.git", tag = "v1.0.0-alpha.N" }`
   and moves `billing/`, `db/billing.rs`, the storage-billing job, the
   `/family/billing*` handlers, the `admin/` console and the production
   deploy under it. A `[patch]` to a sibling path checkout is allowed
   **locally only**; CI in `audio2` fails if `Cargo.toml` contains it.
3. Hosted migrator in the `hosted` schema. Verify against a dump of
   production restored into Postgres 16.14 (production is 16, dev is 17 —
   already a known trap).
4. Delete `audio2/backend/src` except what `hosted/` owns. The Dockerfile
   builds `hosted/`. Canary deploys automatically on push, runs the smoke
   test and the conformance suite; promote by hand as today.
5. `GET /api/v1/server` (§3 of the compatibility policy) in the core, with
   `features` assembled from config + `hooks.features()`. The
   `feature_unavailable` 501 convention replaces today's mix of 400/500/501
   for unconfigured features. (In the hosted edition, narration without a
   TTS key currently *simulates progress and produces no audio*; that becomes
   a 501 at submit time there too.)

Exit: production and canary on the two-crate build for a week without
incident; `audio2` CLAUDE.md rewritten for its new shape (see §6).

### Phase 3 — The contract becomes executable (≈ 1 week)

1. `utoipa` annotations on every `/api/v1` handler and DTO →
   `docs/api/openapi.json`, committed, CI fails when stale. The 1,100-line
   endpoint spec is retired in favour of it; the client guide stays as prose
   and links into it.
2. Contract revision constant, bumped by the same commit as any additive
   change; a CI check diffs `openapi.json` against the previous tag and
   refuses a change without a bump.
3. `conformance/` (already consolidated in Phase 2 step 0) learns to read
   `/server` and skip features that are `false`; runs in this repo against
   compose and in `audio2` against canary.
4. Document the error-model quirk (only `/podcasts/*` returns 404; other
   modules return 401 for not-found, which makes clients refresh tokens for
   nothing). Fixing it is a status-code change for a handled case, so it is
   **not** allowed inside v1 — record it as the first entry on the `/api/v2`
   list instead.
5. Tighten `CorsLayer::permissive()` to an allow-list from config
   (`SERVER__CORS_ORIGINS`, default: the server's own origin). Additive —
   same-origin console keeps working; the hosted console build sets it.

Exit: `v1.0.0-beta.1`; the compatibility policy is in force.

### Phase 4 — Library folders, local media, public metadata (≈ 4 weeks)

Self-hosters already have their music and audiobooks on a disk or NAS share.
They will not re-upload a 2 TB library into Garage, and most will not want
Garage at all. This phase is designed in **before 1.0** because it fixes the
data-model shape (`media_objects.backend`) and the stream-URL shape that
every self-hosted install will carry forever. The gap analysis against
Audiobookshelf and Navidrome (`docs/backend-gap-analysis-abs-navidrome.md`)
already names the scanner as the one structural thing both have and audio2
lacks.

One mechanism, three uses:

**A. Library folders — read-only, indexed in place.** Configured as
`LIBRARY__FOLDERS` (a JSON list: `{ "path", "kind": "music" | "audiobooks", "family": <id or "default">, "visibility": "family" | "private" }`),
mounted into the container with `:ro`. A `library_scan` job walks each tree,
reads tags with `lofty` (already a dependency), checksums files (the
`media_checksum` pattern), and creates or updates tracks, books and files
whose `media_objects` row has `backend = 'folder'`, `bucket = <folder id>`,
`object_key = <relative path>`. Music is organised by tags, not by path
(Navidrome's model); audiobooks follow the `Author/Title/*.{m4b,mp3}`
convention with `cover.jpg` / `folder.jpg` sidecars (Audiobookshelf's). The
scan is incremental (mtime + size, checksum on change), hourly by default
(`LIBRARY__SCAN_INTERVAL_SECS`), and on demand (`POST /library/folders/{id}/scan`,
`GET /library/folders` for admins). Subsonic `startScan` / `getScanStatus`
become real. A moved or renamed file is matched by checksum, so progress,
stars and playlists survive; a vanished file marks the item `missing` rather
than trashing it (the server cannot restore a file it does not own).

Items from folders are **read-only**: no upload into a folder, no deletion
of the file, trash never purges it, `metadata/apply` writes the database and
not the tags, `file-tags` reports read-only. Responses gain two additive
fields: `source: "upload" | "folder"` and `read_only: bool`, so clients can
hide the actions that cannot work. `features.library_folders` is `true` when
at least one folder is configured.

**B. Streaming without S3.** `GET /api/v1/media/{object_id}?t=<token>`
serves a file with HTTP Range support, the token an HMAC over object id +
expiry (4 h, like the presigned URLs). `StreamResponse.url` points here for
`folder` and `local` objects and at the presigned URL for `s3` ones. Clients
already treat the URL as opaque and expiring, so none change.

**D. Public metadata providers.** `metadata/` gets a `MetadataProvider`
trait. `MirrorProvider` wraps today's client for the private service and is
what the hosted edition configures. `PublicProvider` ships here: MusicBrainz
web service (`musicbrainz.org/ws/2`, a token bucket at 1 request/s, the
identifying `User-Agent` MusicBrainz requires, with contact address from
config), Cover Art Archive as today, iTunes Search for podcast search, and
Podcast Index (free key, optional) for categories and similar shows. The
scanner's tag-based identify queues lookups through the provider so a
10,000-track first scan takes hours rather than failing — the console shows
the queue. `features.music_identify` and `podcast_discovery` are `true` when
any provider is configured; the public one is on by default in this edition.
Add ≈ 1 week to this phase.

**C. Writable local storage.** `STORAGE__KIND = s3 | local`;
`local` takes `STORAGE__DATA_DIR` and makes Garage optional. Uploads keep the
presign flow: `uploads/presign` returns `PUT /api/v1/media/upload/{token}`
instead of an S3 URL, and the client PUTs the body there exactly as it does
to S3 today. The hosted edition stays `s3`.

Schema (additive): `media_objects.backend TEXT NOT NULL DEFAULT 's3'`
(check `s3 | local | folder`), `media_objects.folder_id` nullable, a
`library_folders` table (id, family_id, kind, path, visibility,
last_scan_at, last_scan_status). Every existing row is `s3` by default, so
the hosted database is untouched in behaviour.

Rules that keep it safe:

- **The server never writes inside a library folder.** Files are opened
  read-only, the mount is `:ro`, and a test asserts that no code path under
  `library/scan` or `storage/` takes a writable handle to a folder root.
  `storage_sweep`, `trash_purge` and `find_unreferenced` skip `folder`
  objects entirely — a sweep that deleted a self-hoster's only copy of their
  CDs would end the project.
- `object_key` is canonicalised and must resolve under the folder root
  (path traversal); names are stored NFC (the `filesync::paths` rule).
- The scanner batches (a 100k-track library must not hold a transaction or
  a request open); counts and errors are visible in the console and in
  `GET /library/folders`.
- The container runs as a configurable non-root user (`PUID`/`PGID` as the
  linuxserver.io images do), documented next to the volume mounts in
  `INSTALL.md`; `install.sh` asks for library paths.
- The hosted edition's storage billing counts `backend = 's3'` only
  (folder objects are not stored by anyone). This is a filter in the hosted
  crate, not a core concern.

Exit: a fresh install with `STORAGE__KIND=local`, no Garage, one read-only
music share and one audiobook share, plays in the console and in the Mac
app, survives a rename on the share, and the conformance suite passes with
`library_folders: true`.

### Phase 5 — Clients learn to ask (rolling, per repo)

What the survey found: every client already lets the user type a host, all
use `/api/v1`, all probe `GET setup/status`, and the only server-driven
gating is `auth/providers` (SSO buttons) and `family/billing → payments.enabled`
(Stripe). Nothing handles 501. Android and tvOS offer `https://api.own.audio`
as a one-tap default; Mac and iOS do not. No client has a "server kind".

Per client, in the order the Mac-first rule implies (Mac → web console →
iOS book/podcast/music → tvOS → Android ×3 → Windows → `audio2-sync`):

1. Call `GET /api/v1/server` once after `setup/status`, cache it with the
   session, treat 404 as the pre-discovery baseline. One `ServerFeatures`
   type in the shared layer (`Audio2Networking` for Apple, each `core/network`
   copy on Android, `Audio2.Networking` on Windows, `api/client.ts` on web).
2. Replace the two ad-hoc gates with `features.*`: SSO buttons, billing page
   and storage/credit widgets (`billing`), top-up (`payments`), Narrate and
   Translate entry points (`narration`, `translation`), Identify
   (`music_identify`), podcast discovery (`podcast_discovery`), the Finder
   folder (`file_sync`), library-folder admin (`library_folders`). Hide
   delete, trash and write-tags actions on items with `read_only: true`.
3. Map `501 feature_unavailable` to a user-visible "this server doesn't
   offer X" error instead of a generic network error (`AppError` on Apple,
   `ApiException` on Windows, `apiErrorMessage()` on Android).
4. Show `edition` and `version` on the account/about screen for bug reports.
5. Run the conformance suite's client-side counterpart (the existing UI
   tests) against the FOSS compose stack once in CI, not only against canary.
6. README of each client: minimum contract revision, last tested revision.

No client needs a "hosted vs self-hosted" switch; the Android/tvOS one-tap
default is just a prefilled host and stays.

### Phase 6 — Release for self-hosters (≈ 1 week)

1. **GitHub is the public home; Docker Hub is where the image lives.** The
   repository is `own-audio/server` on GitHub, private until publication. GitHub Actions builds the multi-arch image (amd64 +
   arm64) on every tag and pushes `ownaudio/server:v<semver>` and `latest`
   to Docker Hub, with GHCR as a mirror. Forgejo stays a mirror.
   `docker-compose.yml` uses
   `image: ${SERVER_IMAGE:-ownaudio/server:latest}` with
   Postgres and, optionally, an S3 store — **RustFS** (v1.0.1, Rust,
   S3-compatible) is on trial as that default since 2026-10-06 (Kornel's
   call: use it for development and testing, report bugs upstream); Garage
   stays the documented alternative until the trial is over; `install.sh` gets an update mode and
   asks for library folders (Phase 4). A `docker run` recipe for an existing
   Postgres, with local storage as the default and S3 as the alternative.
2. SMTP mail, optional, replacing JMAP (decided; `mail/` is rewritten, the
   `audio2-www` waitlist mailer is the reference implementation).
3. Docs: `INSTALL.md`, `UPGRADING.md` (back up first; forward-only
   migrations; the Postgres 16/17 note), `BACKUP.md` (database + bucket;
   never the MusicBrainz mirror), `docs/EDITIONS.md` (what the hosted
   edition adds, so nobody opens an issue asking where payments are),
   `CONTRIBUTING.md` with the CLA, `SECURITY.md` with a contact address,
   `docs/LICENSING.md` kept current,
   `CHANGELOG.md` restarted at 1.0.0 with a pointer to the pre-split history
   summary.
4. Run the whole thing on a machine that is not Kornel's: fresh VPS, follow
   `INSTALL.md` only, connect the Mac and one iPhone app. Fix what breaks.
5. **Gate: rotate the Google Cloud key that leaked into `audio2`'s history**
   (pre-flight), create `security@own.audio`, confirm `gitleaks` is clean on
   the whole tree. Then tag `v1.0.0`. Make the repository public. Announce on the roadmap page
   and the blog (audio2-www; "self-hostable" becomes a true claim then, not
   before — §3 of that repo's CLAUDE.md).

### Phase 7 — Later, deliberately

- Single-container mode with embedded Postgres (`postgresql_embedded`) for
  self-hosters who want one container and one volume — **only if demand
  appears**; SQLite was considered and rejected (`docs/SCOPE.md` decision 10).
- Filesystem watching (inotify / FSEvents) for library folders instead of the
  hourly scan; a Docker/NAS agent built on `audio2-sync` for the own.audio
  folder.
- Migration squash at 2.0 (one `0001_schema.sql` for fresh installs; existing
  databases baselined by the version they are on).
- `/api/v2` list: the 401/404 error model; anything else that accrues.
- Crate and binary rename to `own-audio-server` if it still matters.

---

## 5. Risks and how the plan handles them

| Risk | Handling |
|---|---|
| Drift between editions | Structurally impossible to drift the core: there is one copy, and the hosted repo cannot build without a tag of this one. Only the hosted layer can be out of date, and it is small. |
| A core change breaks the hosted binary | `audio2` CI builds against the new tag before the pin is bumped; the conformance suite runs on canary. A core change that needs a hosted change ships as "tag core → bump pin + hosted change in one `audio2` commit". |
| Old clients in the wild during the switch | Phase 2 changes no contract: same paths, same DTOs. `/server` is additive. The 501 convention replaces responses no client handled anyway. |
| Secret leakage | Fresh history; gitleaks before the first commit and in CI; the key already leaked gets rotated. |
| Self-hosters hit features that silently do nothing (narration without TTS "succeeds" with no audio today) | The capabilities endpoint and the 501 convention make unavailability explicit; the console hides what is `false`. |
| The scanner or a sweep deletes or rewrites a self-hoster's files | Library folders are read-only by construction (`:ro` mount, read-only handles, sweeps skip `folder` objects, a test guards the write path). |
| A 1.0 self-hoster later has to migrate storage layout | `media_objects.backend` and the server media route ship before 1.0 (Phase 4), so local, folder and S3 objects coexist from the first release. |
| Legal: contributions | CLA before the first outside PR; no PRs merged before it exists. |
| MusicBrainz mirror is a 49 GB dependency | Not needed here: the open-source edition uses the public API at 1 request/s behind the same provider interface. Slow, honest, keyless. |
| Scope: one person, nine clients | Phases 1–4 and 6 are server-only. Phase 5 is additive per client and can trail by months without breaking anything, because the servers keep answering old clients exactly as today. |

---

## 6. The two CLAUDE.md files after the split

This repo's `CLAUDE.md` is written now. `audio2`'s is rewritten in Phase 2 to
say: it is the hosted edition; the core is a dependency, never edited here;
to change core behaviour, change it in `own-audio-foss`, tag, bump the pin;
the hosted layer is billing/payments/operations only; the seam is the five
extension points in §3 and nothing else; `[patch]` path overrides never get
committed. The client repos' CLAUDE.md files gain one paragraph in Phase 4:
features come from `/api/v1/server`, and both editions must pass.

---

## 7. Status

| Phase | State | Notes |
|---|---|---|
| 0 | done except open items (2026-10-06) | Files written; scope, licence, names, security, sequencing decided; remote `own-audio/server` on GitHub. Open without deadline: key rotation (Phase 6 gate), lawyer, EUIPO |
| 1 | not started | |
| 2 | in progress (2026-10-06) | step 0 done: conformance suite green locally and on canary |
| 3 | not started | |
| 4 | not started | library folders + local media + public metadata providers |
| 5 | not started | clients |
| 6 | not started | release: GitHub + Docker Hub |
| 7 | not started | |
