# Scope of the open-source server

What the open-source own.audio server does, what it deliberately leaves to
the hosted service, and what neither does yet. Agreed 2026-10-06 (see the
"Decisions" table at the end for who decided what and when); change it only by
editing this file in the same commit as the code.

A feature is **in** when it ships in this repository's binary and image. A
feature is **optional** when it is in, but off until the operator configures
it (a key, a URL, a mail server) — and `GET /api/v1/server` reports it as
`false` until then. A feature is **hosted-only** when its code lives in the
private hosted repository and never in this one.

---

## In

### Accounts and families
- Email + password accounts; first-admin setup (`/setup`); registration open
  or invite-only; family join by link or QR; account claim for invited
  members; device-code sign-in for TVs.
- Families with roles, per-member parental controls and access policies,
  private vs family-shared content, content reports.
- Instance admin: cross-family view, lock/unlock, server dashboard — without
  the credit columns.
- Avatars and family photo.

### Audiobooks
- Upload (multipart for small files, presigned direct-to-storage for large),
  files, authors, series, collections, favourites, per-file names and
  durations, cover art.
- Identify a book against Google Books (optional: needs a key).

### Podcasts
- Feeds, episodes, hourly refresh, server-side episode download, transcripts
  where the feed publishes them, OpenSubsonic podcast surface.
- YouTube channels as podcast feeds and audio download through `yt-dlp`.
- Podcast search, similar shows, categories, browse — through public
  directories (iTunes Search; Podcast Index with a free key).

### Music
- Tracks, releases/albums, album artists, disc numbers, artists with images,
  playlists, smart and generated playlists (rules, intent, affinity,
  sequence), lyrics, stars, duplicate detection, audio analysis.
- Identify tracks against MusicBrainz — directly against the public API in
  this edition (slow by design: 1 request per second, which is why the
  hosted edition runs its own mirror).
- Cover art cascade: embedded art → Cover Art Archive → iTunes → Deezer.
- OpenSubsonic API at `/rest` for third-party music apps.

### Playback and library
- Progress, bookmarks, cross-device play queue, listening sessions,
  personal and family statistics.
- Cross-library search, incremental sync (`/library/changes`), the 30-day
  trash with restore.
- Notification inbox (clients poll; no push delivery anywhere yet).
- The own.audio folder: the file-sync protocol (`/sync/*`) that the Mac
  Finder extension and the future Docker/Windows agents speak.

### Library folders and storage
- **Read-only library folders**: existing music and audiobook collections on
  a disk or NAS share, mounted read-only, indexed in place by a scanner and
  streamed by the server. Nothing is copied, nothing is ever written there.
  Items from folders are read-only in every client.
- **Local storage** for uploads (`STORAGE__KIND=local`), so a self-hosted
  install needs no S3 at all; or any S3-compatible store. RustFS is the one
  the compose stack ships with (on trial since 2026-10-06); Garage and MinIO
  tested; R2 and AWS work.

### Platform
- The web console (React), embedded in the image, in English and Czech from
  the shared `i18n/` catalog.
- **PostgreSQL 16 or newer, only.** No SQLite (decision 10 below).
- Background job worker with per-container job allow-lists.
- Docker image on Docker Hub (amd64 + arm64, built by GitHub Actions),
  compose stack, one-command installer, install/upgrade/backup documentation.
- The API contract: OpenAPI generated from code, the compatibility policy,
  the conformance suite.

## Optional (in, off until configured)

| Feature | Needs | Reported as |
|---|---|---|
| Sign in with Google / Apple / Microsoft | the provider's client ids | `features.auth.<provider>` |
| Music identify | the public MusicBrainz API (`musicbrainz.org`, 1 request/s, keyless) in this edition; the hosted edition uses its private mirror service behind the same provider interface | `features.music_identify` |
| Podcast discovery (search, similar, categories) | the public iTunes Search API (keyless) and, with a free key, the Podcast Index API; the hosted edition uses its private catalogue service | `features.podcast_discovery` |
| Book identify | Google Books key | part of audiobooks; degrades to "unavailable" |
| Mail notifications (family invite, inbox events) | an SMTP server, plain authenticated SMTP only — the JMAP code is replaced, not kept | `features.mail` |
| Artist images | none (Wikidata/Commons, keyless) | always on |

The rule: an optional feature that is off answers `501 feature_unavailable`,
never a fake success.

## Hosted-only (never in this repository)

- **The AI pipelines.** Narrate a book (EPUB/MOBI/TXT → translated,
  narrated M4B) and translate a podcast episode: `audiobook_gen/`,
  `podcast_translate/`, their `db/` modules, the four job types
  (`gen_pipeline`, `assemble_audiobook`, `podcast_translate`,
  `assemble_podcast_translation`), the assembler container, the
  "audiobook ready" mail. Decided 2026-10-06 as the reversible choice: they can
  be opened later with a tag bump if users ask; they cannot be closed again.
  Their tables (`voice_profiles`, `generation_*`, `podcast_episode_translations`,
  `podcast_translation_blocks`) stay in the core schema, empty, like the
  billing tables below. The console's Narrate and Translate pages stay in
  this repo and hide themselves when `features.narration` /
  `features.translation` are false — the UI is forms and polling, nothing
  worth hiding.
- **Money.** The credit ledger, the $5 welcome grant, the daily storage
  charge, credit alerts, Stripe Checkout top-ups and webhook, the billing
  page's server side, credit adjustments in the admin view. The three tables
  they use (`credit_ledger`, `credit_alerts`, `stripe_events`) exist in the
  core schema for migration continuity and stay empty here.
- **Operations of own.audio.** The operator console (`admin/`), the VPS and
  Cloudflare Tunnel/R2/Pages deployment, canary and promote workflows, the
  hosted console build with its baked-in API URL, the Pages functions.
- **Hosted-service rules.** Waitlist credit eligibility, the "playback pauses,
  library kept three months" policy, anything else that only makes sense
  when someone else pays for the disk.

## Not in either edition today

Listed so nobody files it as a packaging bug: push notification delivery,
transcoding or a quality selector, chapter extraction from M4B/ID3,
server-side auto-download rules, bulk delete, resumable generation jobs,
browser-redirect OIDC flows (clients use loopback + PKCE or ID tokens). See
the client guide's "Known gaps".

---

## Decisions

| # | Question | Decision | By / when |
|---|---|---|---|
| 1 | Narration and translation in the open-source server, bring-your-own-key? | **Hosted-only.** "If users later ask for something else we decide differently." | Kornel, 2026-10-06 |
| 2 | SSO providers in the open-source server, config-gated? | **In**, config-gated, off by default | Kornel, 2026-10-06 |
| 3 | Metadata service: open-source the `music-metadata` repo, offer the own.audio mirror, both? | **Neither — `music-metadata` stays private.** The open-source server calls the public MusicBrainz API and public podcast directories directly, behind the same provider interface the hosted mirror uses. | Kornel, 2026-10-06 (revised the same day) |
| 4 | Outbound mail: port to SMTP and keep it optional, or drop mail from the server? | **SMTP only**, optional, for notifications | Kornel, 2026-10-06 |
| 5 | Web console in this repo, billing UI capability-gated | yes | survey, 2026-10-06 |
| 6 | File sync protocol and OpenSubsonic in | yes | survey, 2026-10-06 |
| 7 | Everything under "Money" and "Operations" hosted-only | yes | Kornel's brief, 2026-10-06 |
| 8 | Read-only library folders indexed in place, designed in before 1.0 | **yes** — "typical for self-hoster solutions" | Kornel, 2026-10-06 |
| 10 | SQLite as a second database for self-hosters? | **No — PostgreSQL only.** 10k lines of Postgres-dialect SQL in 320 runtime-checked queries, `SKIP LOCKED` in the job queue, arrays, `jsonb`, views, 85 migrations: two to three months to port and a permanent double test matrix for one maintainer. Self-hosters accept Postgres (Immich, Paperless-ngx, Authentik). If "one container, one volume" ever becomes a real barrier, the answer is a single-container mode with embedded Postgres (`postgresql_embedded`), Phase 7, on demand — one SQL dialect either way. | Kornel, 2026-10-06 |
| 11 | Who is one install for? | **One family**, with **no limit on its members** (sized for about a dozen; "let people use it as they want", same day). Many families on one server is the hosted service's job. Enforcing it in code is planned, see `IMPLEMENTATION_PLAN.md` "Scale and footprint". | Kornel, 2026-10-07 |
| 12 | How large a catalog? | **600,000 songs and 1,000 audiobooks** (data-hoarder households). Catalog size is the database's and storage's job; server memory and latency must not grow with it. Known gaps: `CAPACITY.md`, issue #2. | Kornel, 2026-10-07 |
| 14 | PostgreSQL-only: whose problem are its costs? | **Ours.** It stays the only database (decision 10); before 1.0 the server ships family-sized PostgreSQL settings, nightly automatic backups with a tested restore, and a one-command PostgreSQL major upgrade (plan, Phase 6 item 4). | Kornel, 2026-10-07 |
| 13 | Footprint goal | **Smaller than Navidrome and Audiobookshelf.** Measurements: `RAM_USAGE.md`. | Kornel, 2026-10-07 |
| 9 | Public home GitHub, image on Docker Hub | **yes**: `github.com/own-audio/server` (public), Docker Hub `kornelko2/own-audio-server` (Kornel's existing account, like `mindmapvault-server`), GHCR `ghcr.io/own-audio/server` | Kornel, 2026-10-06 |
