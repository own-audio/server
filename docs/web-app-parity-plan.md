# Web app — parity with the Mac app

The plan for turning `audio2/frontend` (the React console) into the own.audio
**web app**: the Mac app's feature set, in a browser, minus the things that only
make sense on a desktop with a filesystem. Roadmap Phase 2 in
`audio2-www/docs/roadmap.md` ("Library in any browser: management plus
playback"). Written 2026-08-27 from a read of `audio2-mac` (sidebar, Settings
tabs, feature packages, sub-plans), the current `frontend/` code, and the
endpoint index in `docs/android-client-guide.md` §14.

**Legend:** ✅ done · 🚧 in progress · ⬜ not started

---

## 1. Scope

### In scope — everything the Mac app does that isn't about *this machine*

Library browsing and playback for all three kinds, upload, metadata editing,
playlists, collections/series/authors/tags, favorites, search, Home dashboard,
family (members, invites incl. QR/link join, per-member access), billing
(storage, credit, alerts, top-up), stats, notifications inbox, devices,
account (profile, password, photo, delete, Subsonic key, Google sign-in),
playback settings, AI narration wizard, episode translation, music identify
(MusicBrainz), lyrics, duplicates review, instance admin.

### Out of scope — desktop-only, by the user's decision

| Mac feature | Why it stays on the Mac |
|---|---|
| Library folder, mirror rules, eviction, storage breakdown, "adopt files copied in" | Needs a persistent local filesystem. A browser has none. |
| Downloads / "On This Mac" section, Transfers screen, bandwidth limit | Same. Podcast auto-download rules are client-side on the Mac too. |
| Prevent-sleep, media keys via `MPRemoteCommandCenter`, menu-bar commands | Browser gets the Media Session API instead (in scope, W3). |
| Local-only (never uploaded) audiobooks and music | Filesystem-backed. |
| **Connected Servers (Audiobookshelf / Navidrome)** | The Mac talks to those servers directly from the client. A browser can't (CORS, mixed content, credentials in a tab) without the backend proxying — that is backend work, not a web-app feature. **Deferred, see open question Q3.** |
| Drag-and-drop *folder* upload with recursive scan | Partially possible (`webkitdirectory`, File System Access API in Chromium). Keep file/multi-file drop; folder drop is a nice-to-have in W4, not parity. |
| Gapless/crossfade music engine, EQ | Possible with Web Audio but not parity-critical. W3 ships a plain `<audio>` player; crossfade is a later polish item. |

---

## 2. What exists today (`frontend/`, ~10k lines) — keep, rework, or drop

Read before building anything: a lot of the API layer is already right.

**Keep as-is (API layer, stores):** `src/api/*` — auth (incl. Google, Subsonic
key, admin users), audiobooks (multipart + presigned `from-uploads` flow),
authors/tags, collections/series/favorites, podcasts, music, playback,
generation, podcastTranslate, setup, uploads. `authStore`, `playerStore`
(queue, repeat, shuffle), `audiobookPlayerStore`. TanStack Query + axios +
zustand + react-router are fine choices; don't swap the stack.

**Rework (right feature, wrong shape):** every page under `src/pages/` and
`AppShell.tsx`. They're Tailwind `gray-*/indigo-*`, **dark-only, hardcoded**,
a flat top nav with no sidebar, no detail column, no design tokens, no brand
colour anywhere. The nav links to `/audiobooks/sharing` which has **no route**
(404 today). Player bar lives inline in `AppShell` (800 lines) and should be its
own component.

**Missing entirely:** Family (no `api/family.ts` at all), Billing, Stats,
Notifications inbox, Devices/sessions UI, Home dashboard, playlists cover/
reorder polish, music identify, lyrics, duplicates, join-by-code page, light
theme.

**Decision: rebuild in place, not a new repo.** Keep `frontend/` (served by
the backend's `ServeDir("ui/dist")` fallback, deployed as `app.own.audio` /
`app-canary.own.audio` per `docs/production-deployment.md` §1b). Replace the
shell and pages phase by phase; the API layer carries over.

---

## 3. Design — "2026, with the proper branding"

Source of truth: `audio2-www/docs/brand-identity-brief.md` Parts 1–4, and the
neutrals in `audio2-android-book/docs/design-tokens.json`.

- **Accent = brand core purple** `#6E44FF` light / `#9B7BFF` dark. One accent
  per screen section. Cloud colours mark *which cloud you're in* — Book gold
  (`#8A6100`/`#FABB05`), Podcast sky (`#0A6E9C`/`#29ABE2`), Music red
  (`#C42B21`/`#FF6B61`) — as a tint (6–10% over `--bg-alt`), a sidebar
  indicator, and cover placeholders. Never as text on light backgrounds, never
  all together except in the mark. (⚠ `design-tokens.json` still says
  `brand.primary = #0071E3` blue — see Q1.)
- **Neutrals** exactly as the tokens: bg `#FFF/#000`, bgAlt `#F7F7F8/#0C0C0E`,
  card `#FFF/#131316`, fg `#1D1D1F/#F5F5F7`, muted `#6E6E73/#98989D`, border
  `#E5E5E7/#1D1D1F`. **Light and dark both, system-following with a toggle**
  (same three-state model as the marketing site: `data-theme` attribute +
  `prefers-color-scheme`).
- **Type:** system stack (`-apple-system, … Segoe UI, sans-serif`) — the
  brand brief forbids third-party fonts without a decision. Tight tracking on
  headings (`-0.02em`), 15–17 px body.
- **Radii** from tokens: card/cover 12, mini-player 16, sheet 28, pill 999.
- **Look:** Linear/Apple restraint. Generous whitespace, hairline borders,
  flat covers with a soft shadow on hover, no gradients/glass/blur panels
  except the frosted bottom player. Motion 200 ms cover crossfade, 300 ms
  view transitions (View Transitions API where supported), nothing looping.
- **Layout = the Mac navigation model**, because it's the product's shape:
  sidebar (Home · Library: Audiobooks / Podcasts / Music · Admin) → **content
  column** (grid ⇄ list toggle, sort, search) → **detail column** (the one
  selected thing). Drill-down is a selection swap, not a route push, on wide
  screens; on < 900 px the detail column becomes a route (`/music/albums/:id`)
  so mobile browsers still work. Bottom mini-player is persistent across all
  routes.
- **Iconography:** no lock icons, ever. Family badge for shared content,
  favorite badge; nothing for private (it's the default). Icons: one
  consistent set (Lucide — MIT, tree-shakeable, matches SF Symbols weight).
- **Implementation:** Tailwind v4 stays, but *only* through CSS variables
  defined once in `src/styles/tokens.css` (`--color-accent`, `--color-book`,
  …) and exposed via `@theme`. No raw `gray-800` classes anywhere in new code
  — that's the lint rule that keeps the brand from drifting. A drift test
  (`tokens.test.ts`) compares `tokens.css` against `design-tokens.json`, like
  the Android and Swift mirrors do.
- Naming: "own.audio" wordmark in the sidebar (static SVG mark from
  `audio2-www/src/lib/mark.ts`, compact variant), section names "Audiobooks /
  Podcasts / Music" in-app (the "Cloud" names are marketing prose).

---

## 4. Parity matrix

Status columns: **Mac** = what the Mac app has (from its plans, 2026-08-27);
**Web now** = current `frontend/`; **Phase** = where it lands below.

### Shell, auth, account

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| Setup wizard (first user) | `/setup/*` | — (Mac uses host entry) | ✅ | W1 keep |
| Email/password sign-in, register with invite code | `/auth/*` | ✅ | ✅ | W1 restyle |
| Google sign-in | `/auth/providers`, `/auth/google` | ✅ | ✅ | W1 restyle |
| Apple sign-in | `/auth/apple` | ✅ (dormant, needs Apple dev account) | ✅ (dormant) | W1 (button only when `providers` says so) |
| Join by code / link / QR landing (`/join/:code`) | `/join/{code}`, `/join/{code}/claim`, `/family/invites/accept` | ✅ (creates) | ✅ | W6 — spec is `family-join-qr-plan.md` Phase B |
| Sidebar + content/detail layout, theme toggle | — | ✅ | ✅ | W1 |
| Profile edit, photo | `PATCH /users/me`, photo upload | ✅ | ✅ | W7 |
| Change password (warns: signs out other devices) | `POST /auth/password` | ✅ | ✅ | W7 restyle |
| Devices/sessions, revoke (current one never revocable) | `GET/DELETE /auth/sessions` | ✅ | ✅ | W7 |
| Subsonic key show/regenerate | `/users/me/subsonic-key*` | ✅ | ✅ | W7 restyle |
| Delete account (typed confirmation) | `DELETE /users/me` | ✅ | ✅ | W7 restyle |
| Sign out = revoke session then clear tokens | `POST /auth/logout` | ✅ | ✅ | — |

### Home

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| Continue listening | `/library/continue` | ✅ widget | ✅ (in "Library" page) | W2 |
| Recent songs / albums / playlists widgets | `/music/*` | ✅ | ✅ | W2 |
| Listening stats widget | `/stats/me` | ✅ | ✅ | W5 |
| Storage & credit widget | `/family/billing` | ✅ | ✅ | W6 |
| Widget picker (Settings → Home) | local prefs | ✅ | ✅ (on Home itself) | W2 (localStorage) |
| Global search across all kinds | `/library/search` | ✅ per-section | ✅ | W2 (⌘K palette) |

### Audiobooks

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| Grid / list / grouped-by-author, sort, search | `GET /audiobooks` | ✅ | 🚧 grid + list, sort, search; no author grouping | W2 |
| Detail: cover, meta, files, chapters (if any), play | `/audiobooks/{id}*` | ✅ | ✅ | W2 restyle |
| Player: speed 0.5–3×, skip ±, sleep timer, bookmarks, chapter/file prev-next, resume | `/playback/*` | ✅ | ✅ | W3 |
| Upload (multi-file, order by `relative_path`, presigned flow >100 MB) | `/uploads/*`, `/audiobooks/from-uploads` | ✅ | ✅ | W4 restyle + queue UI |
| Edit metadata, cover, reorder files, delete | `PUT /audiobooks/{id}`, `/files/reorder`, `/upload-cover` | ✅ | ✅ | W4 restyle |
| Authors (CRUD, roles), tags | `/audiobooks/authors*` | 🚧 | ✅ | W4 |
| Collections, series, favorites | `/audiobooks/organize/*` | ✅ (add to collection) | ✅ | W4 |
| Visibility private ⇄ family | `PUT …/visibility` | ✅ | ✅ | W4 |
| Multi-select + batch (visibility, delete, add to collection) — N client requests | — | ⬜ (planned P8) | ⬜ | W4 |

### Podcasts

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| Feed grid/list, search field | `GET /podcasts` | ✅ | ✅ | W2 |
| Feed detail + paged episodes, remaining time, played state | `/podcasts/{id}/episodes` | ✅ | ✅ | W2 restyle |
| Play episode (server-side download gate: `has_local` → `/download` → `/stream`) | ✅ | ✅ | ✅ | W3 |
| Mark played / unplayed, mark all played | `/playback/episodes/progress(/bulk)` | 🚧 | ✅ | W3 |
| Add by RSS/YouTube URL, directory search, visibility at subscribe | `/podcasts/subscribe`, `/podcasts/search` | ✅ | ✅ | W4 restyle |
| Refresh, sync-images, unsubscribe, change visibility | `/podcasts/{id}/*` | 🚧 | ✅ | W4 |
| Episode translation (quote → create → poll → play) | `/podcast-translate/*` | ✅ | ✅ | W8 restyle |

### Music

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| Artists / Albums / Genres / Playlists modes, grid ⇄ list, search | `/music/*` | ✅ | ✅ | W2 |
| Detail column = tracks of the selection, play all / shuffle | — | ✅ | ✅ | W2 |
| Player: queue, shuffle, repeat, volume, resume | `/playback/queue`, `/music/tracks/{id}/progress` | ✅ | ✅ | W3 restyle |
| Cross-device queue sync (also shared with Subsonic apps) | `/playback/queue` | ✅ | ✅ | W3 |
| Upload tracks (one request each), progress | `/music/tracks/upload` | ✅ | ✅ | W4 |
| Edit track, cover, delete, visibility | `PUT /music/tracks/{id}*` | ✅ | ✅ | W4 restyle |
| Playlists: create, edit, cover, add/remove, **drag reorder**, visibility | `/music/playlists/*` | ✅ | ✅ (+ add song/album to playlist) | W4 |
| Identify (MusicBrainz) single + batch by album/artist | `/music/tracks/{id}/metadata/search|apply` | ✅ | ✅ | W8 |
| Lyrics (embedded tag), edit lyrics | `/music/tracks/{id}/lyrics` | ✅ | ✅ | W8 |
| Duplicates review (exact tier) | `/music/duplicates` | ✅ | ✅ | W8 |

### Family, billing, stats, notifications

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| Family: name, photo, members list, my role | `GET/PUT /family`, `/family/members` | ✅ | ✅ | W6 |
| Invites: email / link / QR / claim-account; list, revoke | `/family/invites*`, provisioning endpoint | ✅ | ✅ | W6 |
| Member access: per-kind policy, per-item grants (bulk replace), block/remove | `/family/members/{id}/access|policy|grants` | ✅ | ✅ (+ managed member's stats visibility) | W6 |
| "Who can hear this" audience | `/family/content/{kind}/{id}/audience` | ⬜ | ✅ | W6 |
| Leave family | `DELETE /family/members/{me}` | ✅ | ✅ | W6 |
| Billing: storage by kind, credit, days left, burn rate | `GET /family/billing` | ✅ | ✅ | W6 |
| Credit alerts (admin) | `/family/billing/alerts` | ✅ | ✅ | W6 |
| Top-up via Stripe checkout (dormant until secrets set) | `/billing/topup`, `/billing/success|cancel` | ✅ | ✅ (dormant) | W6 (gate on `providers`/config) |
| Stats: ranges, totals, streak, by kind, top items, history; visibility | `/stats/me*` | 🚧 widget only | ✅ | W5 |
| Family stats (admin) | `/stats/family*` | ⬜ | ✅ | W5 |
| Listening session reporting (`tz_offset_minutes`, explicit only) | `POST /playback/sessions` | ✅ | ✅ | W3 — required or stats are empty for web listening |
| Notifications inbox, poll on focus, ack, unread badge | `/devices/notifications*` | ✅ | ✅ | W5 |
| Playback settings (audiobook defaults; re-read after save, server clamps) | `/playback/settings*` | ✅ | ✅ | W3 |

### Creation and admin

| Feature | API | Mac | Web now | Phase |
|---|---|---|---|---|
| AI narration wizard + status | `/audiobook-gen/*` | ✅ | ✅ | W8 restyle |
| Instance admin: users, jobs | `/users`, `/jobs` — gated on `users.role == admin` | 🚧 | ✅ | W7 (add jobs) |
| Family-admin gating on `my_role` from `GET /family`, never instance role | — | ✅ | ✅ | W6 |

---

## 5. Phases

**Executor: read §5a first** — the conventions there were learned by breaking
them, and two of them are invisible to the type checker.

Each phase ends the same way: `npm run build` (tsc + vite) clean, `npm run
lint` clean, the phase's own checks done by hand against the local backend
(`docker compose up`, `admin@audio2.local` / `admin`), light **and** dark
looked at, a `CHANGELOG.md` line under the frontend heading, then a commit by
the user. No phase claims a feature the backend doesn't have (§12 of the
client guide is the known-gaps list).

### ✅ W0 — Design system and shell (done 2026-08-27)

- `src/styles/tokens.css` (light/dark/system, all colour/radius/motion
  tokens) + `@theme` wiring; `ThemeToggle`; drift test vs `design-tokens.json`.
- Primitive components: `Button`, `IconButton`, `Input`, `Select`, `Menu`,
  `Dialog`/`Sheet`, `Toast`, `Badge` (family/favorite only), `Cover`
  (kind-tinted placeholder), `EmptyState`, `Skeleton`, `SegmentedControl`,
  `DataTable` (sortable, multi-select). Headless via Radix primitives for
  focus/keyboard/a11y — no styled component libraries.
- `AppShell` → `Sidebar` + `ContentColumn` + `DetailColumn` + `PlayerBar`
  slots; responsive breakpoints (3-col ≥ 1200, 2-col ≥ 900, single below).
- Router restructured around the new shape; old pages mounted temporarily so
  nothing regresses while W1–W8 replace them one by one.
- Delete the dead `/audiobooks/sharing` nav item.

**Verify:** every existing page still loads inside the new shell; toggle
theme; keyboard-navigate the sidebar.

**Done as written, with two deviations:** `DataTable` was not built — no
new page needs it until W2/W4, and an unused primitive is the speculative
abstraction the repo rules forbid. The 1024 px breakpoint (Tailwind `lg`)
replaced the planned 900/1200 pair; below it the sidebar becomes a drawer
behind a top bar. Verified headlessly (Playwright on the installed Chrome,
`npm test`/`npm run build` green) on Home, Music with a track playing, the
account menu, and a 420 px viewport, in both themes. The pre-existing pages
still carry their own dark-only `gray-*` styling and look wrong in light
mode until their phase replaces them — expected, not a regression.

### ✅ W1 — Auth and onboarding (done 2026-08-27)

Restyle setup wizard, sign-in, register, Google button; add Apple button
when `GET /auth/providers` advertises it. Session expiry → sign-in with a
return-to. `device_kind: "web"` on login — already sent by the console and
accepted by all four allowlists (verified 2026-08-27).

**Done.** User's decision: password + Google + Apple, all three. Apple on
the web needed a contract addition — `apple.web_client_id` (Services ID)
on `/auth/providers`, since the JS SDK can't work from `enabled` alone —
landed in the backend with spec docs. The button stays hidden until the
Apple developer account exists and `AUTH__APPLE__WEB_CLIENT_ID` is set; the
popup flow's redirect URI (`<origin>/auth/login`) must be registered
against that Services ID at that point. Setup wizard lost its separate
"welcome" step (two landing pages in a row). Verified: screenshots light/
dark/mobile, and a headless sign-in from `?next=/music` landing on
`/music`.

### ✅ W2 — Browse (all three kinds) + Home (done 2026-08-27)

Content column per kind with grid ⇄ list, sort, search; Music gets
Artists/Albums/Genres/Playlists modes and the detail column; Audiobooks gets
grouped-by-author; Podcasts feed detail with paged episodes. Home with
Continue Listening, recent songs/albums/playlists, widget picker
(localStorage). ⌘K global search over `/library/search`.

**Done.** Two things found by driving it rather than reading it:

1. **`SplitView` built its width class at runtime** (`"w-[420px]".replace(...)`
   → `lg:w-[420px]`). Tailwind scans source *text*, so that class never
   existed and the detail column never got a layout. Column widths are now
   three literal classes (`narrow`/`medium`/`wide`). **Never assemble a
   Tailwind class from a variable anywhere in this app.**
2. **A backend bug**: `GET /playback/{books,episodes}/{id}/progress` returned
   `401 "account not found"` when a user had no saved position — the normal
   first-play case, one per book on the library screen. Fixed to `404`
   (`AuthError::ItemNotFound`); spec + CHANGELOG updated. ~90 other
   `AuthError::NotFound` sites across `music/`, `audiobooks/`, `families/`,
   `users/` have the same smell and were **not** swept — see the CHANGELOG
   entry.

### ✅ W3 — Playback (done 2026-08-27)

**Goal:** one player that behaves correctly for all three kinds, reports
listening sessions, and syncs the queue across devices.

**Files.** Replace `src/components/player/PlayerBar.tsx` (currently the pre-W0
logic, restyled). Delete `src/components/audiobook/AudiobookPlayerBar.tsx` and
`src/components/audiobook/BookmarkPanel.tsx` and fold what they do into the new
player — `AppShell` currently swaps between two whole player bars, which is why
audiobook and music behaviour drifted apart. New: `src/lib/mediaSession.ts`,
`src/lib/sessionReporter.ts`, `src/api/sessions.ts`, `src/pages/settings/
PlaybackSettings.tsx`.

**Work items**

- [x] One `PlayerBar`, kind-aware rather than one component per kind:
      - audiobook: speed 0.5–3× (persisted, and seeded from
        `GET /playback/settings`'s `ab_playback_speed`), skip ±
        (`ab_skip_forward_secs` / `ab_skip_backward_secs`), sleep timer,
        bookmarks (add at position, list, rename, delete — `/playback/bookmarks*`),
        previous/next file, chapter list when the book has real chapters.
      - podcast: same transport minus bookmarks; the **served-download gate**
        already lives in `lib/play.ts` (`playEpisode`).
      - music: queue panel, shuffle, repeat, volume.
- [x] **Media Session API** (`navigator.mediaSession`): metadata (title,
      artist, artwork) + `play`/`pause`/`previoustrack`/`nexttrack`/`seekto`
      handlers, so OS media keys and the lock screen work. Artwork needs a
      real URL — audiobook and music covers require the auth header, so pass a
      blob URL made the way `AuthImage` makes one, not the API URL.
- [x] **Stream URLs are resolved at play time and never persisted** (~4 h
      expiry). Already true in `lib/play.ts`; keep it that way, and on a
      storage 403 mid-playback re-resolve once and retry rather than failing.
- [x] Progress saves every ~20 s plus on pause, track change and teardown.
      The current 10 s interval is twice the traffic for no benefit; match the
      Mac's cadence.
- [x] **`POST /playback/sessions`** — batched, with `client_session_id` (a UUID
      generated when the span closes and reused on every retry: `recorded: 0`
      on a retry is success, not failure), `seconds_listened` = audio consumed
      not wall clock, `playback_speed`, `device_kind: "web"`,
      `tz_offset_minutes`. Accumulate spans and flush on pause/unload; do not
      post per span. **Reporting explicitly switches server-side derivation
      off for that user — do not also rely on progress-derived sessions.**
- [x] Cross-device queue: `GET /playback/queue` on load, `PUT` on change, with
      `device_kind`/`device_label`. Last write wins; `updated_at` tells you
      you were overtaken. The Subsonic surface shares this queue.
      **Completed 2026-08-27.** Read on load and on focus, an explicit
      "Continue here" that rebuilds the queue and resumes at the handed-off
      position, and the position published back on pause. `device_label` is
      still deliberately **not** sent: the contract says omit it unless the
      user actually named the device, and a browser has no such name —
      inventing "Chrome on macOS" is the raw-model-id behaviour that field
      warns against. Fixing this turned up that podcast items were being
      written without their feed id, which made them unplayable elsewhere.
- [x] Playback settings pane (`GET /playback/settings`,
      `PUT /playback/settings/audiobook-defaults`) — **re-read after saving**,
      the server clamps (skip 1–120 s, speed 0.5–3×).
- [x] Mark played / unplayed per episode, and "mark all played" per feed via
      `POST /playback/episodes/progress/bulk` (the only bulk endpoint there is).

**Verify:** listen 20 s on web, then check the session appears in the Mac's
history attributed to `web`; resume position round-trips web ↔ Mac; media keys
work; a 4-hour-old tab can still start playback.

**Done.** Verified in a real browser: playback runs, `POST /playback/sessions`
lands in `/stats/me/history` as `source: reported, device_kind: web`, the
queue PUT fires, Media Session carries title/artist/blob artwork and
`playbackState`.

**One thing found by watching the traffic:** the first pause of a
never-reported-before account produced **two** history rows for the same
span — one `reported`, one `derived`. The backend guard is correct and
self-healing (`db::stats::uses_explicit_sessions`: any reported row in the
last 7 days switches derivation off), but the web app was saving progress
*before* flushing its first session report, so that one save still derived.
Pause now flushes sessions first, then saves progress. Anything else that
starts reporting sessions later — iOS, Android — will hit the same one-time
double count unless it does the same.

### ✅ W4 — Ingest and management (done 2026-08-27)

**Goal:** everything that changes library data, and the multi-select that makes
it bearable at scale.

**Files.** Grow `src/pages/audiobooks/UploadBookDialog.tsx` and
`src/pages/music/UploadMusicDialog.tsx` into a shared upload queue
(`src/components/library/UploadQueue.tsx` + `src/lib/uploads.ts`). New edit
sheets per kind; delete `src/pages/audiobooks/EditBookModal.tsx` and
`src/pages/music/EditTrackModal.tsx` (pre-W0 styling). Rebuild
`AuthorsPage`/`AuthorDetailPage`/`CollectionsPage`/`CollectionDetailPage` onto
`SplitView` and drop their `Legacy` wrappers in `App.tsx`.

**Work items**

- [x] Upload queue: concurrency-limited (4 is what the audiobook flow uses),
      per-item client-side progress, survives navigating between sections,
      and **says plainly that a failed upload restarts from zero** before a
      multi-GB attempt.
- [x] Audiobooks: presigned flow (`/uploads/presign` → PUT →
      `/audiobooks/from-uploads`) is already in place — keep it; multipart is
      only for small files and local instances (100 MB proxy cap when hosted).
      **`relative_path` decides final track order, not send order.**
- [x] Edit sheets: book (title/author/narrator/description, cover, file
      reorder via `PUT /audiobooks/{id}/files/reorder`, delete), track
      (title/artist/album/genre/track number, cover), feed (visibility,
      unfollow), playlist (name/description/cover).
- [x] Playlists: create, add/remove, **drag reorder** (`dnd-kit`) writing
      `PUT /music/playlists/{id}/tracks/reorder` with the full entry-id order,
      cover upload.
- [x] Authors (CRUD + roles), tags, collections, series, favorites — all the
      `/audiobooks/organize/*` and `/audiobooks/authors/*` endpoints already
      wrapped in `src/api/`.
      All built; authors CRUD, per-book roles and tags landed 2026-08-27.
- [x] Visibility private ⇄ family per item. **No lock iconography anywhere** —
      private is the unmarked default, family-shared gets the people badge.
- [x] Multi-select in every list/grid with a batch action bar: change
      visibility, delete, add to collection/playlist. **Every batch is N
      client-side requests** (no bulk endpoints exist beyond episode
      progress) — run them with a concurrency limit and report per-item
      failures rather than a single "it failed".
      Built for audiobooks, podcasts and the music detail column, with
      add-to-playlist (music) and add-to-collection (audiobooks, 2026-08-27)
      as batch actions. The Organize lists still have none — they are
      groupings rather than content, so a batch there would mean something
      different.

      **Both add-to-X batches run one at a time, for different reasons.** A
      playlist *rejects* parallel adds (`UNIQUE (playlist_id, position)`), so
      they silently go missing. A collection accepts them — its key is
      `(collection_id, book_id)` — but every book reads the same
      `MAX(position)` and lands on the same one: six books added four at a
      time all took position 1. Nothing is lost there, but the chosen order
      is. Measured both ways; 25 books added sequentially land 1–25.

**Verify:** make 12 books family-visible in one action with one forced failure
(kill the network mid-run) → the other 11 land, the failure is named, nothing
is silently skipped. Drop a 20-file book folder → correct order server-side.

**Done**, with the batch machinery built to that shape (`runWithLimit` +
`BatchBar` collect failures instead of aborting) — though the forced-failure
run above was **not** performed; the happy path was driven in a browser and
the failure path is covered by construction only. Worth doing by hand before
W9 signs off. Two presentation bugs found while driving it: a mode switcher
clipped inside a fixed-width column (segmented controls now scroll), and the
filter field was invisible in light mode (bg-alt on bg-alt).

### ✅ W5 — Stats and notifications (done 2026-08-27)

**Files.** `src/pages/stats/StatsPage.tsx`, `src/api/stats.ts` (extend —
`listHistory` already exists), `src/components/shell/NotificationsPanel.tsx`.
Add a Stats item to the sidebar under Library, and a bell with an unread count
next to the account menu.

- [x] `GET /stats/me?range=7d|30d|365d|all&tz_offset_minutes=` — **always send
      the offset** (minutes east of UTC) or day buckets and the streak are
      computed in UTC and look wrong. Render totals, `by_kind`, `by_day` (bar
      chart), `top_items`, `streak_days`, `completed_items` (lifetime, ignores
      range — label it as such).
- [x] History log with paging (`/stats/me/history?limit=&offset=`), showing
      `device_kind` and whether the row was `reported` or `derived`.
- [x] Visibility toggle: `PUT /stats/me/visibility`
      (`private` | `family_admin`).
- [x] `GET /stats/family` for family admins — members who keep stats private
      come back `hidden: true` with **no figures**: render the row, omit the
      numbers, don't invent a zero.
- [x] Notifications inbox: `GET /devices/notifications`, ack on open
      (`POST /devices/notifications/ack`), polled on window focus.
      **There is no push delivery server-side** — polling is the design, not a
      stopgap; don't add a service worker for it.

**Done.** One CSS bug found by looking rather than reading: the by-day chart
rendered no bars at all, because a percentage height resolves against the
parent and the per-bar wrapper had no height of its own. `src/api/family.ts`
was created here (not W6) because the family-stats section needs `my_role`.

### ✅ W6 — Family and billing (done 2026-08-27)

**Files.** `src/api/family.ts` (**does not exist yet** — the whole family
surface is unwritten on web), `src/api/billing.ts`, `src/pages/family/*`,
`src/pages/billing/*`, `src/pages/join/JoinPage.tsx` (public route).

- [x] Family page: name, photo, member list with roles, my role.
      **Gate every admin control on `my_role` from `GET /family`, never on
      `users.role`** (that is the instance admin, a different thing).
      Members, roles, gating, the editable family name and photo, and member
      avatars are all done (the last three added 2026-08-27).
- [x] Invites: create/list/revoke (`/family/invites*`), all four kinds from
      `audio2/docs/family-join-qr-plan.md` — email, link, QR, claim-account.
      Render the QR client-side (no dependency needed for a QR of a short
      code; the Mac draws its own). Codes are bearer tokens: never log them,
      never put them in a URL you also send somewhere else.
- [x] `/join/:code` — **public route, works logged out**, spec is that plan's
      Phase B (four states: valid+logged-out → register with the code;
      valid+logged-in → one confirm button; `kind: claim` → set a password for
      an account an admin created; expired/exhausted/404 → plain explanation).
      End inside the app with a small welcome toast, not a dead-end page.
- [x] Member access editor: per-kind policy and per-item grants
      (`/family/members/{id}/policy|grants`) — **grants are a bulk replace of
      the whole set**, not incremental adds. Block/remove/leave.
      Block, remove, "leave this family" and the "who can hear this" audience
      view (`/family/content/{kind}/{id}/audience`) are all built.
      **Built 2026-08-27 at the user's request**, after W6 shipped without it:
      age brackets and the upload/generate permissions from
      `audio2/docs/family-permissions-plan.md` (child forces both off, matching
      the server's CHECK), plus the content policy and its exceptions.
      Affordance gating on `my_can_upload`/`my_can_generate` came with it.
- [x] Billing: `GET /family/billing` (storage by kind, credit balance, days
      left, burn rate), alerts for admins (`/family/billing/alerts`), top-up
      via `POST /billing/topup` → Stripe checkout. **Payments are dormant
      until secrets are set** — hide the top-up button unless the server says
      it is available, and never imply a charge that can't happen.
- [x] Home widget: storage & credit (the widget board is already built, add
      one entry to `src/pages/home/widgets.ts` and `WIDGETS`).

**Done.** Three things worth recording:

1. **The admin role value is `"family_admin"`, not `"admin"`.** Checking for
   `"admin"` type-checks, renders, and silently hides every management
   control — which is exactly what the first build of this page did. There is
   now an `isFamilyAdmin()` helper in `src/api/family.ts`; use it rather than
   comparing strings.
2. **A hand-written QR encoder produced unreadable codes.** Replaced with
   `qrcode-generator`, and there is now a test that decodes the rendered SVG
   with an independent decoder. Anything that exists to be scanned needs that
   test, not an eyeball.
3. `timeAgo` on an expiry rendered "expires -4798m from now". `formatUntil`
   in `src/lib/time.ts` is the forward-looking one, with tests.

### ✅ W7 — Account, devices, admin (done 2026-08-27)

- [x] Profile (`PATCH /users/me`) and photo upload.
- [x] Change password — say up front that it **signs out every other device**.
- [x] Devices/sessions (`GET/DELETE /auth/sessions`): current device sorted
      first and **never revocable from here**.
- [x] Subsonic key: show, regenerate, and explain what it is for
      (third-party Subsonic players, music only).
- [x] Delete account: typed confirmation, states plainly that it cannot be
      undone.
- [x] Instance admin (`users.role === "admin"`): users list (`GET /users`),
      background jobs (`GET /jobs`). Rebuild `AdminPage` on tokens; it is the
      last `Legacy`-wrapped page besides Settings.

**Done.** The `Legacy` wrapper now holds only the two AI-narration pages,
which W8 takes.

### ✅ W8 — Creation and music tools (done 2026-08-27)

- [x] Restyle the narration wizard (`/audiobook-gen/*`) and its status page.
      Keep the quote → confirm → job flow; **narration is charged against
      family credit**, and a failed job cannot be retried in place (only
      resubmitted).
- [x] Episode translation: already functional, restyle onto tokens. Only
      offered where `has_transcript` is true — there is no speech-to-text.
- [x] Identify (MusicBrainz): `POST /music/tracks/{id}/metadata/search` →
      candidate list → `apply`. Cover art URLs in candidates are
      **speculative and may 404** — a broken image is not an error. Batch
      identify for an album and an artist.
      Single-track and batch (album / artist / selection) both built; batch
      landed 2026-08-27 with a confirm step rather than an automatic apply.
- [x] Lyrics: `GET /music/tracks/{id}/lyrics` (embedded tag), view + edit.
- [x] Duplicates review: `GET /music/duplicates` (exact tier, SHA-256 groups,
      scoped to the caller's own tracks).

**Done.** `/music/duplicates` returns an **envelope** (`{ groups: [...] }`),
not a bare array — typing it as an array compiled fine and crashed the page
at runtime. Worth checking the actual response shape rather than inferring it
from a handler's name. The `Legacy` route wrapper is now gone entirely.

### 🟡 W9 — Polish, a11y, ship (2026-08-27 — open: Lighthouse number, deployment)

- [x] Keyboard: space, ←/→ seek, shift+←/→ track, ⌘K, focus rings everywhere,
      full tab order through sidebar → content → detail → player.
- [x] `prefers-reduced-motion` respected (the token file already has the
      global rule — check nothing overrides it with an inline animation).
- [x] Empty, loading and error states for every screen; long titles, missing
      art, a feed with 1000+ episodes, a book with 200 files.
      Exercised 2026-08-27 against a synthesised 1200-episode feed with
      pathological titles: 800ms to first row, 1206 rows rendered, 1.2s to
      jump to the end, zero horizontal overflow. A 200-file book was **not**
      tried — no such book exists to test with, and the file list is the same
      component as the episode list.
- [~] Lighthouse accessibility ≥ 95; both themes audited by eye, not just by
      token.
      **An axe-core sweep was run instead of Lighthouse** — zero serious or
      critical violations across every route in both themes, signed in and
      out. That is a stronger check for correctness than Lighthouse's score,
      but it is not the number this box asks for.
- [ ] `docs/production-deployment.md` §1b still describes reality
      (`app.own.audio` served by the backend's `ServeDir("ui/dist")`).
      **Not verified — nothing has been deployed.**
- [x] Roadmap Phase 2 flipped in **`audio2-www`** — `docs/roadmap.md` and
      `src/pages/roadmap/index.astro` in the same change, per that repo's
      CLAUDE.md §6a. Verify against what actually works before ticking.
      **Left uncommitted in that repo** — it is the public site, and the
      standing rule there is to prepare the change and let the user commit.
      The wording says "built and verified", not "shipped": none of this is
      deployed.

**Done, and the sweep found three real defects** — see the CHANGELOG entry.
The one worth carrying to other work: an interactive wrapper around
interactive children is invisible to a screen reader, and it type-checks
perfectly. Only axe caught it.

**Still outstanding for a genuine ship:** deploy to canary and check
`docs/production-deployment.md` §1b end to end, and run the forced-failure
batch test W4 recorded as unperformed.

---

## 5a. Conventions an executor must follow

Learned in W0–W2; violating any of these has already caused a bug here.

1. **Never assemble a Tailwind class from a variable.** Tailwind v4 scans
   source text. `` `lg:w-[${n}px]` `` or `"w-" + size` produces no CSS.
   Use a lookup object of literal class strings (`SplitView`'s `COLUMN_WIDTH`).
2. **Only tokens for colour.** `bg-card`, `text-muted`, `border-border`,
   `text-accent`, `bg-book/10` — never `gray-800`, `indigo-600`, `#fff`. The
   token list is `src/styles/tokens.css`; `npm test` guards it against
   `audio2-android-book/docs/design-tokens.json`.
3. **Both themes, always.** Nothing gets a colour that only exists in one.
4. **No lock icons, no security language.** "Private" means "not shared with
   the family". Badges: family, favorite. Private gets none.
5. **Stream URLs resolve at play time**, never stored, never given an
   `Authorization` header.
6. **Cover art auth differs by kind**: audiobook and music covers need the
   bearer token (`AuthImage`); podcast art is public (plain `<img>`).
   `Cover`'s `auth` prop defaults correctly per kind — don't override it.
7. **404 means "missing or not visible to you"**, deliberately. Never
   distinguish them in copy.
8. **New API calls go in `src/api/<domain>.ts`**, typed against
   `src/api/types.ts`, and are used through TanStack Query with a stable key.
   Invalidate the keys a mutation actually affects.
9. **Comments explain WHY.** Default to none. If deleting it wouldn't confuse
   anyone, delete it.
10. **Per phase:** `npm run build` (tsc + vite) and `npm run lint` clean for
    new code, `npm test` green, then drive the real app against
    `docker compose up` in `audio2/` (`admin@audio2.local` / `admin`) — the
    two bugs found in W2 were both invisible to the type checker. Then a
    `CHANGELOG.md` entry and a commit.

**Pre-existing lint noise:** `AudiobookPlayerBar.tsx`, `BookmarkPanel.tsx`,
`AdminPage.tsx`, `SettingsPage.tsx` still carry pre-W0 errors. They are not
regressions; each disappears when its phase rewrites the file. Don't "fix"
them in place — W3 deletes the first two outright.

---

## 5b. What is not built (audit, 2026-08-27)

The phase headings above were ticked before their sub-items were, which made
the plan read as more finished than it was. Every box has since been checked
against the code. What remains open, smallest first:

| Gap | Phase | Notes |
|---|---|---|
| Multi-select on the Organize lists | W4 | Deliberate — they are groupings, not content; see W4's note. |
| A 200-file book | W9 | The 1200-episode equivalent was exercised; no 200-file book exists to test with. |
| Lighthouse ≥ 95 | W9 | An axe sweep was run instead — zero serious/critical across every route in both themes. Stronger for correctness, but not this number. |
| Forced-failure batch run | W4 | The machinery collects per-item failures by construction; only the happy path was driven. |
| Canary deploy + `production-deployment.md` §1b | W9 | Nothing is deployed. |

None of these block the app being usable end to end; they are the difference
between "works" and "finished".

### Endpoint gaps closed 2026-08-27

A second pass diffed every endpoint in `docs/android-client-guide.md`'s index
against `frontend/src/api/*.ts`. Thirteen were never called; nine were false
positives (cover and image URLs arrive as `cover_url`/`image_url` fields,
`POST /auth/admin-create-user` is called inline from `AdminPage`) or do not
apply (`/devices/push-token*` — there is no web push server-side). The other
four are now built:

| Endpoint | What was missing |
|---|---|
| `POST /auth/refresh` | The client discarded the refresh token login returns, so a session ended when the access token expired. Refreshes are single-flight (`lib/singleFlight.ts`): the server treats a replayed single-use token as a leak and revokes the whole device chain, so two parallel refreshes would sign the user out everywhere. Verified against the live server — replaying a spent token 401s *and* kills the token issued alongside it. |
| `GET /library/private` | No "Just me" view. Now `/private/`, grouped by kind. |
| `GET /audiobook-gen/cover-prompt` | The wizard sent no title, author or cover field at all. New Cover step; the custom brief is seeded from this endpoint so it starts from the generator's own default. |
| `PUT /stats/family/members/{id}/visibility` | No UI. In the member access sheet, shown only for a member carrying a `deny_all` policy — the server answers 403 for an unrestricted adult (confirmed), so a parent cannot quietly start watching another grown-up. |

## 6. Open questions

**Settled 2026-08-27 with the user:** Q1 purple (update `design-tokens.json`
and the app mirrors in a separate change), Q2 stay in `audio2/frontend`,
Q3 Connected Servers out of scope, Q4 nothing to do, Q5 desktop-first with
a usable single column, Q6 Lucide. Q7 still open — batch ops get built on
web in W4 unless told otherwise.

1. **Accent colour.** Brand brief says purple `#6E44FF` is the core and
   "stays that way"; `design-tokens.json` (the declared source of truth for
   the *apps*) says `brand.primary = #0071E3` blue, and the Mac/iOS/Android
   apps use that. Recommendation: **purple for the web app**, and update
   `design-tokens.json` + the app mirrors in a separate change so the
   product and the site stop disagreeing. Alternative: keep the apps blue
   and accept the web app matching the apps rather than the site.
2. **Where it lives / hostname.** Stay in `audio2/frontend`, served by the
   backend at `app.own.audio` (current setup). Recommendation: yes — no new
   repo, no new deploy pipeline. The alternative (separate Cloudflare Pages
   project) only helps if we want to deploy the UI independently of the API.
3. **Connected Servers in the browser.** Out of parity scope until the
   backend can proxy Audiobookshelf/Subsonic (a real backend feature with
   its own plan). Confirm that's acceptable for Phase 2.
4. ~~`device_kind` for the web~~ — settled: the console sends `"web"` and
   both Rust allowlists and both CHECK constraints accept it. Nothing to do.
5. **Mobile browsers.** Parity plan targets desktop-width first with a
   usable single-column fallback. Is a phone-first layout wanted now, or is
   that what the iOS/Android apps are for? Recommendation: usable, not
   optimised — the phone apps are Phases 3–5.
6. **Icon set.** Lucide (MIT) vs hand-drawn set from
   `audio2-www/docs/icon-system.md`. Recommendation: Lucide for UI chrome;
   the brand's own icons only for the three clouds and the mark.
7. **Batch operations.** Mac's P8 (multi-select batch) isn't built there
   yet. Build it on web first (W4) and let the Mac follow? It's the first
   time web would lead rather than follow.
