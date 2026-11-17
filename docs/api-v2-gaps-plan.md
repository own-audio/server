# /api/v2 gaps — what the clients still need from the server

Written 2026-11-17 from the clients' migration plans (`API_V2_PLAN.md` in `audio2-mac`,
`audio2-tvos`, `audio2-win`, `audio2-sync`, `audio2-android-book`, `audio2-android-music`,
`audio2-android-podcast`). Each line is a v1 call some client makes that has no v2 counterpart
yet. Lines marked (audit) come from the full v1 → v2 audit of the same day
(`api-v1-v2-coverage.md`): v1 functions no client plan named. Clients keep calling v1 for these, with a comment, until the line is done
(`api-v2-first.md`). Letters in brackets: who needs it — **M**ac, **T**V, **W**indows,
**S**ync core, **A**ndroid (b/m/p = book/music/podcast).

## 0. Contract first

- [x] **v2 is the main API and counts its revisions** (Kornel, 2026-11-17): revision 1,
      additive only from now on; discovery's v2 revision goes up with each line below so a
      client can check for it. It stays `draft` until the release that completes
      `api-v2-completion-plan.md`, then becomes stable. (M T W S A)
- [x] **Stable images** (v2 revision 2): catalog art is linked for good; people's pictures keep
      a weekly expiry. Was: `image` links are signed and expire in 1–2 weeks; `/sync/changes`
      doesn't resend unchanged records, so links kept in a cache, a widget or the car go dead.
      Either long-lived by-id image routes or a refresh rule clients can follow. (A, W, T)
- [x] **Auth error shapes** (written into `api-v2-first.md`, 2026-11-17). Wrong credentials are `invalid_grant` on `/auth/token` but other
      v2 auth errors answer 401 `invalid_credentials`; a dead refresh token is 400
      `invalid_grant` (not v1's 401). Document it in `api-v2-first.md` and keep it. (Ab Am Ap)
- [x] **A v2 home for `/api/v1/media`** (v2 revision 2: `/api/v2/media`). v2's playback and upload links for local storage are
      `/api/v1/media` links, so removing v1 would break v2 on servers without object storage.
      (audit)

## 1. Blocking — a client phase can't finish without it

- [x] **Podcast feeds** (v2 revision 4): follow (`POST /podcasts` with `feed_url`), unfollow
      (`DELETE /podcasts/{id}`), refresh (`POST /podcasts/{id}/refresh`, `POST /podcasts/refresh`).
      (M T W Ap)
- [ ] **File sync records:** `path` on track, book, episode; `relative_path` and `sha256` on
      book files; `sha256` on tracks and episodes; also in the create answers. (S, so M W)
- [ ] **Change log triggers** for `sync_paths` (renames) and `content_grants`. (S)
- [ ] **Companion files** as a sync kind with create, visibility, delete, download. (S)
- [ ] **`/sync/changes?kinds=`**, and an ids-only snapshot for the reconcile. (S, A)
- [ ] **A `member` record kind** for owner names, including people who left. (S)
- [ ] **Continue listening in one read:** `include=` on `GET /me/progress` (or a batch lookup).
      (T Ap)

## 2. Account and sign-in

- [x] Register with an invite (v2 revision 3: `POST /auth/register` with `invite_code`). Join
      links and a join preview before sign-in move with the household (completion plan phase 7).
      (W Ab Am Ap)
- [x] `PATCH /me` (display name, `discovery_languages`, `recommendations_enabled`); profile picture
      (`PUT`/`DELETE /me/image`) (v2 revision 3). Skip intervals belong to the listening settings
      (phase 8). (M W A T)
- [x] Password change, `DELETE /me`, two-factor setup and disable, security events, Subsonic
      key, verification e-mail resend (v2 revision 3). (M W Ab)
- [x] Desktop Google/Microsoft: the loopback code + PKCE flow as the `authorization_code` grant,
      and the desktop client ids in discovery (`auth.clients`) (v2 revision 3). (W, M)
- [x] Setup state and the demo account in `/.well-known/own-audio` (`setup`, `demo`; v2
      revision 3). (W Ab Am)
- [x] Sign-up without an invite (`POST /auth/register`; v2 revision 3). (audit)
- [x] Forgot and reset password, and the e-mail verification link's landing call
      (`POST /auth/password-reset`, `…/confirm`, `POST /auth/email-verification`; v2 revision 3).
      (audit)
- [x] Whether two-factor is on: `two_factor` in `GET /me` (v2 revision 3). (audit)
- [x] A second session from a refresh token for a helper process — the Mac's Finder
      extension: the token-exchange grant (RFC 8693) on `POST /auth/token` (v2 revision 3). (audit)
- [x] The Google and Apple web client ids in discovery (`auth.clients`; v2 revision 3). (audit)

## 3. Podcasts

- [x] Directory (v2 revision 4): `/podcasts/directory/search`, `…/categories`,
      `/podcasts/directory?category=`, `…/similar`, `…/preview`. (M T Ap)
- [x] Unplayed counts (v2 revision 4: `GET /me/podcasts/unplayed`). (M T)
- [x] Keep an episode on the server / drop the server's copy / store-all / auto-store (v2
      revision 4: `PUT`/`DELETE /episodes/{id}/stored`, `POST /podcasts/{id}/store`,
      `PATCH /podcasts/{id}`). (M Ap)
- [ ] Episodes, books and podcasts in `/search` (music only today). (T W)
- [ ] Translations as a v2 kind for `playback-info` and progress. (Ap)
- [x] Transcripts as text (v2 revision 4: `GET /episodes/{id}/transcript`, timed segments with
      speakers). (audit)
- [x] Storing a show's and its episodes' art (v2 revision 4: part of the refresh). (audit)

## 4. Music

- [ ] Track feedback (dislike/ban) — or fold it into ratings. (M T W)
- [ ] Delete a track (and a book, a podcast). (M W Am)
- [ ] Lyrics write; file tags and rescan; the duplicates list; album identify. (M W Am)
- [ ] Cover uploads: track, playlist, artist picture. (M W Am)
- [ ] Playlists in `/sync/changes` (not blocking: read when shown). (M W Am)
- [ ] Playlists shared with chosen members, live or as copies, and who can play one
      (`share`, `audience`); keeping a generated playlist (`keep`). v2 shares with the whole
      household only. (audit)
- [ ] Artist and author pictures: forget one and look it up again; the fetched picture's
      credit and licence (`image-info`). (audit)
- [ ] A sentence to a smart-playlist rule (`/music/intent`). (audit)
- [ ] The disc-number back-fill for old uploads (`tracks/discs`), or a server job instead.
      (audit)

## 5. Books

- [ ] `PATCH /books/{id}` (metadata), file rename and reorder, replace the cover, metadata
      search and apply. (M W Ab)
- [ ] `POST /books/{id}/files` (add files to a book). (S)
- [ ] Series and collections as writable resources; author create and delete. (W Ab)
- [ ] Delete one file of a book. (audit)
- [ ] Book tags (list, read, replace). (audit)
- [ ] Author photo upload; `bio` in `PATCH /contributors/{id}`. (audit)

## 6. Listening settings

- [ ] The cross-device queue. (M W Ab Ap)
- [ ] Playback settings and audiobook defaults; EQ. (M T W A)

## 7. Household and the rest

- [ ] Block/unblock, provisioned members and claim codes, invite regenerate, member and family
      pictures, content reports. (M W Ab)
- [ ] Per-member policies, grants and audience (v1 by decision so far). (W Ab)
- [ ] Stats; notifications inbox; push devices. (M W A)
- [ ] Trash with batches, restore, purge, holdings; shortcuts (album shortcuts by `album_id`).
      (S M)
- [ ] Instance admin: users, jobs. (W)
- [ ] Instance admin, the rest: families (list, detail, trash, lock, unlock, delete), server
      stats, create a user, revoke a user's sessions, finish setup; hosted: grant credit.
      (audit)
- [ ] A member leaving the household themselves: v2's `DELETE /family/members/{id}` is for
      admins only. (audit)
- [ ] Family settings: audio analysis, book source. (audit)
- [ ] Library folders and their scan. (audit)
- [ ] Moving items to their default folder paths (`/sync/paths/organise`); a companion image
      as cover (`use-as-cover`). (audit)
- [ ] An owner or visibility filter on lists, for a "private" section (v1 `/library/private`).
      (audit)
- [ ] Hosted billing, narration and translation: v2 bodies (they are v1's under `/api/v2`).
      (W A)

## Order

0 first (it decides how clients check for the rest), then 1 in the order the client plans
need it — podcast feeds and the sync records unblock the most — then 2–7 as clients reach
them. Each line lands with its `openapi-v2.json` change, conformance tests and a CHANGELOG
line, like every v2 phase.
