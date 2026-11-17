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
| covered | 182 |
| partly covered | 12 |
| missing | 115 |
| not carried over | 7 |
| **total** | **316** |

"Not carried over" is either a decision in `api-v2-plan.md` (per-member media policies and item
grants stay on v1) or a route no client calls (the Stripe webhook). The hosted routes count as
covered because they answer under `/api/v2`, but their bodies are still v1's.
What is missing or partly covered is tracked for the work in `api-v2-gaps-plan.md`.

Kept current as `api-v2-completion-plan.md` moves functions (updated for its phase 2, v2
revision 4). `scripts/check-v2-coverage.py` checks this table against both OpenAPI documents and
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
| `POST /family/avatar` | — | missing |  |
| `DELETE /family/avatar` | — | missing |  |
| `GET /family/content-reports` | — | missing |  |
| `POST /family/content-reports` | — | missing |  |
| `POST /family/content-reports/{id}/resolve` | — | missing |  |
| `GET /family/content/{kind}/{item_id}/audience` | — | not carried over | Per-member policies and grants stay on v1 (api-v2-plan.md). |
| `PUT /family/content/{kind}/{item_id}/audience` | — | not carried over | Same decision. |
| `POST /family/content/{kind}/{item_id}/request-access` | — | not carried over | Belongs to the same per-member access model. |
| `GET /family/invites` | `GET /family/invites` | covered |  |
| `POST /family/invites` | `POST /family/invites` | covered |  |
| `POST /family/invites/accept` | `POST /family/join` | covered |  |
| `DELETE /family/invites/{id}` | `DELETE /family/invites/{id}` | covered |  |
| `POST /family/invites/{id}/regenerate` | — | missing |  |
| `GET /family/members` | `GET /family/members` | covered |  |
| `POST /family/members/provision` | — | missing | Accounts for a kid or a grandparent, with a claim code. |
| `PUT /family/members/{user_id}` | `PATCH /family/members/{user_id}` | covered | Role, age bracket, rights, label. |
| `DELETE /family/members/{user_id}` | `DELETE /family/members/{user_id}` | partly | Admins only on v2: a member cannot leave the household themselves. |
| `GET /family/members/{user_id}/access` | — | not carried over | Per-member policies. |
| `POST /family/members/{user_id}/avatar` | — | missing |  |
| `DELETE /family/members/{user_id}/avatar` | — | missing |  |
| `POST /family/members/{user_id}/block` | — | missing |  |
| `POST /family/members/{user_id}/unblock` | — | missing |  |
| `PUT /family/members/{user_id}/grants` | — | not carried over | Item grants stay on v1 (api-v2-plan.md). |
| `PUT /family/members/{user_id}/policy` | — | not carried over | Per-member media policies stay on v1. |
| `GET /family/settings/audio-analysis` | — | missing |  |
| `PUT /family/settings/audio-analysis` | — | missing |  |
| `GET /family/settings/book-source` | — | missing |  |
| `PUT /family/settings/book-source` | — | missing |  |
| `GET /family/storage` | `GET /family/storage` | covered |  |
| `GET /join/{code}` | — | missing | The preview before sign-in. |
| `POST /join/{code}/claim` | — | missing | Claiming a provisioned account. |

## Audiobooks

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /audiobooks` | `GET /books` | covered |  |
| `POST /audiobooks` | `POST /books` | partly | v2 creates a book with its files in one call; adding files afterwards is missing. |
| `POST /audiobooks/from-uploads` | `POST /books` | covered |  |
| `POST /audiobooks/upload` | `POST /uploads` + `POST /books` | covered | No multipart on v2, by design. |
| `GET /audiobooks/{id}` | `GET /books/{id}` | covered |  |
| `PUT /audiobooks/{id}` | `PUT /books/{id}/credits` | partly | Author and narrator through credits; title and description cannot be edited. |
| `DELETE /audiobooks/{id}` | — | missing |  |
| `GET /audiobooks/{id}/chapters` | `GET /books/{id}` | covered | Chapters are in the book. |
| `GET /audiobooks/{id}/cover` | `Book.image` → `GET /images/{id}` | covered |  |
| `GET /audiobooks/{id}/files` | `GET /books/{id}` | covered | Files are in the book. |
| `POST /audiobooks/{id}/files/from-uploads` | — | missing |  |
| `PUT /audiobooks/{id}/files/reorder` | — | missing |  |
| `PUT /audiobooks/{id}/files/{file_id}` | — | missing | Rename a file. |
| `DELETE /audiobooks/{id}/files/{file_id}` | — | missing |  |
| `GET /audiobooks/{id}/files/{file_id}/stream` | `POST /playback-info` | covered | Kind `book_file`. |
| `POST /audiobooks/{id}/metadata/search` | — | missing |  |
| `POST /audiobooks/{id}/metadata/apply` | — | missing |  |
| `POST /audiobooks/{id}/upload-cover` | — | missing |  |
| `POST /audiobooks/{id}/upload-file` | — | missing |  |
| `PUT /audiobooks/{id}/visibility` | `PUT /books/{id}/visibility` | covered |  |
| `GET /audiobooks/authors` | `GET /authors` | covered |  |
| `POST /audiobooks/authors` | — | missing |  |
| `GET /audiobooks/authors/{id}` | `GET /contributors/{id}` | covered |  |
| `PUT /audiobooks/authors/{id}` | `PATCH /contributors/{id}` | partly | Name and sort name; `bio` is gone. |
| `DELETE /audiobooks/authors/{id}` | — | missing |  |
| `GET /audiobooks/authors/{id}/books` | `GET /books?contributor_id=` | covered |  |
| `GET /audiobooks/authors/{id}/image` | `Contributor.image` → `GET /images/{id}` | covered |  |
| `POST /audiobooks/authors/{id}/image` | — | missing |  |
| `DELETE /audiobooks/authors/{id}/image` | — | missing | Forget the picture and look it up again. |
| `GET /audiobooks/authors/{id}/image-info` | — | missing | Credit and licence of a fetched picture. |
| `GET /audiobooks/authors/book/{book_id}` | `GET /books/{id}` | covered | Credits with roles. |
| `POST /audiobooks/authors/book/{book_id}` | `PUT /books/{id}/credits` | covered |  |
| `DELETE /audiobooks/authors/book/{book_id}/{author_id}/{role}` | `PUT /books/{id}/credits` | covered |  |
| `GET /audiobooks/authors/tags` | — | missing | Book tags have no v2 counterpart at all. |
| `GET /audiobooks/authors/tags/book/{book_id}` | — | missing |  |
| `PUT /audiobooks/authors/tags/book/{book_id}` | — | missing |  |
| `GET /audiobooks/organize/collections` | — | missing | Collections cannot be read on v2. |
| `POST /audiobooks/organize/collections` | — | missing |  |
| `GET /audiobooks/organize/collections/{id}` | — | missing |  |
| `PUT /audiobooks/organize/collections/{id}` | — | missing |  |
| `DELETE /audiobooks/organize/collections/{id}` | — | missing |  |
| `GET /audiobooks/organize/collections/{id}/books` | — | missing |  |
| `POST /audiobooks/organize/collections/{id}/books` | — | missing |  |
| `DELETE /audiobooks/organize/collections/{id}/books/{book_id}` | — | missing |  |
| `GET /audiobooks/organize/favorites` | `GET /me/favourites?kind=book` | covered |  |
| `POST /audiobooks/organize/favorites/{book_id}` | `PUT /me/favourites/book/{id}` | covered |  |
| `DELETE /audiobooks/organize/favorites/{book_id}` | `DELETE /me/favourites/book/{id}` | covered |  |
| `GET /audiobooks/organize/favorites/{book_id}/check` | `GET /me/favourites`, sync | covered |  |
| `GET /audiobooks/organize/series` | `Book.series` | partly | A book names its series and place; no list of series. |
| `GET /audiobooks/organize/series/{id}` | `Book.series` | partly | Same. |
| `POST /audiobooks/organize/series` | — | missing |  |
| `PUT /audiobooks/organize/series/{id}` | — | missing |  |
| `DELETE /audiobooks/organize/series/{id}` | — | missing |  |
| `POST /audiobooks/organize/series/{id}/books` | — | missing |  |
| `DELETE /audiobooks/organize/series/{id}/books/{book_id}` | — | missing |  |

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
| `DELETE /podcasts/{id}` | `DELETE /podcasts/{id}` | covered | Unfollow; the episodes' removals reach the change feed with the show's. The owner, or a household admin for a shared show (v1: the owner). An uploaded show answers `409` until the trash moves (phase 4). |
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
| `POST /music/artists/upload-image` | — | missing |  |
| `DELETE /music/artists/image` | — | missing | Forget the picture and look it up again. |
| `GET /music/artists/image-info` | — | missing | Credit and licence of a fetched picture. |
| `GET /music/genres` | `GET /genres` | covered |  |
| `GET /music/duplicates` | — | missing |  |
| `GET /music/feedback` | — | missing | The dislike/ban undo list. |
| `POST /music/intent` | — | missing | A sentence to a smart-playlist rule (server fallback for the on-device model). |
| `GET /music/starred` | `GET /me/favourites?kind=track` | covered |  |
| `GET /music/tracks` | `GET /tracks`, `GET /sync/snapshot` | covered |  |
| `POST /music/tracks/upload` | `POST /uploads` + `POST /tracks` | covered | No multipart on v2, by design. |
| `POST /music/tracks/from-upload` | `POST /tracks` | covered |  |
| `POST /music/tracks/discs` | — | missing | Back-fills disc numbers of old uploads; could be a server job instead. |
| `GET /music/tracks/{id}` | `GET /tracks/{id}` | covered |  |
| `PUT /music/tracks/{id}` | `PATCH /tracks` | covered | One song or many, as a batch in the edit history. |
| `DELETE /music/tracks/{id}` | — | missing |  |
| `GET /music/tracks/{id}/chapters` | `GET /tracks/{id}` | covered | Chapters are in the track answer. |
| `GET /music/tracks/{id}/cover` | `Track.image` | covered |  |
| `POST /music/tracks/{id}/upload-cover` | — | missing |  |
| `GET /music/tracks/{id}/stream` | `POST /playback-info` | covered | Adds transcoding. |
| `PUT /music/tracks/{id}/visibility` | `PUT /tracks/{id}/visibility` | covered |  |
| `PUT /music/tracks/{id}/star` | `PUT /me/favourites/track/{id}` | covered |  |
| `DELETE /music/tracks/{id}/star` | `DELETE /me/favourites/track/{id}` | covered |  |
| `PUT /music/tracks/{id}/feedback` | — | missing |  |
| `DELETE /music/tracks/{id}/feedback` | — | missing |  |
| `GET /music/tracks/{id}/lyrics` | `GET /tracks/{id}/lyrics` | covered |  |
| `PUT /music/tracks/{id}/lyrics` | — | missing |  |
| `POST /music/tracks/{id}/metadata/search` | `POST /tracks/{id}/identity/search` | covered |  |
| `POST /music/tracks/{id}/metadata/apply` | `PUT /tracks/{id}/identity` | covered |  |
| `POST /music/tracks/{id}/metadata/revert` | `DELETE /tracks/{id}/identity` | covered |  |
| `GET /music/tracks/{id}/metadata/file-tags` | — | missing |  |
| `POST /music/tracks/{id}/metadata/rescan` | — | missing |  |
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
| `POST /music/playlists/{id}/upload-cover` | — | missing |  |
| `PUT /music/playlists/{id}/share` | `PATCH /playlists/{id}` | partly | v2 shares with the whole household only; chosen members and copies are missing. |
| `GET /music/playlists/{id}/audience` | — | missing | Which members can play it. |
| `PUT /music/playlists/{id}/keep` | — | missing | Keep a generated playlist. |
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
| `GET /playback/queue` | — | missing |  |
| `PUT /playback/queue` | — | missing |  |
| `GET /playback/settings` | — | missing |  |
| `PUT /playback/settings/audiobook-defaults` | — | missing |  |
| `GET /playback/settings/eq` | — | missing |  |
| `PUT /playback/settings/eq` | — | missing |  |
| `GET /stats/me` | — | missing | `GET /me/played` gives tops, not totals over a range. |
| `GET /stats/me/history` | — | missing |  |
| `PUT /stats/me/visibility` | — | missing |  |
| `GET /stats/family` | — | missing |  |
| `PUT /stats/family/members/{user_id}/visibility` | — | missing |  |
| `GET /media` | `GET /media` | covered | Phase 0: the server's signed links point at `/api/v2/media`; v1's route takes the same links. `/stream` sends a file the server keeps itself without redirecting there. |
| `PUT /media` | `PUT /media` | covered | Phase 0: the upload target for local storage. |

## Sync and library

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /library/changes` | `GET /sync/changes`, `GET /sync/snapshot` | covered | Every kind, tombstones, user state. |
| `GET /library/continue` | `GET /me/progress?kind=` | partly | One kind per call and no records along (`include=`). |
| `GET /library/search` | `GET /search`, `GET /books?q=` | partly | `/search` is music only; podcasts and episodes cannot be searched. |
| `GET /library/private` | `owner_id`, `visibility` on records | partly | No filter on the lists; a client filters its synced records. |
| `GET /library/folders` | — | missing | Server-side library folders and their last scan. |
| `POST /library/folders/scan` | — | missing |  |
| `GET /sync/tree` | `GET /sync/changes` | partly | No `path` or `sha256` on records, no rename triggers. |
| `GET /sync/tree/ids` | — | missing | An ids-only snapshot. |
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
| `GET /trash` | — | missing |  |
| `POST /trash/batches/{batch}/restore` | — | missing |  |
| `POST /trash/empty` | — | missing |  |
| `DELETE /trash/{kind}/{id}` | — | missing |  |
| `POST /trash/{kind}/{id}/restore` | — | missing |  |
| `GET /jobs` | — | missing | Instance admin. |
| `GET /jobs/{id}` | — | missing |  |

## Notifications and devices

| v1 operation | v2 | status | note |
|---|---|---|---|
| `GET /devices/notifications` | — | missing | The inbox clients poll. |
| `POST /devices/notifications/ack` | — | missing |  |
| `POST /devices/push-token` | — | missing |  |
| `DELETE /devices/push-token` | — | missing |  |
| `GET /devices/push-tokens` | — | missing |  |

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
