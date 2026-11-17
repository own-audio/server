# /api/v2 gaps — what the clients still need from the server

Done 2026-11-17: every line below is ticked, with v2 revision 16 (release 1.0.4-beta.1, v2 stable);
what stays on v1 by decision is under "Later". Written 2026-11-17 from the clients' migration plans (`API_V2_PLAN.md` in `audio2-mac`,
`audio2-tvos`, `audio2-win`, `audio2-sync`, `audio2-android-book`, `audio2-android-music`,
`audio2-android-podcast`). Each line is a v1 call some client makes that has no v2 counterpart
yet. Lines marked (audit) come from the full v1 → v2 audit of the same day
(`api-v1-v2-coverage.md`): v1 functions no client plan named. Clients keep calling v1 for these, with a comment, until the line is done
(`api-v2-first.md`). Letters in brackets: who needs it — **M**ac, **T**V, **W**indows,
**S**ync core, **A**ndroid (b/m/p = book/music/podcast).

## 0. Contract first

- [x] **v2 is the main API and counts its revisions** (Kornel, 2026-11-17): revision 1,
      additive only from now on; discovery's v2 revision goes up with each line below so a
      client can check for it. It was `draft` until the release that completed
      `api-v2-completion-plan.md`, and is stable since 1.0.4-beta.1 (revision 16). (M T W S A)
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
- [x] **File sync records** (v2 revision 5): `path` on track, book, episode; `relative_path`
      and `sha256` on book files; `sha256` on tracks and episodes; also in the create answers.
      (S, so M W)
- [x] **Change log triggers** for `sync_paths` (renames) and `content_grants` (v2 revision 5;
      also media policies as a reset for that member, checksums, and what sharing an item
      makes visible: its album, contributors, a show's episodes). (S)
- [x] **Companion files** as a sync kind with create, visibility, delete, download (v2 revision 13:
      `POST /files`, `GET`/`DELETE /files/{id}`, `PUT /files/{id}/visibility`, kind `file` in sync,
      `POST /playback-info` kind `file`). (S)
- [x] **`/sync/changes?kinds=`**, and an ids-only snapshot for the reconcile
      (`/sync/snapshot?fields=ids`; v2 revision 5). (S, A)
- [x] **A `member` record kind** for owner names, including people who left (v2 revision 5).
      (S)
- [x] **Continue listening in one read** (v2 revision 5): `GET /me/progress` with several
      kinds, `unfinished=true` and `include=items,podcasts`. (T Ap)

## 2. Account and sign-in

- [x] Register with an invite (v2 revision 3: `POST /auth/register` with `invite_code`). The
      join preview before sign-in (`POST /family/join/preview`) and claiming an account an admin
      made (`POST /auth/claim`) came with the household (v2 revision 9). (W Ab Am Ap)
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
- [x] Episodes, books and podcasts in `/search` (v2 revision 12: `types=` takes `books`,
      `podcasts`, `episodes` and `contributors` beside the music). (T W)
- [x] Translations as a v2 kind for `playback-info` and progress (v2 revision 15, hosted:
      kind `translation` in `POST /playback-info`, `/me/progress/translation/{id}` and the change
      feed's `translation` and `progress.translation`; `PUT /translation/episodes/{id}/progress`
      stays as the alias writing the same position). (Ap)
- [x] Transcripts as text (v2 revision 4: `GET /episodes/{id}/transcript`, timed segments with
      speakers). (audit)
- [x] Storing a show's and its episodes' art (v2 revision 4: part of the refresh). (audit)

## 4. Music

- [x] Track feedback (dislike/ban) (v2 revision 8: `GET /me/feedback`, `PUT`/`DELETE
      /me/feedback/track/{id}`, kind `feedback.track` in sync; kept apart from ratings, as v1). (M T W)
- [x] Delete a track and a book (v2 revision 6: `DELETE /tracks/{id}`, `DELETE /books/{id}`,
      several at once with `POST /trash`). (M W Am)
- [x] Delete an uploaded podcast (v2 revision 13: `DELETE /podcasts/{id}` puts it in the trash
      with its episodes, kind `podcast`); a followed show is unfollowed with `DELETE
      /podcasts/{id}` since revision 4. (M W Am)
- [x] Lyrics write; file tags and rescan; the duplicates list; album identify (v2 revision 8:
      `PUT /tracks/{id}/lyrics`, `GET /tracks/{id}/file-tags`, `POST /tracks/{id}/rescan`,
      `GET /tracks/duplicates`; album identify is `POST /organize/releases` since revision 1).
      (M W Am)
- [x] Cover uploads: track, playlist, artist picture (v2 revision 8: `PUT`/`DELETE
      /tracks/{id}/image`, `/playlists/{id}/image`; artists through `/contributors/{id}/image`
      since revision 7). (M W Am)
- [x] Playlists in `/sync/changes` (v2 revision 8: kinds `playlist` and `smart_playlist` in the
      snapshot and the change feed, the trash included; entries stay a paged read). (M W Am)
- [x] Keeping a generated playlist (v2 revision 8: `kept` in `PATCH /playlists/{id}`). Playlists
      shared with chosen members, live or as copies, and who can play one (`share`, `audience`)
      stay on v1 with the per-member model (completion plan question 2, decided 2026-11-17 in
      line with question 1, pending Kornel); v2 shares with the whole household. (audit)
- [x] Artist and author pictures: forget one and look it up again; the fetched picture's
      credit and licence (`image-info`) (v2 revisions 7–8: `PUT`/`DELETE
      /contributors/{id}/image`, the lookup after a `DELETE`, `Contributor.image_credit`). (audit)
- [x] A sentence to a smart-playlist rule (v2 revision 8: `POST /smart-playlists/intent`). (audit)
- [x] The disc-number back-fill for old uploads (`tracks/discs`): not carried over as a route;
      the server's `disc_backfill` job since v2 revision 14, its progress in
      `GET /admin/jobs`. (audit)

## 5. Books

- [x] `PATCH /books/{id}` (metadata), file rename and reorder, replace the cover, metadata
      search and apply (v2 revision 7: `PATCH /books/{id}`, `PATCH /books/{id}/files/{file_id}`,
      `PUT /books/{id}/files/order`, `PUT`/`DELETE /books/{id}/image`,
      `POST /books/{id}/identity/search`, `PUT /books/{id}/identity`). (M W Ab)
- [x] `POST /books/{id}/files` (add files to a book; v2 revision 7). (S)
- [x] Series and collections as writable resources; author create and delete (v2 revision 7:
      `/series`, `/collections`, kinds `series` and `collection` in sync; `POST`/`DELETE
      /contributors`). (W Ab)
- [x] Delete one file of a book (v2 revision 7: `DELETE /books/{id}/files/{file_id}`). (audit)
- [x] Book tags (list, read, replace; v2 revision 7: `GET /book-tags`, `Book.tags`,
      `PUT /books/{id}/tags`, `GET /books?tag=`). (audit)
- [x] Author photo upload; `bio` in `PATCH /contributors/{id}` (v2 revision 7:
      `PUT`/`DELETE /contributors/{id}/image`, `Contributor.bio`, `Contributor.image_credit`).
      (audit)

## 6. Listening settings

- [x] The cross-device queue (v2 revision 10: `GET`/`PUT /me/queue` with a `revision` and
      `If-Match`, kind `queue` in the change feed and a `library` event on `/events`).
      (M W Ab Ap)
- [x] Playback settings and audiobook defaults; EQ (v2 revision 10: `GET`/`PATCH /me/settings`,
      milliseconds, every device kind's curve; kind `settings` in the change feed). (M T W A)

## 7. Household and the rest

- [x] Block/unblock, provisioned members and claim codes, invite regenerate, member and family
      pictures, content reports (v2 revision 9: `blocked` in `PATCH /family/members/{id}`,
      `POST /family/members`, `POST /family/join/preview`, `POST /auth/claim`,
      `POST /family/invites/{id}/regenerate`, `PUT`/`DELETE /family/image` and
      `/family/members/{id}/image`, `GET`/`POST /family/reports`, `PATCH /family/reports/{id}`;
      kind `family` and `MemberRecord.blocked` in the change feed). (M W Ab)
- [x] Notifications inbox; push devices (v2 revision 10: `GET /me/notifications`,
      `POST /me/notifications/ack` with `ids` or `up_to`, kind `notification` in the change
      feed; `GET`/`POST /me/push-tokens`, `DELETE /me/push-tokens/{id}`, `modules.push`).
      (M W A)
- [x] Stats (v2 revision 11: `GET /me/stats`, `GET /me/plays`, `GET /family/stats`,
      `stats_visible_to_admin` in `/me/settings` and `PATCH /family/members/{id}`). (M W A)
- [x] Trash with batches, restore, purge (v2 revision 6: `POST`/`GET`/`DELETE /trash`,
      `POST /trash/batches/{batch}/restore`, `POST /trash/{kind}/{id}/restore`,
      `DELETE /trash/{kind}/{id}`). (S M)
- [x] Holdings; shortcuts (album shortcuts by `album_id`) (v2 revision 13: `GET`/`PUT
      /me/holdings`, `GET /me/holdings/{kind}/{id}`, `GET`/`POST /sync/shortcuts`, `DELETE
      /sync/shortcuts/{id}`, kind `shortcut` in sync). (S M)
- [x] Instance admin: users, jobs (v2 revision 14: `GET`/`POST /admin/users`, `GET`/`PATCH`/
      `DELETE /admin/users/{id}`, `GET /admin/jobs` with `kind=` and `state=`,
      `GET /admin/jobs/{id}`). (W)
- [x] Instance admin, the rest: families (list, detail, trash, lock, unlock, delete), server
      stats, create a user, revoke a user's sessions, finish setup (v2 revision 14:
      `GET /admin/families?state=active|locked|trashed`, `GET`/`PATCH {locked}`/`DELETE
      /admin/families/{id}`, `GET /admin/stats`, `DELETE /admin/users/{id}/sessions`,
      `POST /setup`; the disc-number back-fill as the server's `disc_backfill` job). (audit)
- [x] Hosted: an administrator grants credit (v2 revision 15: `POST /admin/families/{id}/credit`
      with `{amount_minor, currency, note}` and `Idempotency-Key`; `credit` in `GET
      /admin/families/{id}` and `GET /admin/stats`). (audit)
- [x] A member leaving the household themselves: `DELETE /family/members/{id}` on oneself
      (v2 revision 9; `409 last_admin`, `409 one_household`). (audit)
- [x] Family settings: audio analysis, book source (v2 revision 9: `GET`/`PATCH
      /family/settings`). (audit)
- [x] Library folders and their scan (v2 revision 14: `GET /library/folders`,
      `POST /library/folders/scan`). (audit)
- [x] Moving items to their default folder paths (`/sync/paths/organise`); a companion image
      as cover (`use-as-cover`) (v2 revision 13: `POST /library/organise` with `dry_run`;
      `PUT /tracks/{id}/image` and `PUT /books/{id}/image` with `{file_id}`). (audit)
- [x] An owner or visibility filter on lists, for a "private" section (v1 `/library/private`;
      v2 revision 12: `owner=me` and `visibility=private|family` on `GET /tracks`, `/albums`,
      `/books`, `/podcasts`). (audit)
- [x] Hosted billing, narration and translation: v2 bodies (v2 revision 15, additively: minor
      units with `currency`, milliseconds, `limit`/`cursor` on the growing lists, `x-permission`
      in `audio2`'s `openapi-v2-hosted.json`; narration and translation jobs on `/events`). (W A)

## Later

Not in v2 by decision (`api-v1-v2-coverage.md`, not carried over); a phase of its own if a
client really needs it:

- [ ] Per-member policies, grants and audience, and sharing a playlist with chosen members
      (completion plan questions 1 and 2: the per-member access model stays on v1). (W Ab)

## Order

0 first (it decides how clients check for the rest), then 1 in the order the client plans
need it — podcast feeds and the sync records unblock the most — then 2–7 as clients reach
them. Each line lands with its `openapi-v2.json` change, conformance tests and a CHANGELOG
line, like every v2 phase.
