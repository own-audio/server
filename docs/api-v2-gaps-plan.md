# /api/v2 gaps — what the clients still need from the server

Written 2026-11-17 from the clients' migration plans (`API_V2_PLAN.md` in `audio2-mac`,
`audio2-tvos`, `audio2-win`, `audio2-sync`, `audio2-android-book`, `audio2-android-music`,
`audio2-android-podcast`). Each line is a v1 call some client makes that has no v2 counterpart
yet. Clients keep calling v1 for these, with a comment, until the line is done
(`api-v2-first.md`). Letters in brackets: who needs it — **M**ac, **T**V, **W**indows,
**S**ync core, **A**ndroid (b/m/p = book/music/podcast).

## 0. Contract first

- [ ] **Decide when v2 stops being `2.0-draft`.** The iPhone apps ship on it; every plan asked.
      From then on the additive rule applies, and discovery's v2 revision goes up with each
      line below so a client can check for it (it is 0 today). (M T W S A)
- [ ] **Stable images.** `image` links are signed and expire in 1–2 weeks; `/sync/changes`
      doesn't resend unchanged records, so links kept in a cache, a widget or the car go dead.
      Either long-lived by-id image routes or a refresh rule clients can follow. (A, W, T)
- [ ] **Auth error shapes.** Wrong credentials are `invalid_grant` on `/auth/token` but other
      v2 auth errors answer 401 `invalid_credentials`; a dead refresh token is 400
      `invalid_grant` (not v1's 401). Document it in `api-v2-first.md` and keep it. (Ab Am Ap)

## 1. Blocking — a client phase can't finish without it

- [ ] **Podcast feeds:** follow (`podcasts/subscribe`), unfollow, refresh. `POST /podcasts`
      only creates uploaded shows. (M T W Ap)
- [ ] **File sync records:** `path` on track, book, episode; `relative_path` and `sha256` on
      book files; `sha256` on tracks and episodes; also in the create answers. (S, so M W)
- [ ] **Change log triggers** for `sync_paths` (renames) and `content_grants`. (S)
- [ ] **Companion files** as a sync kind with create, visibility, delete, download. (S)
- [ ] **`/sync/changes?kinds=`**, and an ids-only snapshot for the reconcile. (S, A)
- [ ] **A `member` record kind** for owner names, including people who left. (S)
- [ ] **Continue listening in one read:** `include=` on `GET /me/progress` (or a batch lookup).
      (T Ap)

## 2. Account and sign-in

- [ ] Register with an invite; join links and a join preview before sign-in. (W Ab Am Ap)
- [ ] `PATCH /me` (display name, `discovery_languages`, skip intervals); profile picture
      upload. (M W A T)
- [ ] Password change, `DELETE /me`, two-factor setup and disable, security events, Subsonic
      key, verification e-mail resend. (M W Ab)
- [ ] Desktop Google/Microsoft: the loopback code + PKCE flow as a grant, and the desktop
      client id in discovery. (W, M)
- [ ] Setup state and the demo account in `/.well-known/own-audio`. (W Ab Am)

## 3. Podcasts

- [ ] Directory: search, categories, browse, similar, preview of a show not followed. (M T Ap)
- [ ] Unplayed counts. (M T)
- [ ] Keep an episode on the server / drop the server's copy / store-all / auto-store. (M Ap)
- [ ] Episodes, books and podcasts in `/search` (music only today). (T W)
- [ ] Translations as a v2 kind for `playback-info` and progress. (Ap)

## 4. Music

- [ ] Track feedback (dislike/ban) — or fold it into ratings. (M T W)
- [ ] Delete a track (and a book, a podcast). (M W Am)
- [ ] Lyrics write; file tags and rescan; the duplicates list; album identify. (M W Am)
- [ ] Cover uploads: track, playlist, artist picture. (M W Am)
- [ ] Playlists in `/sync/changes` (not blocking: read when shown). (M W Am)

## 5. Books

- [ ] `PATCH /books/{id}` (metadata), file rename and reorder, replace the cover, metadata
      search and apply. (M W Ab)
- [ ] `POST /books/{id}/files` (add files to a book). (S)
- [ ] Series and collections as writable resources; author create and delete. (W Ab)

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
- [ ] Hosted billing, narration and translation: v2 bodies (they are v1's under `/api/v2`).
      (W A)

## Order

0 first (it decides how clients check for the rest), then 1 in the order the client plans
need it — podcast feeds and the sync records unblock the most — then 2–7 as clients reach
them. Each line lands with its `openapi-v2.json` change, conformance tests and a CHANGELOG
line, like every v2 phase.
