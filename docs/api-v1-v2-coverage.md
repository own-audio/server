# API v1 → v2 coverage

Audit of 2026-11-17: every `/api/v1` operation, and whether `/api/v2` does the same job. Sources:
`docs/api/openapi.json` (v1, revision 26, 291 operations), `docs/api/openapi-v2.json` (v2,
revision 1), the hosted edition's routes in `audio2/hosted` (25 more under `/api/v1`, mounted
again under `/api/v2` through `finalize_with`), and the handlers where the specs left a doubt.
OpenSubsonic (`/rest`) and gPodder (`/api/2`) are out of scope.

Judged by function, not by path: v2 has normalised records, milliseconds, cursor paging, one
sync for every kind, one progress model, `playback-info` instead of stream URLs, and token
grants instead of separate sign-in routes. Paths below drop the `/api/v1` and `/api/v2`
prefixes.

| Status | Count |
|---|---|
| covered | 274 |
| partly covered | 3 |
| missing | 29 |
| not carried over | 10 |
| **total** | **316** |

"Not carried over" is either a decision in `api-v2-plan.md` (per-member media policies and item
grants stay on v1, and with them sharing a playlist with chosen members), a route no client calls
(the Stripe webhook), or a one-off back-fill that becomes a server job (disc numbers). The hosted routes count as
covered because they answer under `/api/v2`, but their bodies are still v1's.
What is missing or partly covered is tracked for the work in `api-v2-gaps-plan.md`.

Kept current as `api-v2-completion-plan.md` moves functions (updated for its phase 9, v2
revision 11). `scripts/check-v2-coverage.py` checks this table against both OpenAPI documents and
lists what is still open.


## Auth and account

| v1 operation | v2 | status | note |
|---|---|---|---|
| `POST /auth/register` | `POST /auth/register` | covered | Answers `Me`; signing in is `POST /auth/token`. `registration_closed`, `email_taken`, `weak_password`, `invalid_invite`. |
| `GET /auth/registration-status` | `GET /.well-known/own-audio` | covered | `modules.registration_open`. |
| `POST /auth/login` | `POST /auth/token` | covered | `grant_type=password`. |
| `POST /auth/totp/verify` | `POST /auth/token` | covered | The `mfa-otp` grant after `403 mfa_required`. |
| `POST /auth/refresh` | `POST /auth/token` | covered | `grant_type=refresh_token`. |
| `POST /auth/logout` | `POST /auth/revoke`, `DELETE /me/sessions/{id}` | covered |  |
| `POST /auth/fork` | `POST /auth/token` | covered | The token-exchange grant (RFC 8693), `subject_token_type` a refresh token. |
| `GET /auth/me` | `GET /me` | covered |  |
| `POST /auth/google` | `POST /auth/token` | covered | The jwt-bearer grant for an id token; the `authorization_code` grant with PKCE for the desktop's loopback code. |
| `POST /auth/apple` | `POST /auth/token` | covered | Assertion grant, `full_name` included. |
| `POST /auth/microsoft` | `POST /auth/token` | covered | Same as Google. |
| `GET /auth/providers` | `GET /.well-known/own-audio`, `AuthMethods.clients` | covered | `auth.providers` names them, `auth.clients` gives the desktop and web client ids, `auth.grants` the grants. |
| `POST /auth/device/start` | `POST /auth/device` | covered | RFC 8628 wire format. |
| `POST /auth/device/poll` | `POST /auth/token` | covered | The device-code grant. |
| `GET /auth/device/{user_code}` | `GET /auth/device/{user_code}` | covered |  |
| `POST /auth/device/{user_code}/approve` | `POST /auth/device/{user_code}/approve` | covered |  |
| `POST /auth/device/{user_code}/deny` | `POST /auth/device/{user_code}/deny` | covered |  |
| `POST /auth/password` | `PUT /me/password` | covered | Signs the other devices out, as v1. A signed-in session only, not a personal token. |
| `POST /auth/password/forgot` | `POST /auth/password-reset` | covered | `202` whatever the address; `501 feature_unavailable` on a server without mail (v1 answers `200` there). |
| `POST /auth/password/reset` | `POST /auth/password-reset/confirm` | covered | `204`; `invalid_token`, `weak_password`. |
| `POST /auth/email/resend` | `POST /me/email-verification` | covered | `202`; `409 already_verified`, `501` without mail instead of v1's `{sent, verified}`. |
| `POST /auth/email/verify` | `POST /auth/email-verification` | covered | `204`; `invalid_token`. |
| `GET /auth/security-events` | `GET /me/security-events` | covered | Cursor-paged over the whole year kept (v1: the last 100); the address cut to its network. |
| `GET /auth/sessions` | `GET /me/sessions` | covered |  |
| `DELETE /auth/sessions/{chain_id}` | `DELETE /me/sessions/{id}` | covered | v2 adds "sign out everywhere". |
| `GET /auth/totp` | `Me.two_factor` | covered | In `GET /me`. |
| `DELETE /auth/totp` | `DELETE /me/two-factor` | covered | A code or a recovery code, as v1 (not the password). |
| `POST /auth/totp/enable` | `PUT /me/two-factor` | covered |  |
| `POST /auth/totp/setup` | `POST /me/two-factor` | covered | `201`. |
| `GET /users/me` | `GET /me` | covered |  |
| `PATCH /users/me` | `PATCH /me` | covered | Answers `Me`; nothing is written unless all of it is valid. |
| `DELETE /users/me` | `DELETE /me` | covered | Asks for the password when the account has one (v1 does not); `409 last_admin`. |
| `POST /users/me/avatar` | `PUT /me/image` | covered | `{upload_id}`, or the image as the body (anyone may set their picture, not only those who may upload). `Me.image` is a weekly signed link. |
| `DELETE /users/me/avatar` | `DELETE /me/image` | covered |  |
| `GET /users/me/subsonic-key` | `GET /me/subsonic-key` | covered | `{username, key}`; a signed-in session only. |
| `POST /users/me/subsonic-key/regenerate` | `POST /me/subsonic-key` | covered |  |

## Server, setup and admin

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /server` | `GET /.well-known/own-audio`, `Discovery.demo` | covered | `demo` since v2 revision 3. |
| `GET /setup/status` | `Discovery.setup` | covered | `setup.complete`. The health checks are `GET /health`'s. |
| `POST /setup/complete` | — | missing | The first-run wizard. |
| `POST /auth/admin-create-user` | — | missing |  |
| `GET /users` | — | missing | Instance admin. |
| `GET /users/{id}` | `GET /me` | partly | Your own account only; an admin reading another account is missing. |
| `PATCH /users/{id}` | — | missing | Instance admin. |
| `DELETE /users/{id}` | — | missing | Instance admin. |
| `POST /users/{id}/revoke-sessions` | — | missing | Instance admin. |
| `GET /users/{id}/avatar` | `Member.image` → `GET /images/{id}` | covered | For people in your household. A signed link since phase 0; until then `Member.image` was this v1 route, which wants the access token. |
| `GET /admin/families` | — | missing |  |
| `GET /admin/families/trash` | — | missing |  |
| `GET /admin/families/{id}` | — | missing |  |
| `DELETE /admin/families/{id}` | — | missing |  |
| `POST /admin/families/{id}/lock` | — | missing |  |
| `POST /admin/families/{id}/unlock` | — | missing |  |
| `GET /admin/stats` | — | missing |  |

## Family and household

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /family` | `GET /family`, `GET /family/members` | covered |  |
| `PUT /family` | `PATCH /family` | covered |  |
| `GET /family/avatar` | `Family.image` → `GET /images/{id}` | covered | A signed link since phase 0; until then `Family.image` was this v1 route. |
| `POST /family/avatar` | `PUT /family/image` | covered | `{upload_id}` or the image itself, as `/me/image`; answers `Family`. |
| `DELETE /family/avatar` | `DELETE /family/image` | covered |  |
| `GET /family/content-reports` | `GET /family/reports` | covered | Cursor-paged; `state=open` (the default), `resolved` or `all` (v1: open ones only); v2 kinds `track`, `book`, `podcast`; who closed it and their note. |
| `POST /family/content-reports` | `POST /family/reports` | covered | `{kind, item_id, reason, note?}`, answers the report; only what the reporter can see (v1 took any id). |
| `POST /family/content-reports/{id}/resolve` | `PATCH /family/reports/{id}` | covered | `{resolved, note?}`: closes with a note, or opens again. |
| `GET /family/content/{kind}/{item_id}/audience` | — | not carried over | Per-member policies and grants stay on v1 (api-v2-plan.md). |
| `PUT /family/content/{kind}/{item_id}/audience` | — | not carried over | Same decision. |
| `POST /family/content/{kind}/{item_id}/request-access` | — | not carried over | Belongs to the same per-member access model. |
| `GET /family/invites` | `GET /family/invites` | covered |  |
| `POST /family/invites` | `POST /family/invites` | covered |  |
| `POST /family/invites/accept` | `POST /family/join` | covered |  |
| `DELETE /family/invites/{id}` | `DELETE /family/invites/{id}` | covered |  |
| `POST /family/invites/{id}/regenerate` | `POST /family/invites/{id}/regenerate` | covered | `409 invite_used` when spent (v1: `400`). |
| `GET /family/members` | `GET /family/members` | covered |  |
| `POST /family/members/provision` | `POST /family/members` | covered | Accounts for a kid or a grandparent: also the age bracket and rights; answers the member and its `claim` `Invite`. |
| `PUT /family/members/{user_id}` | `PATCH /family/members/{user_id}` | covered | Role, age bracket, rights, label. |
| `DELETE /family/members/{user_id}` | `DELETE /family/members/{user_id}` | covered | A member leaves themselves, as v1 (since phase 7); `409 last_admin`, `409 one_household`. |
| `GET /family/members/{user_id}/access` | — | not carried over | Per-member policies. |
| `POST /family/members/{user_id}/avatar` | `PUT /family/members/{user_id}/image` | covered | The member themselves too, not only an admin; answers `Member`. |
| `DELETE /family/members/{user_id}/avatar` | `DELETE /family/members/{user_id}/image` | covered | The member themselves too. |
| `POST /family/members/{user_id}/block` | `PATCH /family/members/{user_id}` | covered | `{blocked: true}`; not a server administrator (`403 cannot_block_instance_admin`). `MemberRecord.blocked` in the change feed. |
| `POST /family/members/{user_id}/unblock` | `PATCH /family/members/{user_id}` | covered | `{blocked: false}`. |
| `PUT /family/members/{user_id}/grants` | — | not carried over | Item grants stay on v1 (api-v2-plan.md). |
| `PUT /family/members/{user_id}/policy` | — | not carried over | Per-member media policies stay on v1. |
| `GET /family/settings/audio-analysis` | `GET /family/settings` | covered | One resource for the household's settings: `audio_analysis`. |
| `PUT /family/settings/audio-analysis` | `PATCH /family/settings` | covered | `{audio_analysis: {enabled}}`. |
| `GET /family/settings/book-source` | `GET /family/settings` | covered | `book_source`. |
| `PUT /family/settings/book-source` | `PATCH /family/settings` | covered | `{book_source: {source?, google_books_api_key?}}`; a server that offers no choice answers `501 feature_unavailable` (v1: `403`). |
| `GET /family/storage` | `GET /family/storage` | covered |  |
| `GET /join/{code}` | `POST /family/join/preview` | covered | The code in the body (rule 6); a used or expired code is `410` (v1: `200` with a status). |
| `POST /join/{code}/claim` | `POST /auth/claim` | covered | `{code, password}` → `Me`; signing in is `POST /auth/token` (v1 answered a session). |

## Audiobooks

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /audiobooks` | `GET /books` | covered |  |
| `POST /audiobooks` | `POST /books` | covered | A book with its files in one call; more files later with `POST /books/{id}/files`. |
| `POST /audiobooks/from-uploads` | `POST /books` | covered |  |
| `POST /audiobooks/upload` | `POST /uploads` + `POST /books` | covered | No multipart on v2, by design. |
| `GET /audiobooks/{id}` | `GET /books/{id}` | covered |  |
| `PUT /audiobooks/{id}` | `PATCH /books/{id}` | covered | Title, description, publisher, year, ISBN; author and narrator through `PUT /books/{id}/credits`. |
| `DELETE /audiobooks/{id}` | `DELETE /books/{id}` | covered | Into the 30-day trash, a batch of its own; several things at once through `POST /trash`. |
| `GET /audiobooks/{id}/chapters` | `GET /books/{id}` | covered | Chapters are in the book. |
| `GET /audiobooks/{id}/cover` | `Book.image` → `GET /images/{id}` | covered |  |
| `GET /audiobooks/{id}/files` | `GET /books/{id}` | covered | Files are in the book. |
| `POST /audiobooks/{id}/files/from-uploads` | `POST /books/{id}/files` | covered | Many at once, `Idempotency-Key`; answers the book. |
| `PUT /audiobooks/{id}/files/reorder` | `PUT /books/{id}/files/order` | covered | As a playlist's order: files left out follow; an unknown or repeated id is `400 invalid_file` (v1 left them unplaced). |
| `PUT /audiobooks/{id}/files/{file_id}` | `PATCH /books/{id}/files/{file_id}` | covered | Rename a file. |
| `DELETE /audiobooks/{id}/files/{file_id}` | `DELETE /books/{id}/files/{file_id}` | covered | The last file stays (`409 last_file`); v1 left a book without files. |
| `GET /audiobooks/{id}/files/{file_id}/stream` | `POST /playback-info` | covered | Kind `book_file`. |
| `POST /audiobooks/{id}/metadata/search` | `POST /books/{id}/identity/search` | covered | The owner's, or a household admin's on a shared book (v1: anyone who sees it); `{items}`. |
| `POST /audiobooks/{id}/metadata/apply` | `PUT /books/{id}/identity` | covered | Source failures as `502 source_unavailable`, `503 source_refused` (v1: `400` or `500`). |
| `POST /audiobooks/{id}/upload-cover` | `PUT /books/{id}/image` | covered | An upload id (`POST /uploads`); `DELETE /books/{id}/image` takes it away. |
| `POST /audiobooks/{id}/upload-file` | `POST /uploads` + `POST /books/{id}/files` | covered | No multipart on v2. Replacing the file at a place: delete it, add the new one. |
| `PUT /audiobooks/{id}/visibility` | `PUT /books/{id}/visibility` | covered |  |
| `GET /audiobooks/authors` | `GET /authors` | covered |  |
| `POST /audiobooks/authors` | `POST /contributors` | covered | A name the household has answers that one (`200`). |
| `GET /audiobooks/authors/{id}` | `GET /contributors/{id}` | covered |  |
| `PUT /audiobooks/authors/{id}` | `PATCH /contributors/{id}` | covered | Name, sort name, kind and `bio`. |
| `DELETE /audiobooks/authors/{id}` | `DELETE /contributors/{id}` | covered | `409 in_use` while anything credits them (v1 took the credits with them). |
| `GET /audiobooks/authors/{id}/books` | `GET /books?contributor_id=` | covered |  |
| `GET /audiobooks/authors/{id}/image` | `Contributor.image` → `GET /images/{id}` | covered |  |
| `POST /audiobooks/authors/{id}/image` | `PUT /contributors/{id}/image` | covered | An upload id. Serves artists too. |
| `DELETE /audiobooks/authors/{id}/image` | `DELETE /contributors/{id}/image` | covered | Forget the picture and look it up again. |
| `GET /audiobooks/authors/{id}/image-info` | `Contributor.image_credit` | covered | In the record: no request of its own. |
| `GET /audiobooks/authors/book/{book_id}` | `GET /books/{id}` | covered | Credits with roles. |
| `POST /audiobooks/authors/book/{book_id}` | `PUT /books/{id}/credits` | covered |  |
| `DELETE /audiobooks/authors/book/{book_id}/{author_id}/{role}` | `PUT /books/{id}/credits` | covered |  |
| `GET /audiobooks/authors/tags` | `GET /book-tags` | covered | Cursor-paged, with how many visible books carry each. |
| `GET /audiobooks/authors/tags/book/{book_id}` | `Book.tags` | covered | In the full book record; `GET /books?tag=` filters. |
| `PUT /audiobooks/authors/tags/book/{book_id}` | `PUT /books/{id}/tags` | covered |  |
| `GET /audiobooks/organize/collections` | `GET /collections` | covered | Cursor-paged; shared ones too (`visibility`, v1's were the caller's only). |
| `POST /audiobooks/organize/collections` | `POST /collections` | covered | With books to start with. |
| `GET /audiobooks/organize/collections/{id}` | `GET /collections/{id}` | covered |  |
| `PUT /audiobooks/organize/collections/{id}` | `PATCH /collections/{id}` | covered | `visibility` instead of `is_public`. |
| `DELETE /audiobooks/organize/collections/{id}` | `DELETE /collections/{id}` | covered |  |
| `GET /audiobooks/organize/collections/{id}/books` | `GET /collections/{id}/books` | covered | Cursor-paged, the v2 `Book` in each entry. |
| `POST /audiobooks/organize/collections/{id}/books` | `POST /collections/{id}/books` | covered | Many at once. |
| `DELETE /audiobooks/organize/collections/{id}/books/{book_id}` | `DELETE /collections/{id}/books/{book_id}` | covered |  |
| `GET /audiobooks/organize/favorites` | `GET /me/favourites?kind=book` | covered |  |
| `POST /audiobooks/organize/favorites/{book_id}` | `PUT /me/favourites/book/{id}` | covered |  |
| `DELETE /audiobooks/organize/favorites/{book_id}` | `DELETE /me/favourites/book/{id}` | covered |  |
| `GET /audiobooks/organize/favorites/{book_id}/check` | `GET /me/favourites`, sync | covered |  |
| `GET /audiobooks/organize/series` | `GET /series` | covered | Cursor-paged; shared ones too. |
| `GET /audiobooks/organize/series/{id}` | `GET /series/{id}` | covered | The books the reader can see, by place. |
| `POST /audiobooks/organize/series` | `POST /series` | covered | With books to start with; `visibility`. |
| `PUT /audiobooks/organize/series/{id}` | `PATCH /series/{id}` | covered |  |
| `DELETE /audiobooks/organize/series/{id}` | `DELETE /series/{id}` | covered |  |
| `POST /audiobooks/organize/series/{id}/books` | `PUT /series/{id}/books` | covered | One ordered list instead of a book at a time. |
| `DELETE /audiobooks/organize/series/{id}/books/{book_id}` | `PUT /series/{id}/books` | covered | The list without it. |

## Podcasts

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /podcasts` | `GET /podcasts` | covered |  |
| `GET /podcasts/{id}` | `GET /podcasts/{id}` | covered |  |
| `GET /podcasts/{id}/episodes` | `GET /podcasts/{id}/episodes` | covered | Progress comes from `GET /me/progress?kind=episode` or sync. |
| `GET /podcasts/{id}/image` | `Podcast.image` | covered |  |
| `GET /podcasts/{id}/episodes/{ep_id}/image` | `Episode.image` | covered |  |
| `GET /podcasts/{id}/episodes/{ep_id}/stream` | `POST /playback-info` | covered | Stored copy or the publisher's file. |
| `GET /podcasts/{id}/episodes/{ep_id}/transcript` | `GET /episodes/{id}/transcript` | covered | The server reads the publisher's file (or the uploaded one) into timed lines, now with the speaker where the file names one; `Episode.transcript_url` still links the file itself. A missing one is `404 no_transcript`. |
| `PUT /podcasts/{id}/visibility` | `PUT /podcasts/{id}/visibility` | covered |  |
| `POST /podcasts/subscribe` | `POST /podcasts` | covered | `{feed_url, visibility?}`; a show the caller has already (or sees shared) answers `200` without fetching; `Idempotency-Key`. Any member, as on v1; an uploaded show still needs `can_upload`. |
| `DELETE /podcasts/{id}` | `DELETE /podcasts/{id}` | covered | Unfollow; the episodes' removals reach the change feed with the show's. The owner, or a household admin for a shared show (v1: the owner). An uploaded show answers `409` until the trash holds shows (completion plan phase 11). |
| `POST /podcasts/{id}/refresh` | `POST /podcasts/{id}/refresh` | covered | Queued (`202`), the art stored on the way; new episodes arrive through `/sync/changes`. `POST /podcasts/refresh` refreshes every show. |
| `GET /podcasts/unplayed-counts` | `GET /me/podcasts/unplayed` | covered | Paged by podcast id, every visible show once (zero included); one grouped query per page. |
| `PUT /podcasts/{id}/auto-store` | `PATCH /podcasts/{id}` | covered | `{auto_store}`; `Podcast.auto_store` tells. |
| `POST /podcasts/{id}/store-all` | `POST /podcasts/{id}/store` | covered | `{latest?, preview?}`; `202` queued, `200` for a preview. |
| `POST /podcasts/{id}/episodes/{ep_id}/download` | `PUT /episodes/{id}/stored` | covered | Queued (`202`); `Episode.stored` turns true in the change feed. v1 fetched it during the request. |
| `DELETE /podcasts/{id}/episodes/{ep_id}/download` | `DELETE /episodes/{id}/stored` | covered | Into the trash, as v1. |
| `POST /podcasts/{id}/sync-images` | `POST /podcasts/{id}/refresh` | covered | Part of the refresh. |
| `POST /podcasts/search` | `GET /podcasts/directory/search` | covered | A `GET` with `q`; results carry `podcast_id` when the caller has the show. `501` when `modules.podcast_search` is off, `502` when the directory does not answer (v1: `500`). |
| `GET /podcasts/discover/browse` | `GET /podcasts/directory` | covered | `category=`; cursor-paged instead of `offset`. |
| `GET /podcasts/discover/categories` | `GET /podcasts/directory/categories` | covered | One page. |
| `GET /podcasts/discover/preview` | `GET /podcasts/directory/preview` | covered | Adds the show itself and each episode's file and art; milliseconds. |
| `GET /podcasts/discover/similar` | `GET /podcasts/directory/similar` | covered | `feed_url=`. |
| `GET /podcasts/{id}/similar` | `GET /podcasts/directory/similar` | covered | `podcast_id=`; one route for both. |

## Music

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /music/albums` | `GET /albums` | covered |  |
| `POST /music/albums/identify` | `POST /organize/releases` | covered | Then `POST /organize/apply-release`. |
| `PATCH /music/albums/{id}` | `PATCH /albums/{id}` | covered |  |
| `POST /music/albums/{id}/merge` | `POST /albums/{id}/merge` | covered |  |
| `PUT /music/albums/{id}/release-group` | `PUT /albums/{id}/release-group` | covered |  |
| `GET /music/artists` | `GET /artists` | covered |  |
| `PATCH /music/artists/{id}` | `PATCH /contributors/{id}` | covered |  |
| `POST /music/artists/{id}/merge` | `POST /contributors/{id}/merge` | covered |  |
| `POST /music/artists/{id}/split` | `POST /contributors/{id}/split` | covered |  |
| `GET /music/artists/image` | `Contributor.image` | covered |  |
| `POST /music/artists/upload-image` | `PUT /contributors/{id}/image` | covered | An upload id; an artist is a contributor, so one route for artists and authors. Kept for good: never replaced by a looked-up photo. |
| `DELETE /music/artists/image` | `DELETE /contributors/{id}/image` | covered | Forget the picture and look again, in the background: an artist's photo when credited on music, an author's when only on books. What is found reaches the change feed as the contributor. |
| `GET /music/artists/image-info` | `Contributor.image_credit` | covered | In the record: no request of its own. `null` for a picture set by hand (v1's `is_user_set`). |
| `GET /music/genres` | `GET /genres` | covered |  |
| `GET /music/duplicates` | `GET /tracks/duplicates` | covered | Cursor-paged groups `{key, kind, track_ids}` (the file's SHA-256, `identical`), found in SQL; `include=tracks` embeds the tracks once (v1: the whole list with copies of each track). The caller's own tracks only, as v1. |
| `GET /music/feedback` | `GET /me/feedback` | covered | Cursor-paged, most recent first; `value=dislike` or `ban` (v1's `banned`); only tracks the caller can see. Also kind `feedback.track` in `/sync/snapshot` and `/sync/changes`. |
| `POST /music/intent` | `POST /smart-playlists/intent` | covered | `{rule}`; a sentence with no music in it is `422 not_understood` (v1: `400`). Reads the household's genre names from the genre table instead of grouping every track. |
| `GET /music/starred` | `GET /me/favourites?kind=track` | covered |  |
| `GET /music/tracks` | `GET /tracks`, `GET /sync/snapshot` | covered |  |
| `POST /music/tracks/upload` | `POST /uploads` + `POST /tracks` | covered | No multipart on v2, by design. |
| `POST /music/tracks/from-upload` | `POST /tracks` | covered |  |
| `POST /music/tracks/discs` | — | not carried over | A one-off back-fill of the disc numbers of uploads made before they were read: no client should have to start it. It becomes a server job with the instance-admin and jobs work (completion plan phase 12); v1's route stays until then. |
| `GET /music/tracks/{id}` | `GET /tracks/{id}` | covered |  |
| `PUT /music/tracks/{id}` | `PATCH /tracks` | covered | One song or many, as a batch in the edit history. |
| `DELETE /music/tracks/{id}` | `DELETE /tracks/{id}` | covered | As a book. v1's `X-Trash-Batch` header is not taken: one `POST /trash` is one batch. |
| `GET /music/tracks/{id}/chapters` | `GET /tracks/{id}` | covered | Chapters are in the track answer. |
| `GET /music/tracks/{id}/cover` | `Track.image` | covered |  |
| `POST /music/tracks/{id}/upload-cover` | `PUT /tracks/{id}/image` | covered | An upload id (no multipart); `DELETE /tracks/{id}/image` takes it away. Both are changes in the edit history, as v1's upload. |
| `GET /music/tracks/{id}/stream` | `POST /playback-info` | covered | Adds transcoding. |
| `PUT /music/tracks/{id}/visibility` | `PUT /tracks/{id}/visibility` | covered |  |
| `PUT /music/tracks/{id}/star` | `PUT /me/favourites/track/{id}` | covered |  |
| `DELETE /music/tracks/{id}/star` | `DELETE /me/favourites/track/{id}` | covered |  |
| `PUT /music/tracks/{id}/feedback` | `PUT /me/feedback/{kind}/{id}` | covered | `{value}`, `dislike` or `ban`, kind `track`; the listener's state like favourites. |
| `DELETE /music/tracks/{id}/feedback` | `DELETE /me/feedback/{kind}/{id}` | covered | Idempotent, as v1. |
| `GET /music/tracks/{id}/lyrics` | `GET /tracks/{id}/lyrics` | covered |  |
| `PUT /music/tracks/{id}/lyrics` | `PUT /tracks/{id}/lyrics` | covered | `{text}` (plain or LRC) or timed `{lines}`, answers what `GET` answers. Taking them away reads the file's own again (v1 stored an empty text and never looked again). |
| `POST /music/tracks/{id}/metadata/search` | `POST /tracks/{id}/identity/search` | covered |  |
| `POST /music/tracks/{id}/metadata/apply` | `PUT /tracks/{id}/identity` | covered |  |
| `POST /music/tracks/{id}/metadata/revert` | `DELETE /tracks/{id}/identity` | covered |  |
| `GET /music/tracks/{id}/metadata/file-tags` | `GET /tracks/{id}/file-tags` | covered | The owner's (v1: anyone who sees the track; reading a whole file is not every member's to ask for); disc numbers too. |
| `POST /music/tracks/{id}/metadata/rescan` | `POST /tracks/{id}/rescan` | covered | Answers the track; `409 identified`, `422 no_tags` (v1: `400` for both). |
| `GET /music/playlists` | `GET /playlists` | covered |  |
| `POST /music/playlists` | `POST /playlists` | covered |  |
| `GET /music/playlists/{id}` | `GET /playlists/{id}` | covered |  |
| `PUT /music/playlists/{id}` | `PATCH /playlists/{id}` | covered |  |
| `DELETE /music/playlists/{id}` | `DELETE /playlists/{id}` | covered |  |
| `PUT /music/playlists/{id}/visibility` | `PATCH /playlists/{id}` | covered |  |
| `GET /music/playlists/{id}/tracks` | `GET /playlists/{id}/entries` | covered |  |
| `POST /music/playlists/{id}/tracks` | `POST /playlists/{id}/entries` | covered |  |
| `PUT /music/playlists/{id}/tracks/reorder` | `PUT /playlists/{id}/entries/order` | covered |  |
| `DELETE /music/playlists/{id}/tracks/{entry_id}` | `DELETE /playlists/{id}/entries/{entry_id}` | covered |  |
| `GET /music/playlists/{id}/cover` | `Playlist.image` | covered |  |
| `POST /music/playlists/{id}/upload-cover` | `PUT /playlists/{id}/image` | covered | An upload id; `DELETE /playlists/{id}/image` goes back to the first track's cover. |
| `PUT /music/playlists/{id}/share` | — | not carried over | Sharing a playlist with chosen members, live or as copies, is the per-member access model that stays on v1 (question 2 of the completion plan, decided 2026-11-17 in line with question 1, pending Kornel). Sharing with the whole household is `visibility` in `PATCH /playlists/{id}`. |
| `GET /music/playlists/{id}/audience` | — | not carried over | Which chosen members can play it: the same per-member model (question 2 of the completion plan, decided 2026-11-17 in line with question 1, pending Kornel). |
| `PUT /music/playlists/{id}/keep` | `PATCH /playlists/{id}` | covered | `{kept}`, which also lets one go again; `Playlist.generated` and `Playlist.kept` in the record; `409 not_generated` for one made by hand (v1: `404`). |
| `GET /music/smart-playlists` | `GET /smart-playlists` | covered |  |
| `POST /music/smart-playlists` | `POST /smart-playlists` | covered |  |
| `GET /music/smart-playlists/presets` | `GET /smart-playlists/presets` | covered |  |
| `POST /music/smart-playlists/resolve` | `POST /smart-playlists/resolve` | covered |  |
| `GET /music/smart-playlists/{id}` | `GET /smart-playlists/{id}` | covered |  |
| `PUT /music/smart-playlists/{id}` | `PATCH /smart-playlists/{id}` | covered |  |
| `DELETE /music/smart-playlists/{id}` | `DELETE /smart-playlists/{id}` | covered |  |
| `POST /music/smart-playlists/{id}/freeze` | `POST /smart-playlists/{id}/freeze` | covered |  |
| `POST /music/smart-playlists/{id}/resolve` | `POST /smart-playlists/{id}/resolve` | covered |  |

## Playback, progress and stats

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /music/tracks/{id}/progress` | `GET /me/progress/track/{id}` | covered |  |
| `PUT /music/tracks/{id}/progress` | `PUT /me/progress/track/{id}` | covered |  |
| `GET /playback/books/progress` | `GET /me/progress?kind=book` | covered |  |
| `GET /playback/books/{book_id}/progress` | `GET /me/progress/book/{id}` | covered | Plus each device's last place. |
| `PUT /playback/books/{book_id}/progress` | `PUT /me/progress/book/{id}` | covered | ms across files, or file + position. |
| `DELETE /playback/books/{book_id}/progress` | `DELETE /me/progress/book/{id}` | covered |  |
| `GET /playback/episodes/{episode_id}/progress` | `GET /me/progress/episode/{id}` | covered |  |
| `PUT /playback/episodes/{episode_id}/progress` | `PUT /me/progress/episode/{id}` | covered |  |
| `POST /playback/episodes/progress/bulk` | `POST /sync/push` | covered | Many progress changes in one request. |
| `GET /playback/bookmarks` | `GET /me/bookmarks` | covered |  |
| `POST /playback/bookmarks` | `POST /me/bookmarks` | covered |  |
| `PUT /playback/bookmarks/{id}` | `PATCH /me/bookmarks/{id}` | covered |  |
| `DELETE /playback/bookmarks/{id}` | `DELETE /me/bookmarks/{id}` | covered |  |
| `GET /playback/books/{book_id}/bookmarks` | `GET /me/bookmarks?kind=book&item_id=` | covered |  |
| `POST /playback/sessions` | `POST /me/plays` | covered | Feeds the same listening statistics. |
| `GET /playback/queue` | `GET /me/queue` | covered | v2 words (`{kind, id, file_id?}`, `index`, `position_ms`) and a `revision`, also the `ETag`; one queue with v1 and Subsonic. Also in the change feed as `queue`. |
| `PUT /playback/queue` | `PUT /me/queue` | covered | `If-Match: <revision>` refuses a stale write with `412 revision_mismatch` (v1: last write wins, still the case without it); items must be playable by the caller. |
| `GET /playback/settings` | `GET /me/settings` | covered | One resource with the audiobook defaults and the equaliser (optimisation 8); milliseconds. Also in the change feed as `settings`. |
| `PUT /playback/settings/audiobook-defaults` | `PATCH /me/settings` | covered | `audiobook: {speed, skip_back_ms, skip_forward_ms}`; out of range is `400`, not clamped. |
| `GET /playback/settings/eq` | `GET /me/settings` | covered | `eq`: every device kind's saved curve, `bands_db` as six numbers. |
| `PUT /playback/settings/eq` | `PATCH /me/settings` | covered | `eq: {device_kind?, enabled?, preamp_db?, bands_db?, preset_name?}`; the caller's device kind when none is named. |
| `GET /stats/me` | `GET /me/stats` | covered | `from`/`to` instead of `range`, `group` (day, week, month), `kind=`; ms; the most listened item by episode, not by show. Read from the plays, as v1. |
| `GET /stats/me/history` | `GET /me/plays` | covered | Cursor-paged (v1: offset); the plays `POST /me/plays` records, in its shape, without titles (records come from sync). |
| `PUT /stats/me/visibility` | `PATCH /me/settings` | covered | `stats_visible_to_admin` (v1's `family_admin`); in the change feed with the settings. |
| `GET /stats/family` | `GET /family/stats` | covered | Household admins, as v1; members who do not share are absent rather than `hidden`. |
| `PUT /stats/family/members/{user_id}/visibility` | `PATCH /family/members/{id}` | covered | `stats_visible_to_admin`, for a managed account or oneself (`403 member_not_restricted`), as v1. |
| `GET /media` | `GET /media` | covered | Phase 0: the server's signed links point at `/api/v2/media`; v1's route takes the same links. `/stream` sends a file the server keeps itself without redirecting there. |
| `PUT /media` | `PUT /media` | covered | Phase 0: the upload target for local storage. |

## Sync and library

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /library/changes` | `GET /sync/changes`, `GET /sync/snapshot` | covered | Every kind, tombstones, user state. |
| `GET /library/continue` | `GET /me/progress?kind=` | covered | `kind=track,book,episode&unfinished=true&include=items,podcasts`: one list across kinds, most recent first, the records embedded once. Translations follow with the hosted edition (phase 13). |
| `GET /library/search` | `GET /search`, `GET /books?q=` | partly | `/search` is music only; podcasts and episodes cannot be searched. |
| `GET /library/private` | `owner_id`, `visibility` on records | partly | No filter on the lists; a client filters its synced records. |
| `GET /library/folders` | — | missing | Server-side library folders and their last scan. |
| `POST /library/folders/scan` | — | missing |  |
| `GET /sync/tree` | `GET /sync/changes` | covered | With `GET /sync/snapshot`: `path` and `sha256` on tracks, books and episodes, `relative_path` and `sha256` on book files; renames, grants and policy changes in the feed; owners' names as `member` records, people who left included; `kinds=` to read only what the folder shows. Companion files have rows of their own below. |
| `GET /sync/tree/ids` | `GET /sync/snapshot` | covered | `fields=ids`: `{type: id, kind, id}` lines, every kind (v1: the folder's), about 60 bytes an item. |
| `POST /sync/paths/organise` | — | missing | Move items to their default folder paths. |
| `GET /sync/holdings` | — | missing |  |
| `PUT /sync/holdings` | — | missing |  |
| `GET /sync/shortcuts` | — | missing |  |
| `POST /sync/shortcuts` | — | missing |  |
| `DELETE /sync/shortcuts/{id}` | — | missing |  |
| `POST /sync/files` | — | missing | Companion files (images, booklets, lyrics, cue sheets). |
| `DELETE /sync/files/{id}` | — | missing |  |
| `GET /sync/files/{id}/stream` | — | missing |  |
| `PUT /sync/files/{id}/visibility` | — | missing |  |
| `POST /sync/files/{id}/use-as-cover` | — | missing |  |

## Uploads, trash and jobs

| v1 operation | v2 | status | note |
|---|---|---|---|
| `POST /uploads/presign` | `POST /uploads` | covered | The size is now required. |
| `POST /uploads/complete` | `POST /tracks`, `/books`, `/podcasts/{id}/episodes` | covered | The create call names the `upload_id`. |
| `GET /trash` | `GET /trash` | covered | Cursor-paged, newest first (v1: the whole trash); v2 kinds (`track`, `book`, `playlist`, `episode`, `file`); `deleted_at`, `deleted_by`, `owner_id` instead of v1's people objects; `scope=family` for a household admin. |
| `POST /trash/batches/{batch}/restore` | `POST /trash/batches/{batch}/restore` | covered | One result per item and the charge. A v1 `X-Trash-Batch` batch restores too. |
| `POST /trash/empty` | `DELETE /trash` | covered | `{purged}`; walks the trash page by page instead of reading it whole. |
| `DELETE /trash/{kind}/{id}` | `DELETE /trash/{kind}/{id}` | covered | v2 kinds; `400 invalid_kind`. |
| `POST /trash/{kind}/{id}/restore` | `POST /trash/{kind}/{id}/restore` | covered | `204` (v1 answers the charge; the list quotes it beforehand). |
| `GET /jobs` | — | missing | Instance admin. |
| `GET /jobs/{id}` | — | missing |  |

## Notifications and devices

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /devices/notifications` | `GET /me/notifications` | covered | Cursor-paged, newest first, read ones too (`unread_only=true` for v1's list); `item` in v2 words. New and read ones reach the change feed as `notification`. |
| `POST /devices/notifications/ack` | `POST /me/notifications/ack` | covered | `{ids}` as v1, or `{up_to}` for everything up to one. |
| `POST /devices/push-token` | `POST /me/push-tokens` | covered | Answers the device (`201`, `200` when already the caller's). `modules.push` says whether the server sends any. |
| `DELETE /devices/push-token` | `DELETE /me/push-tokens/{id}` | covered | By the device's id, not the token. |
| `GET /devices/push-tokens` | `GET /me/push-tokens` | covered |  |

## Hosted edition (audio2)

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /family/billing` | `GET /family/billing` | covered | Same handler, v1 body. |
| `POST /family/billing/topup` | `POST /family/billing/topup` | covered | Same handler, v1 body. |
| `GET /family/billing/alerts` | `GET /family/billing/alerts` | covered | Same handler, v1 body. |
| `PUT /family/billing/alerts` | `PUT /family/billing/alerts` | covered | Same handler, v1 body. |
| `POST /billing/stripe/webhook` | — | not carried over | Stripe calls it, not a client; stays where Stripe is configured. |
| `POST /admin/families/{id}/credit` | — | missing | Instance admin grants credit. |
| `GET /audiobook-gen/languages` | `GET /narration/languages` | covered | Same handler, v1 body. |
| `GET /audiobook-gen/cover-prompt` | `GET /narration/cover-prompt` | covered | Same. |
| `GET /audiobook-gen/voices` | `GET /narration/voices` | covered | Same. |
| `POST /audiobook-gen/estimate` | `POST /narration/estimate` | covered | Same. |
| `POST /audiobook-gen/quote` | `POST /narration/quote` | covered | Same. |
| `POST /audiobook-gen/jobs` | `POST /narration/jobs` | covered | Same, with `Idempotency-Key`. |
| `GET /audiobook-gen/jobs/{id}` | `GET /narration/jobs/{id}` | covered | Same. |
| `POST /podcast-translate/episodes/{id}/quote` | `POST /translation/episodes/{id}/quote` | covered | Same handler, v1 body. |
| `POST /podcast-translate/episodes/{id}` | `POST /translation/episodes/{id}` | covered | Same. |
| `GET /podcast-translate/episodes/{id}` | `GET /translation/episodes/{id}` | covered | Same. |
| `PUT /podcast-translate/episodes/{id}/progress` | `PUT /translation/episodes/{id}/progress` | covered | Same; not in `/me/progress`. |
| `GET /podcast-translate/feeds/{id}/count` | `GET /translation/feeds/{id}/count` | covered | Same. |
| `GET /podcast-translate/feeds/{id}` | `GET /translation/feeds/{id}` | covered | Same. |
| `GET /podcast-translate/recent` | `GET /translation/recent` | covered | Same. |
| `GET /podcast-ai/episodes/{id}/summary` | `GET /summaries/episodes/{id}/summary` | covered | Same handler, v1 body. |
| `POST /podcast-ai/episodes/{id}/summary` | `POST /summaries/episodes/{id}/summary` | covered | Same. |
| `POST /podcast-ai/episodes/{id}/ask` | `POST /summaries/episodes/{id}/ask` | covered | Same. |
| `GET /podcast-ai/episodes/{id}/questions` | `GET /summaries/episodes/{id}/questions` | covered | Same. |
| `DELETE /podcast-ai/episodes/{id}/questions` | `DELETE /summaries/episodes/{id}/questions` | covered | Same. |
