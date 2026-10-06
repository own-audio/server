# audio2 — Android Client Development Guide

Date: 2026-07-19
Status: living reference — endpoint list verified against the running backend.
Related:
- [mobile-backend-api-spec.md](mobile-backend-api-spec.md) — the API reference this guide builds on
- [backend-family-implementation-plan.md](backend-family-implementation-plan.md) — what exists and what does not

This guide is for building the **Android** app. It answers three questions:

1. What is the app, screen by screen?
2. Which endpoint do I call for each thing?
3. How do the tricky parts (auth, sync, offline, background) actually work?

Everything here is platform-neutral on the server side, so an iOS client
follows the same contracts — only the platform mechanics differ.

---

## 1. What we are building

A single app that plays **audiobooks, podcasts, and music** from a
self-hosted audio2 server, for a **family** where a parent decides who may
hear what.

Two things shape almost every screen:

**Every item lives in one of two folders.** *Private* items are visible only
to their owner — not even to the family admin. *Family* items are shared with
the household. The app must make it obvious which is which, because moving an
item between folders is the main privacy control a user has.

**Not everything shared is playable by everyone.** A family admin can restrict
a member per media kind, with per-item exceptions. The server simply omits
what a member may not play, so **the app never needs to filter** — but it also
means two family members legitimately see different libraries. Do not treat a
missing item as a bug or cache it as "deleted".

### Screen map

```
┌─ Home ────────────────────────────────────────────────┐
│  Continue listening   → GET /library/continue         │
│  Recently added       → from local cache, sorted      │
│  New episodes badge   → GET /devices/notifications    │
└───────────────────────────────────────────────────────┘
┌─ Library (3 tabs) ────────────────────────────────────┐
│  Audiobooks  → authors / series / collections / tags  │
│    → Generate from ebook → POST /audiobook-gen/jobs   │
│  Podcasts    → subscriptions → episodes               │
│  Music       → artists / albums / genres / playlists  │
│  ─ "Soukromé" section → GET /library/private          │
└───────────────────────────────────────────────────────┘
┌─ Player (full screen + mini) ─────────────────────────┐
│  Chapters, speed, sleep timer, bookmarks, queue       │
└───────────────────────────────────────────────────────┘
┌─ Search (all three kinds) → GET /library/search       │
└───────────────────────────────────────────────────────┘
┌─ Family ──────────────────────────────────────────────┐
│  Members, invites, per-member access (admin only)     │
└───────────────────────────────────────────────────────┘
┌─ Stats ─ personal board; family roll-up for admins    │
└───────────────────────────────────────────────────────┘
┌─ Settings ─ account, devices, downloads, playback     │
└───────────────────────────────────────────────────────┘
```

---

## 2. Conventions that apply to every call

- Base URL: `https://<host>/api/v1`. The user supplies the host — this is
  self-hosted software, there is no fixed production server. Store it and
  allow changing it (that means signing out).
- **Ask the server what it offers before showing a feature.**
  `GET /server` (public, cache for an hour) returns
  `{ name, edition, version, api: { version, revision }, features: {...},
  deprecations: [] }`. Every optional capability has a key under `features`
  and is `true` only when fully configured on that server: `auth.google`,
  `auth.apple`, `auth.microsoft`, `registration_open`, `mail`,
  `music_identify`, `podcast_discovery`, `file_sync`, `library_folders`,
  `subsonic`, `billing`, `payments`, `narration`, `translation`. Hide or
  disable the UI for a key that is `false`; ignore keys you don't know;
  treat a missing key as `false`. A server that answers **404** here is
  older than this endpoint: assume local auth and presigned uploads and
  fall back to `GET /auth/providers`. Never branch on `edition` — it is for
  bug reports. `api.revision` is the one number you may compare ("needs
  revision ≥ N"). Full policy: `own-audio-foss/docs/API_COMPATIBILITY.md`.
- **`501 { "error": "feature_unavailable", "feature": "<key>" }`** is the
  answer for a route whose feature this server does not offer (billing,
  narration or translation on an open-source server, a sign-in provider
  that is switched off). Show "this server doesn't offer X"; never retry.
- **`429 { "error": "rate_limited", "retry_after_secs": N }`** with a
  `Retry-After` header comes from the per-IP limits on `/auth/login`,
  `/auth/refresh`, `/auth/device/*`, `/join/*` and `/setup/complete`
  (defaults 30, 120, 60, 30 and 10 per minute). Wait the given seconds
  before retrying; do not count it as a wrong password.
- Auth: `Authorization: Bearer <access-jwt>` on everything except
  `/setup/*`, `/auth/login`, `/auth/register`, `/auth/refresh`,
  `/auth/registration-status`, and the two public artwork endpoints.
- All IDs are UUID strings. All timestamps are RFC3339.
- Errors are `{"error": "<message>"}` with the status carrying the meaning:
  - **400** — bad input; the message is safe to show
  - **401** — token expired/revoked → refresh, then retry once
  - **403** — authenticated but not permitted (e.g. member hitting an
    admin route)
  - **404** — missing **or not visible to you**. The server deliberately does
    not distinguish, so a private item cannot be probed for. Treat 404 on a
    known-cached item as "it went away".
    **Only the `/podcasts/*` routes actually answer 404 today.** Everywhere
    else, not-found and not-visible still arrive as a **401** (the shared
    `AuthError::NotFound`), which a client that refreshes on 401 will read as an
    expired session — a wasted refresh, and a sign-out in clients that treat a
    second 401 as terminal. Podcasts were converted first because family
    sharing makes "visible to someone else, not to you" an everyday case there;
    the other modules are the same bug, unconverted.
- Uploads are `multipart/form-data`; everything else is JSON.
- **Send multipart text fields as UTF-8.** Titles, authors, descriptions and
  the like are decoded as UTF-8, falling back to Windows-1250 for older
  Central European exports; anything that is neither is rejected with a
  **400 `form field is not valid text`**. The server used to accept such a
  field and silently replace every undecodable byte with `U+FFFD`, which
  destroyed the value on the way into the database — a rejected upload the
  client can retry is the deliberate replacement for that.

---

## 3. Authentication

### The model

Short-lived **access JWT** + long-lived **rotating refresh token**. Refresh
tokens rotate on every use: each refresh returns a *new* one and invalidates
the old. Presenting an already-used refresh token is treated as theft and the
server kills that whole device chain.

**This means storage must be transactional.** If you overwrite the stored
refresh token before you know the response was persisted, a crash between the
two leaves the app holding a token the server has already retired, and the
user is logged out for good.

```kotlin
// DataStore or EncryptedSharedPreferences; write both tokens atomically.
suspend fun onRefreshSuccess(res: LoginResponse) {
    tokenStore.updateData { it.copy(access = res.token, refresh = res.refreshToken) }
}
```

### Endpoints

| Call | Endpoint | Notes |
|---|---|---|
| Is the server set up? | `GET /setup/status` | First run; if `setup_complete` is false the app can offer to create the first admin. |
| Can I self-register? | `GET /auth/registration-status` | Hide the register button when false — **unless** the user has an invite code. |
| Log in | `POST /auth/login` | Send `device_kind: "android"` and a `device_name` (e.g. `Build.MODEL`) so the user recognises it in their device list. Accepted kinds: `web`, `ios`, `android`, `macos`, `windows` — anything else is stored as `other` and can never be reclassified, so a new client platform must be added server-side (allowlists in `auth`/`playback` **and** the CHECK constraints — see migration `0064`) before it first logs in. |
| Register | `POST /auth/register` | Accepts `invite_code`, which works **even when registration is closed**. This is the normal way a family member joins. |
| Refresh | `POST /auth/refresh` | `{refresh_token}` → new pair. |
| Log out | `POST /auth/logout` | Revokes this device's session **and** its refresh chain. Also `DELETE /devices/push-token` first. |
| My devices | `GET /auth/sessions` | Shows `current: true` for this device. |
| Sign out another device | `DELETE /auth/sessions/{chain_id}` | |
| Change password | `POST /auth/password` | Keeps this device signed in, drops all others. |

Login and register both return:

```json
{ "token": "<jwt>", "refresh_token": "<hex>",
  "user": { "id": "…", "email": "…", "display_name": "…", "role": "user" } }
```

### OkHttp wiring

```kotlin
class AuthInterceptor(private val store: TokenStore) : Interceptor {
    override fun intercept(chain: Interceptor.Chain): Response {
        val req = chain.request().newBuilder()
            .header("Authorization", "Bearer ${store.access()}")
            .build()
        return chain.proceed(req)
    }
}

// Authenticator handles 401 once; OkHttp will not loop forever on it.
class RefreshAuthenticator(private val store: TokenStore,
                           private val api: AuthApi) : Authenticator {
    override fun authenticate(route: Route?, response: Response): Request? {
        if (responseCount(response) >= 2) return null      // already retried
        synchronized(this) {
            val current = store.access()
            // Another thread may have refreshed while we waited.
            if (response.request.header("Authorization") != "Bearer $current") {
                return response.request.newBuilder()
                    .header("Authorization", "Bearer $current").build()
            }
            val res = runCatching { api.refreshBlocking(store.refresh()) }.getOrNull()
                ?: run { store.clear(); return null }        // chain is dead → re-login
            store.save(res)
            return response.request.newBuilder()
                .header("Authorization", "Bearer ${res.token}").build()
        }
    }
}
```

**A 401 from `/auth/refresh` itself is terminal**: the chain was revoked
(password change elsewhere, admin action, or reuse detection). Clear storage
and send the user to the login screen — do not retry.

`users.role` (`admin`/`user`) is **instance-wide** and unrelated to the
family role. For family permissions read `my_role` from `GET /family`.

---

## 4. Sync strategy — read this before designing the data layer

The app should be **offline-first**: a local Room database is the source of
truth for the UI, and the network updates it. Do not bind screens to network
calls.

### The one endpoint that matters

`GET /library/changes?since=<rfc3339>` returns everything that changed since
a cursor, in one request:

```json
{
  "since": "2026-07-19T08:00:00.000Z",
  "now":   "2026-07-19T09:15:22.481Z",
  "full_sync": false,
  "audiobooks": [ … ], "podcasts": [ … ], "tracks": [ … ],
  "deleted": [ { "media_kind": "music", "item_id": "…", "deleted_at": "…" } ]
}
```

Rules that will save you debugging time:

- **Omit `since` on first run.** You get a full snapshot with
  `full_sync: true` and an empty `deleted`. No separate bootstrap path.
- **Store `now` only after the transaction commits.** If you save the cursor
  first and the write fails, those changes are lost forever.
- **`deleted` is the whole point.** A row that vanished is otherwise
  indistinguishable from one you already hold. Apply tombstones or you will
  accumulate ghosts.
- **Tombstones are pruned after 90 days.** If your stored cursor is older,
  discard the local cache and do a full sync.
- **Expect repeats.** `now` is captured before the queries run, so an item
  written mid-request comes again next time. Upsert, never blind-insert.
- **Echo `now` back verbatim.** It is handed out in `Z` form on purpose: an
  RFC3339 `+00:00` offset becomes a space when placed in a query string
  unencoded, which breaks the next sync.

```kotlin
suspend fun sync() {
    val cursor = syncStore.cursor()                       // null on first run
    val res = api.changes(cursor)
    db.withTransaction {
        res.audiobooks.forEach { dao.upsertBook(it.toEntity()) }
        res.podcasts.forEach   { dao.upsertFeed(it.toEntity()) }
        res.tracks.forEach     { dao.upsertTrack(it.toEntity()) }
        res.deleted.forEach    { dao.deleteItem(it.mediaKind, it.itemId) }
        syncStore.setCursor(res.now)                      // inside the transaction
    }
}
```

`/library/changes` covers top-level items only. Fetch **episodes**, **files**,
and **chapters** per parent when the user opens it, and cache them.

### Detail endpoints

| Need | Endpoint |
|---|---|
| Episodes of a feed | `GET /podcasts/{id}/episodes?limit=&offset=` — carries this user's `progress_secs` and `completed` inline, plus `size_bytes`/`sha256` (both nullable — `null` whenever `has_local` is false, and `sha256` can still be `null` even when `has_local` is true if the async checksum job hasn't caught up yet) and `has_transcript` (§11a — gates whether translation can be offered for that episode) |
| Files of a book | `GET /audiobooks/{id}/files` — `position` is the play order; each file also carries `size_bytes` (nullable `i64`, from `media_objects`) for clients that want to show storage size, and `sha256` (nullable — same async-job caveat as episodes above) for verifying a downloaded copy against the server's checksum. No book-level total is provided — sum the files' own `size_bytes` client-side, the same way a book-wide duration can be summed from files when `total_duration_secs` isn't precise enough. |
| Chapters | `GET /audiobooks/{id}/chapters` — **may be empty**; see §11 |
| Playlist entries | `GET /music/playlists/{id}/tracks` |

---

## 5. Library screens

### Audiobooks

| Purpose | Endpoint |
|---|---|
| List | `GET /audiobooks` |
| Detail | `GET /audiobooks/{id}` |
| Cover | `GET /audiobooks/{id}/cover` (needs auth; feed Coil an `OkHttpClient` with the interceptor) |
| Files / chapters | `GET /audiobooks/{id}/files`, `/chapters` |
| Authors, tags | `GET /audiobooks/authors`, `/authors/{id}/books`, `/authors/tags/book/{id}` |
| Series, collections | `GET /audiobooks/organize/series`, `/collections` |
| Favorites | `GET|POST|DELETE /audiobooks/organize/favorites[/{book_id}]` |
| Share / unshare | `PUT /audiobooks/{id}/visibility` `{"visibility":"family"|"private"}` |

A book response:

```json
{ "id": "…", "title": "…", "author": "…", "narrator": "…", "description": "…",
  "cover_url": "/api/v1/audiobooks/…/cover", "total_duration_secs": 43200,
  "visibility": "family", "is_owner": true,
  "created_at": "…", "updated_at": "…" }
```

`is_owner: false` means playable but **not editable** — hide edit, delete, and
the visibility toggle. The server refuses them anyway — as a **401** for
audiobooks (see the error table above), a 404 for podcasts.

### Podcasts

| Purpose | Endpoint |
|---|---|
| Subscriptions | `GET /podcasts` |
| Search directory | `POST /podcasts/search` `{"q":"…", "language":"en"}` — the self-hosted catalogue since 2026-08-29, not a third-party proxy. `language` optional, and a **subtag** (`en`, not `en-US`). |
| Browse by category | `GET /podcasts/discover/categories`, then `GET /podcasts/discover/browse?category=…` |
| Shows like this one | `GET /podcasts/{id}/similar` — empty list when the feed is not in the catalogue |
| Discovery languages | `users.discovery_languages` (in `/auth/me`, set with `PATCH /users/me`) filters **all three** of the above. Empty means every language. A `language` on search or browse overrides it for that one request; `similar` has no override and falls back to the seed show's own language when the list is empty. |
| Subscribe | `POST /podcasts/subscribe` `{"feed_url":"…","visibility":"family"}` — also accepts a YouTube channel URL |
| Episodes | `GET /podcasts/{id}/episodes?limit=50&offset=0` |
| Refresh | `POST /podcasts/{id}/refresh` |
| Search a show's episodes | `GET /podcasts/{id}/episodes?q=` — title and description, any case, accents ignored |
| Store the back catalogue | `POST /podcasts/{id}/store-all` `{"preview":true}` first for the count and size, then `{}` for all or `{"latest":100}` |
| Keep new episodes | `PUT /podcasts/{id}/auto-store` `{"enabled":true}` — the server stores every episode published from now on by itself (paid feeds whose links expire). Subscriber, or a family admin for a shared show. Feeds carry `auto_store`; the list and single-feed reads also carry `has_transcripts` (mark the show as translatable). |
| Unsubscribe | `DELETE /podcasts/{id}` |
| Artwork | `GET /podcasts/{id}/image`, `/episodes/{ep_id}/image` — **no auth**, safe for plain Coil |

`DELETE /podcasts/{id}/episodes/{ep_id}/download` moves that server-side copy
to the trash (§8b) — **the subscriber, or a family admin when the feed is
shared**, because it takes the copy away from the whole household. The
episode itself stays listed; only `has_local` goes back to false. The podcast
app calls it when someone marks an episode "Not Interested".

**Episodes are not streamable until downloaded server-side.** An episode
carries `has_local`; when false, call
`POST /podcasts/{id}/episodes/{ep_id}/download` (server fetches it into
storage) before `GET …/stream`. That is a *server-side* download shared by the
whole household — it is not the same as caching the file on the phone, which
you must implement separately (§8).

### Music

| Purpose | Endpoint |
|---|---|
| Tracks | `GET /music/tracks` — each track carries `size_bytes`/`sha256` (both nullable, from `media_objects`; `sha256` can lag behind upload — same async checksum job as audiobook files and podcast episodes) |
| Artists / albums / genres | `GET /music/artists`, `/music/albums?artist=`, `/music/genres` |
| Playlists | `GET|POST /music/playlists` (create accepts `track_ids` in order, and `generated: true` for one saved from a smart playlist), `GET /music/playlists/{id}/tracks`, `PUT /music/playlists/{id}/keep` for a generated one the listener keeps |
| Share a playlist with family members | `GET /music/playlists/{id}/audience`, `PUT /music/playlists/{id}/share` `{mode: "live"|"copy", user_ids}`; the recipient's inbox gets `playlist_shared` with `data.playlist_id` |
| Favourite songs (heart) | `GET /music/starred` (ids), `PUT`/`DELETE /music/tracks/{id}/star` |
| Add / remove / reorder | `POST` `/tracks`, `DELETE …/tracks/{entry_id}`, `PUT …/tracks/reorder` |
| Identify via MusicBrainz | `POST /music/tracks/{id}/metadata/search`, `POST …/metadata/apply` |

**Owner (2026-09-26).** Books, tracks, feeds and their `/library/changes` rows carry
`owner_id` next to `is_owner` — the member to name in `POST /sync/shortcuts` when adding one
of their books, albums or shows to the own.audio folder.

**Discs (2026-09-26).** Tracks carry `disc_number` and `disc_total` (nullable, from the
file's tags) in `TrackResponse`, `/library/changes` (`disc_number`) and Subsonic
(`discNumber`). Sort an album's tracks by disc, then track number, and head each disc
("CD 1", "CD 2") when the album has more than one. `POST /music/tracks/discs` reads the disc
of tracks uploaded before 0.1.44.

Use the grouped endpoints rather than grouping a flat list client-side: they
apply the same visibility rules and give you counts for free.

**Album artist (2026-09-24).** An album is a release, not an artist: a guest on
one track or a compilation must not split it. Every track carries
`album_artist` — in `TrackResponse` and in `/library/changes` — which is the
explicit value (file tag, compilation → `"Various Artists"`, manual edit) or,
when none is set, the track `artist` without its guests (`feat.`, `ft.`,
`featuring` and what follows are cut; `&` is kept). `GET /music/albums` and
`GET /music/artists` group by it, so an album summary's `artist` **is** its
album artist, and `?artist=` filters on it. The artist list also includes
artists who only **appear on** someone else's album (a singer on a hits
compilation): their `album_count` is 0, `track_count` counts their tracks —
show those albums as "Appears on" on the artist screen. **An album's tracks are the ones
whose `album_artist` and `album` both match** — never filter by `artist`, or
"George Ezra feat. First Aid Kit" drops off George Ezra's album. Show
`album_artist` on the album header and each track's own `artist` on its row.
`PUT /music/tracks/{id}` takes an optional `album_artist`: absent keeps the
stored value, `null` or `""` clears it back to derived. Upload accepts it as a
multipart field and otherwise reads the file's album-artist tag and
compilation flag. Applying a MusicBrainz match sets it to the release's
artist credit, so "Saviour" (credited to "George Ezra feat. First Aid Kit")
lands on George Ezra's album and a hits compilation under "Various Artists";
tracks identified earlier are filled in by a background sweep, which bumps
`updated_at` only when the value changes. See `docs/album-artist-plan.md`.

**Metadata identification.** `search` takes a JSON body `{title?, artist?,
album?, limit?}` (at least one of `title`/`artist`/`album` required, 400
otherwise) and proxies a MusicBrainz recording search — the backend holds the
MusicBrainz API key/user-agent and rate-limits to ~1 req/s process-wide, so no
client needs its own rate-limit handling. Each candidate carries
`mb_recording_id`, `mb_release_id`, and a **speculative, unverified**
`cover_art_url` (may 404 — don't treat a broken image as an error). `apply`
takes `{mb_recording_id, mb_release_id?, fetch_cover?}` (`fetch_cover`
defaults `true`), re-fetches the chosen recording server-side rather than
trusting client-cached search-result fields, and returns the updated
`TrackResponse` — now carrying `musicbrainz_recording_id`, which is `null`
until a track has been matched. A failed cover fetch never fails the apply;
the metadata update alone is the operation that can fail. Owner-only, same
as `PUT /music/tracks/{id}`. Verified live 2026-08-20 against the real
MusicBrainz + Cover Art Archive APIs with a real track.

### Private folder and search

- `GET /library/private` → `[{kind, id, title, subtitle}]`, the user's
  never-shared items. Always self-scoped.
- `GET /library/search?q=&limit=` → a **tagged union**; switch on `kind`,
  whose values are capitalised: `"Feed"`, `"Episode"`, `"Book"`, `"Track"`.

```kotlin
@JsonClassDiscriminator("kind")
@Serializable sealed interface SearchResult {
    @Serializable @SerialName("Book")    data class Book(val id: String, val title: String, val author: String?) : SearchResult
    @Serializable @SerialName("Track")   data class Track(val id: String, val title: String, val artist: String?, val album: String?) : SearchResult
    @Serializable @SerialName("Feed")    data class Feed(val id: String, val title: String, val author: String?, val description: String?) : SearchResult
    @Serializable @SerialName("Episode") data class Episode(val id: String, val feedId: String, val title: String, val feedTitle: String?, val publishedAt: String?) : SearchResult
}
```

`GET /library/continue` uses the same tagged shape with `"Episode"` / `"Book"`.
For `"Book"`, `position_secs` is **book-wide** — the durations of every file
before the one actually being tracked, plus the stored position within it —
not the raw per-file value `PUT /playback/books/{id}/progress` writes. Divide
directly by `total_duration_secs` for a percentage; do not also add file
offsets, that's already done server-side.

### Pulling everyone's book positions back down

`GET /playback/books/progress` returns **every book this user has started**, in
one request:

```json
[{ "book_id": "…", "position_secs": 5310.0, "file_id": "…",
   "file_position_secs": 212.0, "completed": false,
   "updated_at": "2026-09-21T09:14:03+00:00" }]
```

This is the only way a client learns that the listener got further on *another*
device. `GET /library/changes` carries no progress — positions are per-listener
and the library is shared with the family — so a shelf built from `changes`
alone shows every book as unstarted until this endpoint is called. Call it on a
cold start and on every library refresh.

- **`position_secs` is book-wide**, same rule as `/library/continue`: divide by
  `total_duration_secs`, never add file offsets on top.
- **`file_position_secs` + `file_id` is what you resume to.** Both come back
  because neither converts into the other without the book's file list, and a
  shelf that has not opened a book does not have one.
- **A device with an unsent position of its own compares `updated_at` first.**
  Newest write wins; do not let a pull overwrite a local save that has not
  reached the server yet.

---

## 6. Playback

### Getting audio

Every stream endpoint returns a **presigned URL**, not bytes:

```json
{ "url": "https://storage.example/…?X-Amz-Signature=…", "expires_in_secs": 14400 }
```

| Kind | Endpoint |
|---|---|
| Music | `GET /music/tracks/{id}/stream` |
| Audiobook file | `GET /audiobooks/{id}/files/{file_id}/stream` |
| Podcast episode | `GET /podcasts/{id}/episodes/{ep_id}/stream` |

Consequences for the player:

- **URLs expire after 4 hours.** A paused audiobook resumed the next morning
  has a dead URL. Re-request the stream endpoint on resume, and on any 403
  from storage, instead of caching URLs.
- The presigned URL needs **no** `Authorization` header — do not attach one,
  it can break the signature check.
- **There is no transcoding.** The original file is what you get; no
  lower-bitrate variant exists for poor connections.
- Range requests work (they come from S3-compatible storage), so seeking is
  fine.

### Player stack

Media3/ExoPlayer inside a `MediaSessionService`, which gives you the
notification, lock-screen controls, Bluetooth/headset buttons, and Android
Auto in one place.

- **Multi-file audiobooks**: build a `ConcatenatingMediaSource` from
  `/files` ordered by `position`. Resolve each file's URL lazily as you
  approach it, so early URLs do not expire during a long listen.
- **Chapters**: `GET /audiobooks/{id}/chapters` gives
  `{position, title, start_time_secs, file_id}`. May be empty (§11) — fall
  back to per-file navigation.
- **Speed and skip defaults**: `GET /playback/settings` returns
  `playback_speed`, `skip_intro_secs`, `skip_outro_secs`,
  `ab_playback_speed`, `ab_skip_forward_secs` (default 30),
  `ab_skip_backward_secs` (default 15). Use the `ab_*` values in the
  audiobook player. Write back with
  `PUT /playback/settings/audiobook-defaults`.
- **Sleep timer** is entirely client-side.

### Saving position

| Kind | Endpoint | Body |
|---|---|---|
| Audiobook | `PUT /playback/books/{book_id}/progress` | `{position_secs, completed, file_id, device_kind}` — **`file_id` is required** |
| Podcast | `PUT /playback/episodes/{episode_id}/progress` | `{position_secs, completed, device_kind}` |
| Music | `PUT /music/tracks/{id}/progress` | `{position_secs, completed}` |
| Music | `PUT /music/tracks/{id}/feedback` | `{kind}` — `dislike` or `banned` |

Save every ~15–30 s of playback and always on pause, stop, and track change.
Send `device_kind: "android"` so statistics can attribute correctly.

### Bookmarks

`GET|POST /playback/bookmarks`, `PUT|DELETE /playback/bookmarks/{id}`,
`GET /playback/books/{book_id}/bookmarks`. Create with
`{book_id | episode_id, file_id, position_secs, label}`.

### Cross-device queue

`GET|PUT /playback/queue`. Items are
`{media_kind, item_id, part_id?}`; the request also takes `current_index`,
`position_secs`, `device_kind`, and optional `device_label`; the response
carries the same plus `updated_at`, `updated_by_device`, and
`updated_by_device_label`.

`device_kind` here is normalized the same as every other endpoint — send
`"android"`, never a device name or model id. `device_label` is a *separate*,
optional, free-form human-readable name (e.g. "Kornel's Pixel", from
`Settings.Global.DEVICE_NAME`) used only to distinguish two devices of the
same `device_kind` on one account. **Omit it entirely** when no real
user-set name is available — do not fall back to `Build.MODEL` or similar;
a raw model id in this field ends up shown verbatim in another client's
follow-prompt UI.

Semantics are **last-write-wins**. Read before writing when the app comes to
the foreground: if `updated_at` moved and `updated_by_device` (or, when both
sides have one, `updated_by_device_label`) is not this device, another
device took over — offer to follow it rather than silently overwriting. An
out-of-range `current_index` is clamped, not rejected.

---

## 7. Listening statistics

Two ways to feed the stats board. **Pick one — do not do both**, or you will
double-count.

**Recommended for a real player: report sessions explicitly.**

```jsonc
POST /playback/sessions
{ "sessions": [ {
    "media_kind": "audiobook",
    "item_id": "<book/feed/track id>",
    "part_id": "<file or episode id>",
    "started_at": "2026-07-19T09:00:00Z",
    "ended_at":   "2026-07-19T09:12:30Z",
    "seconds_listened": 720,        // audio consumed, not wall clock
    "playback_speed": 1.5,
    "device_kind": "android",
    "ended_reason": "skipped",      // optional — see below
    "client_session_id": "<uuid>"   // REQUIRED in practice — see below
} ] }
```

`client_session_id` is what makes retries safe. WorkManager re-runs failed
jobs, and a batch that reached the server before the response was lost would
otherwise be counted twice. Generate the UUID when the span closes, persist it
*with* the span, and reuse it on every attempt. The response reports
`recorded` vs `received`; `recorded: 0` on a retry is **success**.

Batch aggressively: accumulate spans in Room and flush up to 500 at a time
when connectivity allows. Do not post per span.

**`ended_reason` — why playback stopped.** Optional, one of `completed`,
`skipped`, `stopped` or `replaced`. Only the client knows: a twelve-second
span might be a skip, a phone call, a process the OS killed, or someone
sampling a track deliberately, and nothing server-side can tell those apart
afterwards.

| Value | When |
|---|---|
| `completed` | reached the end, or within the last few seconds |
| `skipped` | the user explicitly advanced to something else |
| `stopped` | ended without a skip — paused and abandoned, app killed, interrupted |
| `replaced` | navigated away and started something unrelated, not a next-track skip |

Two rules:

1. **Send the fact, not a judgement.** Do not decide whether a skip "counts".
   How much an early skip weighs against a track is derived here from
   `seconds_listened`, so that curve can be retuned without an app update on
   every platform.
2. **Omit it when genuinely unknown, and prefer `stopped` when unsure.** An
   unrecognised value is dropped rather than bucketed. A missing reason costs a
   little signal; a wrong one teaches the preference model something false.

Instrument this at the media-session layer, not in your UI — lock screen,
Bluetooth buttons, Android Auto, CarPlay and widgets all advance tracks, and a
skip reported as `stopped` because it came from a headset button is a silently
wrong signal nothing will flag.

### Explicit feedback: two buttons, not one

`PUT /music/tracks/{id}/feedback` with `{"kind": "dislike"}` or
`{"kind": "banned"}`; `DELETE` the same path to undo.

| Kind | Meaning |
|---|---|
| `dislike` | a strong negative preference. Heavily down-weighted, but it recovers over months |
| `banned` | never selected automatically, until the user undoes it |

Ship both, and label them distinctly. One control conflating them makes
"never again" impossible to say and "not right now" impossible to take back —
a track that did not suit the moment is not a track the user never wants to
hear again.

**Neither deletes anything.** The track stays in the family library, stays
visible in your browse views, and is untouched for every other family member;
only this user's automatic selection skips it. It is a preference, not a
permission — **do not draw a lock icon, do not hide the row, do not offer it as
a way to remove content.**

`GET /music/feedback` (optionally `?kind=banned`) returns the list. **Render it
somewhere in settings.** Nothing about a banned track looks different in the
library, so without that list a mis-tap is unrecoverable.

`DELETE` is idempotent — clearing feedback that was never set is success.
Feedback on a track the caller cannot see is `404`.

**The alternative — do nothing.** If a client never reports sessions, the
server derives them from progress saves. Simpler, but it cannot tell a pause
from a seek. Once you start reporting explicitly, derivation switches off for
that user automatically.

Reading stats:

| Call | Endpoint |
|---|---|
| Personal board | `GET /stats/me?range=7d\|30d\|90d\|365d\|all&tz_offset_minutes=…` |
| History log | `GET /stats/me/history?limit=&offset=` |
| Privacy toggle | `PUT /stats/me/visibility` `{"stats_visibility":"private"\|"family_admin"}` |
| Family roll-up | `GET /stats/family?range=…` (family admins; same ranges) |

Always send `tz_offset_minutes` (minutes **east** of UTC):

```kotlin
val tz = TimeUnit.MILLISECONDS.toMinutes(
    TimeZone.getDefault().getOffset(System.currentTimeMillis()).toLong()).toInt()
```

Without it, day buckets and the streak are computed in UTC and will look wrong
to anyone not on UTC.

Response:

```json
{ "range": "7d", "total_seconds": 1800,
  "by_kind": [ {"media_kind":"audiobook","seconds":1800,"sessions":3} ],
  "by_day":  [ {"day":"2026-07-19","seconds":1800} ],
  "by_day_kind": [ {"day":"2026-07-19","media_kind":"audiobook","seconds":1800} ],
  "top_items":[ {"media_kind":"audiobook","item_id":"…","title":"…","seconds":1800,"sessions":3} ],
  "streak_days": 1,
  "completed_items": 2 }
```

`by_day_kind` is `by_day` split by media kind (days without listening are left out) —
what the web Home's 12-month activity grid draws, one row per cloud. Older servers
do not send it.

`completed_items` is a lifetime count of finished audiobooks, unaffected by `range` —
`audiobook_progress` is one row per (user, book), so its `completed` flag is already a
whole-book signal, not something that needs deriving from per-file state.

In `GET /stats/family`, members who keep their stats private appear with
`hidden: true` and **no figures** — render the row, omit the numbers.

---

## 8. Offline downloads

Server-side "download" (podcasts) and on-device caching are **different
things**. The app needs its own download manager; the backend has no concept
of "downloaded to this phone".

Recommended shape:

- WorkManager (`NetworkType.UNMETERED` when the user asks for Wi-Fi only)
  fetching the presigned URL to app-private storage.
- A Room table of downloads: item id, part id, local path, state, bytes.
- ExoPlayer resolves a local file when present, otherwise the network.
- Eviction: a user-set size cap, plus "delete played episodes after N days".
- Re-request the stream URL inside the worker — a URL queued hours ago is
  expired.

There is no server-side per-feed auto-download rule, so "download new
episodes automatically" is a client feature: sync, diff against what you
have, enqueue.

---

## 8b. Deleting: the 30-day trash

Every delete of a book, a track, a playlist or a stored episode copy moves it
to the **trash** for 30 days; after that the server deletes it for good. The
routes and their `204` answers are unchanged — a client that only deletes
keeps working — but:

- **Who may delete** changed: the owner, **or a family admin** when the item is
  shared with their family. A member who can see an item but may not delete it
  gets `403`; one who cannot see it gets `404`. A member's *private* items stay
  invisible to admins, so `404` there too.
- The item disappears from every list at once and `/library/changes` carries
  the usual tombstone (`media_kind` `audiobook`/`music`/`playlist`), so drop
  your local copy — including any file downloaded to the phone.
- Send **`X-Trash-Batch: <uuid>`** on every delete of one user gesture (a
  multi-select delete, "delete album"). The trash can then restore the whole
  deletion with one call. Without it each delete is its own batch.

| Call | Endpoint |
|---|---|
| List | `GET /trash` — the caller's own items; `?scope=family` (family admins) — everything shared with the family |
| Restore | `POST /trash/{kind}/{id}/restore` → `{"restored":1,"charged_micro":0}` |
| Restore a whole deletion | `POST /trash/batches/{batch}/restore` |
| Delete forever now | `DELETE /trash/{kind}/{id}` |
| Empty | `POST /trash/empty` (`?scope=family` for admins) |

`kind` is `audiobook`, `music_track`, `playlist`, `podcast_episode` or
`companion_file` (an image, booklet or lyrics file kept next to the audio by
the desktop sync — show it with a generic file icon and its `title`, which is
the file name). A row:

```json
{ "kind": "music_track", "id": "…", "title": "Hello",
  "owner": {"id": "…", "display_name": "Petr"},
  "trashed_by": {"id": "…", "display_name": "Mum"},
  "trashed_at": "…", "purge_at": "…", "size_bytes": 5242880,
  "batch": "…", "restore_charge_micro": 0 }
```

**Restoring charges the days spent in the trash** (the trash is free unless the
item comes back — this stops anyone trashing everything before the daily
storage charge and restoring it after). `restore_charge_micro` is what a
restore would cost right now; **show it before the user confirms** ("Restoring
charges 12 days in the trash: $0.04"). Undo within the same day is free. The
charge appears in the ledger as `trash_restore_charge`.

When a family admin deletes someone else's item, the owner gets an
`item_trashed` notification (§9) with `data: {kind, id, title, by, purge_at}`.

## 8a. Uploading large files

The multipart upload routes push the bytes through the API. On the hosted
instance the API sits behind a proxy that **rejects a request body over
100 MB**, so `POST /audiobooks/upload`, `/upload-file` and `/upload-cover`
return 413 for a real audiobook. On a self-hosted instance with no such proxy
they are still fine.

Upload straight to object storage instead:

1. `POST /uploads/presign` `{"kind":"audiobook_file","filename":"01.m4a",
   "content_type":"audio/mp4","size_bytes":123456789}` →
   `{object_key, url, method:"PUT", content_type, expires_in_secs}`.
2. `PUT` the bytes to `url`. Send **exactly** the `content_type` from the
   response — it is signed, so a different value (or OkHttp inferring one)
   fails with a signature mismatch. Send **no** `Authorization` header; the
   URL carries its own credentials. This is also where OkHttp's request-body
   progress hooks belong, since the upload no longer goes through the API.
3. `POST /audiobooks/from-uploads` with `{title, author, narrator,
   description, visibility, cover_object_key?, files:[{object_key,
   relative_path, title?, duration_secs?}]}`.

As with the multipart manifest, **`relative_path` decides play order** — the
server sorts, so files may be uploaded in parallel and in any order. Keys are
minted server-side; one from another family is refused with the API's generic
not-found error, which here still arrives as a **401**, not a 404 — don't let
that log the user out. (The `/podcasts/*` routes now answer 404 instead; see the
error table above.) A presigned PUT is valid 6 hours, and a single
object may not exceed 5 GiB.

A single track works the same way: presign with kind `music_track`, PUT, then
`POST /music/tracks/from-upload` `{"object_key":"…","original_filename":"song.flac",
"visibility":"family"}` — tags are read from the stored file exactly as for
the multipart upload. Both answers carry a `path`: where the item sits in the
desktop apps' own.audio folder. Phones can ignore it.

Retrying is cheap: re-presign and re-PUT only the files that failed, then call
`from-uploads` once at the end. The book is created in a single transaction,
so a failure there leaves no half-built book behind.

---

## 9. Notifications

| Call | Endpoint |
|---|---|
| Register | `POST /devices/push-token` `{"platform":"fcm","token":"…","device_name":"…"}` |
| Unregister | `DELETE /devices/push-token` `{"token":"…"}` — on sign-out |
| List devices | `GET /devices/push-tokens` |
| **Inbox** | `GET /devices/notifications` |
| Acknowledge | `POST /devices/notifications/ack` `{"ids":[…]}` |

Register on **every app start** (FCM tokens rotate) and after
`onNewToken`. Re-registering the same token is idempotent and moves it
between accounts if the phone changed hands.

> ⚠️ **Push delivery is not implemented on the server yet.** Registration, the
> queue, and the triggers exist, but nothing sends to FCM today. **Poll
> `GET /devices/notifications` on app resume** — it carries the same
> information (currently "new episodes" per feed), and entries survive until
> acknowledged. When the sender lands, no client change is needed; you will
> just start receiving the same payloads via FCM.

Notification shape:

```json
{ "id": "…", "kind": "new_episodes", "title": "<feed title>",
  "body": "3 new episodes",
  "data": {"media_kind":"podcast","item_id":"<feed id>"},
  "created_at": "…" }
```

Use `data` to deep-link; acknowledge once handled.

**Kinds emitted today:**

| `kind` | When | `data.item_id` |
|---|---|---|
| `new_episodes` | A subscribed feed gained episodes | feed id |
| `generation_complete` | An AI narration job finished (§11) | published book id |
| `generation_failed` | An AI narration job failed (§11) | `null` — no book exists |
| `podcast_translation_complete` | A personal-use episode translation finished (§11a) | the *episode* id, not the translation id |
| `podcast_translation_failed` | An episode translation failed (§11a) | the episode id |

`title` is the book's title for both generation kinds, and the **episode's** title for both
podcast-translation kinds. `data.generation_job_id` is always present on the generation
kinds so a client can deep-link back to its own status screen rather than the library; the
translation kinds carry `data.podcast_episode_translation_id` and `data.item_id` (the
episode id) instead — see §11a. **Only the requester is notified**, never the rest of the
family, for all four of these — each is someone's own job, not shared content arriving.

This matters more than the podcast case: a narration job runs for minutes to hours, and the
status endpoint is only polled while the client's status screen is open. Without this inbox
entry there is nothing at all to tell a user who closed the screen that their book is ready.

---

## 10. Family and parental controls

| Purpose | Endpoint | Who |
|---|---|---|
| My family | `GET /family` | anyone |
| Rename | `PUT /family` | family admin |
| Members | `GET /family/members` | anyone |
| Role / label | `PUT /family/members/{user_id}` | family admin |
| Remove / leave | `DELETE /family/members/{user_id}` | admin, or self |
| Invites | `GET|POST /family/invites`, `DELETE /family/invites/{id}` | family admin |
| Accept invite | `POST /family/invites/accept` `{"code":"…"}` | signed-in user |
| A member's limits | `GET /family/members/{user_id}/access` | family admin |
| Set default | `PUT /family/members/{user_id}/policy` `{"media_kind":"music","policy":"deny_all"}` | family admin |
| Item exceptions | `PUT /family/members/{user_id}/grants` `{"media_kind":"audiobook","allow":[ids],"deny":[ids]}` | family admin |
| Who can hear this | `GET /family/content/{kind}/{item_id}/audience` | family admin |
| Change who can hear this | `PUT /family/content/{kind}/{item_id}/audience` | family admin |
| Ask for access | `POST /family/content/{kind}/{item_id}/request-access` | any member |

`GET /family` returns `my_role` (`family_admin` | `member`) — gate the admin
UI on that, not on `users.role`.

Onboarding a family member: admin creates an invite → shares the `code` out of
band → the invitee registers with `invite_code` (works on a closed instance)
or, if already signed up, calls `/family/invites/accept`. Codes are single-use,
bound to the invited email, and expire after 14 days.

Grants are a **bulk replace** per media kind: send the complete allow/deny
lists as shown in the UI, not deltas.

### Storage & credit

| Purpose | Endpoint | Who |
|---|---|---|
| Storage, cost, balance | `GET /family/billing` | anyone |
| Alert thresholds | `GET\|PUT /family/billing/alerts` | family_admin |

There is no Stripe integration yet — this is **play money**: every family
starts with a pooled $5-per-member welcome credit, and a daily background job
debits it for that day's storage at $0.05/GB/month. All amounts are
micro-USD integers (`1_000_000` = $1); divide by 1,000,000 for display, don't
round to cents before dividing or a sub-cent daily charge disappears.

Show, per the response shape (§4c of `mobile-backend-api-spec.md`): storage
used (with the audiobooks/podcasts/music/other breakdown), cost per day and
per month, the credit balance, and — when `days_remaining`/`runs_out_on`
aren't null — the date the credit runs out. Treat `depleted: true` like a
low-battery indicator: **display it, never act on it.** No client should
block playback, block uploads, or otherwise gate any action on this field —
nothing is enforced while the credit is play money, and that may change only
with an explicit, separately-announced payments rollout.

**Credit alerts** (family_admin only): a "notify me when…" preference with
two independent, optional rules — below a dollar amount, or below a number of
days remaining. A background sweep evaluates them daily and delivers a
`credit_low` notification through the existing in-app inbox
(`GET /devices/notifications`) — **there is no push**, so don't word the UI as
if a system notification will arrive. `balance_alert_active` /
`days_alert_active` in the response are edge-trigger latches (has this
already fired, not "is it currently low" — compute the current state
yourself from `GET /family/billing`); the sweep fires once per
above→below crossing and stays silent until the family recovers and drops
below threshold again. Same rule as `depleted`: informational only, never
enforced.

---

## 11. Audiobook generation (AI narration)

Turns an uploaded ebook into a narrated audiobook: upload → (optional
translate) → LLM chapterize/clean → per-chapter TTS → assemble → publish into
the normal audiobook library (§5). It is a **background job the app polls**,
not a synchronous call — expect it to take minutes to hours depending on book
length.

### Flow

```
1. GET  /audiobook-gen/languages               → source/target language picker
2. GET  /audiobook-gen/voices                  → pick a voice
3. POST /audiobook-gen/estimate  (multipart)   → estimated_char_count
4. POST /audiobook-gen/quote     (json)        → priced quote for that voice
5. POST /audiobook-gen/jobs      (multipart)   → creates + starts the job
6. GET  /audiobook-gen/jobs/{id}               → poll until stage is
                                                  "complete" or "failed"
```

Steps 2–3 are separate calls so the wizard can re-quote when the user changes
voice without re-uploading the file. Step 4 re-uploads the same file — there
is no "commit the estimated upload" shortcut, so warn on cellular for large
files same as the audiobook upload flow (guide §5).

### 1. Languages

`GET /audiobook-gen/languages` →
```json
[{"code":"cs","label":"Czech"},{"code":"en","label":"English"},
 {"code":"de","label":"German"},{"code":"it","label":"Italian"},
 {"code":"fr","label":"French"},{"code":"es","label":"Spanish"}]
```
Single source of truth for the source/target language picker — read this
instead of hardcoding the list client-side (both the web and Android clients
do). It is a curated display list, not a hard whitelist: `quote`/`jobs`
accept any two-letter code, but only languages that also have a voice back
them for real narration (see `GET voices`' `language` field for what's
actually available as a *target*).

### 2. Voices

`GET /audiobook-gen/voices` →
```json
[{"id":"cs-CZ-Chirp3-HD-Aoede","display_name":"Tereza","language":"cs",
  "gender":"female","preview_url":null,"cost_per_million_chars_cents":3000}]
```
`preview_url` is currently always `null` — there is no voice-sample playback
endpoint yet. `id` is the opaque value you send back as `voice_profile_id`;
never construct or guess one.

### 3. Estimate

`POST /audiobook-gen/estimate` — multipart, one field: `file` (`.epub`,
`.mobi`, or `.txt`). Returns `{"estimated_char_count": 53178}`.

This is a **real extraction**, not a guessed number from file size — the
server actually unpacks the EPUB/MOBI and counts characters, so it is exact
for well-formed files. It can be slightly wrong only if the LLM chapterizing
step later reflows text (rare). Don't try to estimate client-side from the
picked file's byte size — always call this endpoint.

### 4. Quote

`POST /audiobook-gen/quote` — JSON:
```json
{"char_count":53178,"source_language":"cs","target_language":"cs",
 "voice_profile_id":"cs-CZ-Chirp3-HD-Charon"}
```
→
```json
{"char_count":53178,"translation_cost_cents":0,"tts_cost_cents":160,
 "internal_cost_cents":160,"quoted_price_cents":320,"currency":"USD"}
```
Show `quoted_price_cents` (already includes the display markup) as the
headline price, with a note that **the user is only ever charged this amount
or less** — the final `charged_price_cents` on a completed job is
`min(actual_cost * markup, quoted_price_cents)`, so a slower/costlier
generation than estimated is never passed on to the user. `translation_cost_cents`
is `0` whenever `source_language == target_language` — hide that line
entirely rather than showing a $0 row (mirrors the web wizard's `QuoteStep`).
Language codes are two-letter (`cs`, `en`, `de`, `it`, `fr`, `es` today —
treat the set as server-driven, not a hardcoded enum, since more may be added).

### 5. Create the job

`POST /audiobook-gen/jobs` — multipart: `file`, `title`, `source_language`,
`target_language`, `voice_profile_id`, `output_mode` (`"single_m4b"` or
`"multi_file"`), and optionally `cover_mode` / `cover_prompt` (below).
Returns a `GenerationStatusResponse` (same shape as the poll endpoint, §5
below) with `stage: "extracting"`.

`output_mode` matters more than it looks: **`multi_file` is the only mode
that will ever support listening to finished chapters before the whole book
is done** (still not implemented server-side as of this writing — see Known
gaps, §12 — but `single_m4b` architecturally can never support it, since an
M4B is one finalized container). Default the picker to `multi_file` unless
the user wants a single downloadable file.

### 5a. Cover art

Cover art is generated from the book's title with Gemini and the real title/author
composited on top afterwards. Two optional multipart fields on `POST /jobs` control it:

- **`cover_mode`** — `"auto"` (default), `"none"`, or `"custom"`.
  `"none"` skips generation entirely and the book keeps the library's normal
  coverless placeholder, same as a plain upload.
- **`cover_show_title`** — `"true"` / `"false"` (default **false**). Composites the
  book's title and author onto the finished artwork in real type. Off by default: the
  art usually reads as a finished cover already, and every screen that shows it prints
  the title underneath anyway.
- **`cover_prompt`** — the image brief. **Required when `cover_mode` is
  `"custom"`** (a custom mode with no prompt is a 400, not a silent fall back to the
  default — the caller would never learn its brief was ignored). `{title}` in the
  prompt is substituted with the book's title; a prompt without the placeholder is
  used as written.

`GET /audiobook-gen/cover-prompt` → `{"prompt": "A clean, modern audiobook cover…"}`
returns the default brief so a client can show it, let the user edit it, and send the
result back — read it rather than shipping your own copy of the wording, which is the
same reason `/languages` exists.

Cover generation is **best-effort and happens after the book is already published**: a
Gemini outage or a rejected prompt logs and is swallowed, leaving a finished, playable
audiobook with no cover. Don't treat a missing cover on a `complete` job as a failure,
and don't block the UI waiting for one — there is no "cover ready" signal to poll.

### 6. Poll status

`GET /audiobook-gen/jobs/{id}` →
```json
{
  "id": "…", "title": "…",
  "stages": ["extracting","preprocessing","narrating","assembling","complete"],
  "stage": "narrating",
  "error": null,
  "chapters": [{"id":"…","title":"Kapitola 1","status":"in_progress",
                "blocks_done": 14, "blocks_total": 22}],
  "download_url": null,
  "quoted_price_cents": 320, "actual_cost_cents": 228,
  "charged_price_cents": null, "currency": "USD"
}
```
- `stages` is the **ordered list this specific job runs through** — it omits
  `"translating"` when `source_language == target_language`. Build the
  progress stepper from this list, not a hardcoded one (mirrors the web
  `GenerationStatusPage`).
- `stage == "failed"`: show `error` verbatim (it is already a safe,
  user-facing message) and make clear **no charge happens on a failed job**.
- `stage == "complete"`: `download_url` is a presigned single-file link
  **only when `output_mode` was `single_m4b`**; for `multi_file` jobs the
  finished book instead appears through the normal library sync
  (`GET /library/changes`, §4) as a regular multi-file audiobook — don't wait
  on `download_url` for that mode, it stays `null`.
- Poll on an interval while `stage` is not terminal (the web client uses
  1000 ms while narrating; something like 3–5 s is reasonable on mobile to
  save battery/data — this is not a chat-latency UI). Stop polling once
  `stage` is `"complete"` or `"failed"`.
- `actual_cost_cents` climbs incrementally as blocks narrate — safe to show
  as a live "cost so far" if you want that level of transparency, but
  `quoted_price_cents` is the number the user actually agreed to.
- Chapter titles come from the server already localized to the book's
  language (`"Kapitola 1"` for Czech, `"Chapter 1"` for English, etc.) when
  no real chapter title could be detected — don't re-localize client-side.

### Finished-job notification

### Narration is charged against family credit

A completed job debits the family's credit ledger by `charged_price_cents` — a
`narration_charge` entry (migration 0049), noted with the book's title and carrying the
generation job id in `external_ref`. A **failed job is never charged**, which the UI and the
completion email both state explicitly, so don't render a pending cost for one.

This is the first thing in the product a user actively *buys* — storage accrues quietly in the
background; this is someone choosing to spend — so it shows up in the family's ledger
(`GET /family/billing`) alongside storage charges and top-ups. Render an unknown `entry_type`
by its `note` rather than dropping it; more kinds will be added.

Like every other credit rule (§10), it is **recorded, not enforced**: a family with no credit
still gets its book. Refusing at the end, after the audio exists and the upstream APIs have
already been paid, would be the worst possible moment to start enforcing.

A terminal job queues a `generation_complete` / `generation_failed` entry into the notification
inbox (§9) for the job's owner, **and emails them** (from `MAIL__FROM_ADDRESS`, `hello@own.audio`
on the hosted instance). Mail matters more here than anywhere else in the product: the job runs
for minutes to hours and the in-app signal only reaches someone whose app is still open. Both are
best-effort — a finished book is never reported as failed because mail or the inbox insert
didn't work. With `MAIL__*` unset the send no-ops with a warning, so a local stack needs no
credentials.

**Clients must not acknowledge a notification just because they showed it.** Acknowledging marks
it delivered and removes it from `GET /devices/notifications`, which is also what a client's own
notification list renders — the Mac did this and emptied its own inbox screen the instant a
banner fired. Track locally what you have surfaced; acknowledge when the *user* clears it. Poll `GET /devices/notifications` on resume — it is the only
signal that survives the status screen being closed, since nothing pushes and
`/audiobook-gen/jobs/{id}` is only read while that screen is open.

### Known caveats specific to this feature (beyond §12's general list)

- **No resume/retry UI server-side.** A failed job cannot be resumed from the
  app; the only recovery today is re-submitting a new job (which re-uploads
  and re-charges nothing — no charge happened — but does redo the work).
  Don't build a "retry" button that expects an endpoint; there isn't one yet.
- **No progressive/partial listening yet**, even for `multi_file` — the book
  only appears in the library once every chapter is narrated and assembly
  finishes. Don't design a "listen to what's done so far" affordance against
  a real endpoint; it doesn't exist. If/when it ships it will be
  `multi_file`-only (see step 4 above).
- **Very long books skip the LLM chapterizer.** Books whose extracted text
  exceeds ~120,000 characters fall back to a mechanical fixed-size chapter
  splitter server-side (still fully narrated correctly, just less precisely
  chapterized, and chapter titles become the localized "Kapitola N"-style
  default rather than real titles). There is no client control for this —
  just don't be surprised if a long book's chapter list looks coarser than a
  short one's.

---

## 11a. Podcast episode translation (personal use only)

Translates and re-narrates **one podcast episode** into another language, from a
transcript the podcast's own feed already publishes — see
`docs/podcast-translation-plan.md`. There is **no speech-to-text anywhere in this
feature**: an episode without a feed-published transcript simply cannot be translated,
full stop. Reuses the same Google Translate / Google Cloud TTS pipeline as §11's
audiobook narration, so most of this section will look familiar.

> ⚠️ **Personal use only.** Every client surface that offers this feature — a button, a
> quote screen, a result row — **must show the `notice` string every response below
> carries**: *"Translated audio is generated for your personal listening only and stays
> in your library."* This is a derivative work of someone else's podcast, generated only
> on the user's own explicit request. Never build a share, export, or publish affordance
> for it, however tempting a "send to a friend" button looks. If the server ever adds
> distribution for this feature, it will be a deliberate new decision, not an extension
> of what's here.

### Flow

```
1. Check `has_transcript` on the episode (§4/§5) — false means don't show the feature at all.
2. POST /podcast-translate/episodes/{episode_id}/quote  → priced quote (stateless, not persisted)
3. POST /podcast-translate/episodes/{episode_id}        → creates + starts the job
4. GET  /podcast-translate/episodes/{episode_id}         → list this user's translations for
                                                            the episode; poll until every row's
                                                            `status` is "complete" or "failed"
```

Two lists for a "translate" screen, so it does not ask once per episode:
`GET /podcast-translate/feeds/{feed_id}` (every translation of that show's episodes, same
objects as above, with `episode_id`) and `GET /podcast-translate/recent` (the family's latest
50 across shows, failed ones left out, each also carrying `episode_title`, `feed_id`,
`feed_title`, `image_url`; only shows the caller can see).

Language and voice pickers reuse §11's endpoints verbatim — `GET /audiobook-gen/languages`
and `GET /audiobook-gen/voices` — there is no separate list for this feature.

### Quote

`POST /podcast-translate/episodes/{episode_id}/quote` — JSON:
```json
{"target_language":"de","voice_profile_id":"de-DE-Chirp3-HD-Aoede"}
```
`source_language` is optional — omit it and the server falls back to the feed's own
`language` field, defaulting to `en` if that's unset too; pass it explicitly only if you
want to override that guess. → 
```json
{"char_count":9214,"translation_cost_cents":19,"tts_cost_cents":31,
 "quoted_price_cents":100,"currency":"USD",
 "notice":"Translated audio is generated for your personal listening only and stays in your library."}
```
`char_count` here is **exact**, not an estimate — the server fetches and normalizes the
transcript synchronously to price it, unlike §11's audiobook quote (which prices off a
real-but-separate `/estimate` call). A 400 with a clear message means either the episode
has no transcript after all (raced with a feed update — re-check `has_transcript`) or
`target_language` equals the resolved source language.

### Create

Same body as the quote, `POST /podcast-translate/episodes/{episode_id}`. A second request
for the **same** `(episode, target_language, voice_profile_id)` returns the existing row
(200, not 201) instead of starting — and charging for — a duplicate. Returns a translation
row in the same shape §11a's poll endpoint does, `status: "translating"`.

### Poll

`GET /podcast-translate/episodes/{episode_id}` → an array (every translation this user has
requested for that episode, any language/voice):
```json
[{
  "id": "…", "episode_id": "…",
  "source_language": "en", "target_language": "de", "voice_profile_id": "de-DE-Chirp3-HD-Aoede",
  "status": "narrating",
  "char_count": 9214, "quoted_price_cents": 100, "charged_price_cents": null, "currency": "USD",
  "error": null, "stream_url": null,
  "notice": "Translated audio is generated for your personal listening only and stays in your library."
}]
```
- `status` progresses `translating` → `narrating` → `assembling` → `complete` | `failed` —
  build a stepper from this list of five, same idea as §11's `stages` array, but it's
  fixed (not per-job) since there's no optional stage to omit here.
- `stream_url` is set **only** once `status == "complete"` — a presigned link, same 4-hour
  expiry as every other stream URL in this API (§8's "never persist them" rule applies
  here too).
- `status == "failed"`: show `error` verbatim and make clear **no charge happens** — same
  rule as §11.
- Poll on the same kind of interval §11 recommends (3–5 s on mobile) while any row is
  non-terminal; stop once all rows are `complete` or `failed`.

### Billing and notifications

Charged against family credit exactly like §11's narration charge — same `credit_ledger`,
same `narration_charge` entry type (migration 0049), just with this translation's id (not
a `generation_jobs` id) in `external_ref`, and a note like `"Podcast translation (de)"`.
Same non-enforcement rule: recorded, never blocks anything.

A terminal translation queues `podcast_translation_complete` / `podcast_translation_failed`
into the notification inbox (§9) for the requesting user only — **no email**, unlike §11
(a single-episode translation is expected to run for minutes, not hours, so the in-app
inbox is enough; this may change if real-world runtimes turn out longer). `data.item_id`
is the **episode** id, so a client can deep-link to wherever it already shows that
episode's translations; `data.podcast_episode_translation_id` carries the specific row
that finished, for a client that wants to highlight just that one instead of re-rendering
the whole list.

### Known caveats specific to this feature

- **No resume/retry UI server-side**, same as §11 — a failed translation isn't
  resumable; the only recovery is a fresh `POST`, which does re-check the transcript and
  re-quote (so a genuinely stale quote won't overcharge) but does redo the narration work.
- **Single voice, no diarization.** A multi-host episode is narrated in one voice
  throughout; there's no per-speaker voice mapping. The transcript's speaker-turn
  structure (when the source used VTT `<v Speaker>` tags) still produces paragraph breaks
  in the narrated audio, it just doesn't switch voices at them.
- **No playback progress persistence for translated audio** — `stream_url` plays like any
  other stream, but there is no `PUT` endpoint to save a resume position against a
  translation id the way §5's episode/book progress works. Don't wire one up against a
  guessed URL shape; it doesn't exist yet.
- **Eligibility is binary and feed-driven.** `has_transcript` reflects only whether the
  feed itself published a `<podcast:transcript>` tag at last refresh — there is no
  "request a transcript" or "check again" action beyond a normal feed refresh
  (`POST /podcasts/{id}/refresh`, §5), and no fallback if the feed never adds one.

---

## 12. Known gaps — plan around these

| Gap | What it means for the app |
|---|---|
| **Push delivery unimplemented** | Poll the inbox (§9). |
| **Chapters may be empty** | Nothing extracts chapters from M4B/ID3 yet, and there is no chapter-write endpoint. Design the player to fall back to per-file navigation. |
| **No transcoding** | No quality selector; on a bad connection the only lever is downloading in advance. |
| **No server auto-download rules** | Implement "auto-download new episodes" client-side. |
| **Stream URLs expire in 4 h** | Never persist them; re-request on resume. |
| **Podcast episodes need a server-side download first** | Check `has_local`; call `/download` before `/stream`. |
| **Uploads have no progress endpoint** | Use OkHttp's request-body progress for the UI; large uploads on cellular need a warning. |
| **Multipart uploads cap at 100 MB when hosted** | The proxy in front of the hosted API rejects larger bodies. Use the presigned flow in §8a for anything book-sized. |
| **OIDC** | Email/password everywhere. **Google and Microsoft both work for desktop clients** via loopback + PKCE (`POST /auth/google` or `POST /auth/microsoft` with `code`, `code_verifier`, `redirect_uri`; the redirect must be `http://127.0.0.1:…`/`localhost`; providers and the desktop client id from `GET /auth/providers`) — see `docs/sso-payments-plan.md` and `docs/microsoft-sign-in-plan.md`. Apple: `POST /auth/apple`. Microsoft uses the `common` authority, so work/school **and** personal accounts sign in; it issues no `email_verified` claim, so the backend records the identity as unverified. **A client must treat a missing `microsoft` object in `/auth/providers` as "not offered"** — older servers omit it entirely. The browser-redirect routes (`/auth/google/redirect\|callback`, `/auth/microsoft/redirect\|callback`) remain literal `"TODO"` stubs and are unrelated. |
| **No bulk delete** | Bulk exists only for marking episodes played (`POST /playback/episodes/progress/bulk`). |
| **Generation has no resume/retry** | A failed generation job can't be retried in place — only re-submitted as a new job (§11). |
| **Generation has no progressive listening** | A `multi_file` generation only appears in the library once every chapter is done, not chapter-by-chapter (§11). |

---

## 13. Suggested build order

1. **Skeleton**: host entry → `GET /setup/status` → login → token storage →
   authenticated `GET /auth/me`. Get the refresh flow right here; everything
   later depends on it.
2. **Sync + cache**: Room + `GET /library/changes` with tombstones. Render
   plain lists from the database.
3. **Player**: `MediaSessionService` + ExoPlayer, single-file music first,
   then multi-file audiobooks, then chapters.
4. **Progress**: save position; wire `GET /library/continue` into Home.
5. **Podcasts**: subscribe, episode list, server download, play.
6. **Offline**: download manager, eviction, local-first playback.
7. **Stats**: session reporting with `client_session_id`, then the board.
8. **Family**: members, invites, and — admin only — access controls.
9. **Polish**: Android Auto, widget, Material You, TalkBack, predictive back.

Steps 1 and 2 are the ones worth over-engineering; the rest sits on top of
them. If the token refresh or the sync cursor is wrong, every later screen
inherits the bug.

---

## 14. Endpoint index

Complete list, generated from the router. Auth is bearer unless noted.

<details>
<summary><b>Setup &amp; auth</b> (public unless marked)</summary>

```
GET    /server                           (public; edition, version, features)
GET    /setup/status                     (public)
POST   /setup/complete                   (public, only before first user; rate-limited)
GET    /auth/registration-status         (public)
POST   /auth/login                       (public)
POST   /auth/register                    (public; accepts invite_code)
POST   /auth/refresh                     (public; rotating)
POST   /auth/fork                        (public; a second chain for a helper process)
POST   /auth/logout
GET    /auth/me
POST   /auth/password
GET    /auth/sessions
DELETE /auth/sessions/{chain_id}
POST   /auth/admin-create-user           (instance admin)
```
</details>

<details>
<summary><b>Users</b></summary>

```
GET    /users/me
PATCH  /users/me                         (display_name, recommendations_enabled,
                                          discovery_languages) → 204, no body
DELETE /users/me
GET    /users/me/subsonic-key
POST   /users/me/subsonic-key/regenerate
GET    /users                            (instance admin)
GET    /users/{id}                       (self or admin)
PATCH  /users/{id}                       (instance admin)
DELETE /users/{id}                       (instance admin)
POST   /users/{id}/revoke-sessions       (instance admin)
```
</details>

<details>
<summary><b>Family</b></summary>

```
GET    /family
PUT    /family                                        (family admin)
GET    /family/members
PUT    /family/members/{user_id}                      (family admin)
DELETE /family/members/{user_id}                      (family admin or self)
GET    /family/invites                                (family admin)
POST   /family/invites                                (family admin)
DELETE /family/invites/{id}                           (family admin)
POST   /family/invites/accept
GET    /family/members/{user_id}/access               (family admin)
PUT    /family/members/{user_id}/policy               (family admin)
PUT    /family/members/{user_id}/grants               (family admin)
GET    /family/content/{kind}/{item_id}/audience      (family admin)
PUT    /family/content/{kind}/{item_id}/audience      (family admin, {can_listen: [user_id]})
POST   /family/content/{kind}/{item_id}/request-access (any member, no body)
GET    /family/storage                   (any member; bytes per kind)
GET    /family/billing
GET    /family/billing/alerts                         (family admin)
PUT    /family/billing/alerts                          (family admin)
```
</details>

<details>
<summary><b>Library</b></summary>

```
GET    /library/changes?since=      delta sync + tombstones
GET    /library/continue            in-progress items
GET    /library/search?q=&limit=    books, feeds, episodes, tracks
GET    /library/private             my never-shared items
```
</details>

<details>
<summary><b>Audiobooks</b></summary>

```
GET    /audiobooks
POST   /audiobooks                                    (visibility)
POST   /audiobooks/upload                             (multipart; ≤100 MB when hosted)
POST   /audiobooks/from-uploads                       (JSON; presigned flow, §8a)
POST   /audiobooks/{id}/files/from-uploads            (JSON; more files for your own book)
GET    /audiobooks/{id}
PUT    /audiobooks/{id}                               (owner)
DELETE /audiobooks/{id}                               (owner)
PUT    /audiobooks/{id}/visibility                    (owner)
GET    /audiobooks/{id}/cover
POST   /audiobooks/{id}/upload-cover                  (owner, multipart)
GET    /audiobooks/{id}/files
PUT    /audiobooks/{id}/files/{file_id}               (owner, or family admin if shared)
POST   /audiobooks/{id}/upload-file                   (owner, multipart)
PUT    /audiobooks/{id}/files/reorder                 (owner)
GET    /audiobooks/{id}/files/{file_id}/stream
GET    /audiobooks/{id}/chapters
GET    /audiobooks/authors
POST   /audiobooks/authors
GET    /audiobooks/authors/{id}
PUT    /audiobooks/authors/{id}
DELETE /audiobooks/authors/{id}
GET    /audiobooks/authors/{id}/books
GET    /audiobooks/authors/book/{book_id}
POST   /audiobooks/authors/book/{book_id}
DELETE /audiobooks/authors/book/{book_id}/{author_id}/{role}
GET    /audiobooks/authors/tags
GET    /audiobooks/authors/tags/book/{book_id}
PUT    /audiobooks/authors/tags/book/{book_id}
GET    /audiobooks/organize/collections
POST   /audiobooks/organize/collections
GET    /audiobooks/organize/collections/{id}
PUT    /audiobooks/organize/collections/{id}
DELETE /audiobooks/organize/collections/{id}
GET    /audiobooks/organize/collections/{id}/books
POST   /audiobooks/organize/collections/{id}/books
DELETE /audiobooks/organize/collections/{id}/books/{book_id}
GET    /audiobooks/organize/series
POST   /audiobooks/organize/series
GET    /audiobooks/organize/series/{id}
PUT    /audiobooks/organize/series/{id}
DELETE /audiobooks/organize/series/{id}
POST   /audiobooks/organize/series/{id}/books
DELETE /audiobooks/organize/series/{id}/books/{book_id}
GET    /audiobooks/organize/favorites
POST   /audiobooks/organize/favorites/{book_id}
DELETE /audiobooks/organize/favorites/{book_id}
GET    /audiobooks/organize/favorites/{book_id}/check
```
</details>

<details>
<summary><b>Audiobook generation</b> (AI narration — see §11)</summary>

```
GET    /audiobook-gen/languages
GET    /audiobook-gen/voices
GET    /audiobook-gen/cover-prompt
POST   /audiobook-gen/estimate                        (multipart)
POST   /audiobook-gen/quote
POST   /audiobook-gen/jobs                             (multipart)
GET    /audiobook-gen/jobs/{id}
```
</details>

<details>
<summary><b>Podcasts</b></summary>

```
GET    /podcasts
POST   /podcasts/search                               (directory search)
GET    /podcasts/discover/categories                  (category list + counts)
GET    /podcasts/discover/browse?category=            (cold-start discovery)
GET    /podcasts/{id}/similar                         (more like this)
POST   /podcasts/subscribe                            (visibility; RSS or YouTube)
GET    /podcasts/{id}
DELETE /podcasts/{id}                                 (owner)
PUT    /podcasts/{id}/visibility                      (owner)
GET    /podcasts/{id}/episodes?limit=&offset=
POST   /podcasts/{id}/refresh
PUT    /podcasts/{id}/auto-store                      (owner or family admin)
POST   /podcasts/{id}/sync-images
POST   /podcasts/{id}/episodes/{ep_id}/download
DELETE /podcasts/{id}/episodes/{ep_id}/download       (owner or family admin; to the trash)
GET    /podcasts/{id}/episodes/{ep_id}/stream
GET    /podcasts/{id}/image                           (public)
GET    /podcasts/{id}/episodes/{ep_id}/image          (public)
```
</details>

<details>
<summary><b>Music</b></summary>

```
GET    /music/tracks
POST   /music/tracks/upload                           (multipart, visibility)
POST   /music/tracks/from-upload                      (JSON; presigned flow, §8a)
GET    /music/tracks/{id}
PUT    /music/tracks/{id}                             (owner)
DELETE /music/tracks/{id}                             (owner)
PUT    /music/tracks/{id}/visibility                  (owner)
GET    /music/tracks/{id}/stream
GET    /music/tracks/{id}/cover
POST   /music/tracks/{id}/upload-cover                (owner, multipart)
GET    /music/tracks/{id}/progress
PUT    /music/tracks/{id}/progress
PUT    /music/tracks/{id}/feedback                    {"kind":"dislike"|"banned"}
DELETE /music/tracks/{id}/feedback                    (idempotent)
GET    /music/feedback?kind=                          (the undo list)
GET    /music/artists
GET    /music/albums?artist=
GET    /music/genres
GET    /music/playlists
POST   /music/playlists                               (visibility)
GET    /music/playlists/{id}
PUT    /music/playlists/{id}                          (owner)
DELETE /music/playlists/{id}                          (owner)
PUT    /music/playlists/{id}/visibility               (owner)
GET    /music/playlists/{id}/tracks
POST   /music/playlists/{id}/tracks
DELETE /music/playlists/{id}/tracks/{entry_id}
PUT    /music/playlists/{id}/tracks/reorder
```
</details>

<details>
<summary><b>Playback</b></summary>

```
GET    /playback/settings
PUT    /playback/settings/audiobook-defaults
GET    /playback/books/progress                       (every started book, one call)
GET    /playback/books/{book_id}/progress
PUT    /playback/books/{book_id}/progress             (file_id required)
GET    /playback/episodes/{episode_id}/progress
PUT    /playback/episodes/{episode_id}/progress
POST   /playback/episodes/progress/bulk               (mark many played)
GET    /playback/bookmarks
POST   /playback/bookmarks
PUT    /playback/bookmarks/{id}
DELETE /playback/bookmarks/{id}
GET    /playback/books/{book_id}/bookmarks
GET    /playback/queue
PUT    /playback/queue
POST   /playback/sessions                             (batched, idempotent)
```
</details>

<details>
<summary><b>Stats &amp; devices</b></summary>

```
GET    /stats/me?range=&tz_offset_minutes=
GET    /stats/me/history?limit=&offset=
PUT    /stats/me/visibility
GET    /stats/family                                  (family admin)
PUT    /stats/family/members/{user_id}/visibility     (family admin)
POST   /devices/push-token
DELETE /devices/push-token
GET    /devices/push-tokens
GET    /devices/notifications
POST   /devices/notifications/ack
GET    /jobs                                          (instance admin)
GET    /jobs/{id}                                     (instance admin)
POST   /uploads/presign                               (§8a)
POST   /uploads/complete                              (§8a)
```
</details>

<details>
<summary><b>OpenSubsonic</b> — <code>/rest</code>, music only, not for this app</summary>

Exists so third-party Subsonic apps can use the server. Auth is the per-user
key from `GET /users/me/subsonic-key`, not the JWT — every account has one
from the moment it is created. Audiobooks are deliberately absent, which is
exactly why the native API is the right choice here; podcasts are served.

50 of the 72 methods in Subsonic 1.16.1 are implemented. The other 22 —
video, sharing, jukebox, chat, user management, internet-radio writes, the
deprecated v1 `search`, `getAvatar`, `hls`, `getCaptions` — answer a proper
error envelope rather than a bare 404, so a client hides one button instead
of deciding the server is not a Subsonic server.

```
ping, getLicense, getOpenSubsonicExtensions, getMusicFolders, getUser,
getScanStatus, startScan, getInternetRadioStations, getNowPlaying,
getArtists, getIndexes, getArtist, getMusicDirectory, getGenres,
getAlbumList, getAlbumList2, getAlbum, getSong, getArtistInfo(2),
getAlbumInfo(2), getSimilarSongs(2), getTopSongs, getRandomSongs,
getSongsByGenre, getStarred(2), search2, search3, stream, download,
getCoverArt, getLyrics, getLyricsBySongId, star, unstar, setRating,
scrobble, getBookmarks, createBookmark, deleteBookmark, savePlayQueue,
getPlayQueue, getPodcasts, getNewestPodcasts, refreshPodcasts,
createPodcastChannel, deletePodcastChannel, downloadPodcastEpisode,
deletePodcastEpisode, getPlaylists, getPlaylist, createPlaylist,
updatePlaylist, deletePlaylist
```

Two deliberate gaps inside those 50:

- **No transcoding.** `stream` returns the stored bytes; `maxBitRate` and
  `format` are accepted and ignored. Clients decode MP3/AAC/FLAC natively, so
  this costs bandwidth control, not playback.
- **`getAlbumList&type=byYear` always returns an empty list.** `music_tracks`
  has no release-year column, so there is nothing to filter on. Returning an
  empty list is the truthful answer; if a year is ever stored, this becomes a
  one-line change in `subsonic::browsing::select_albums`.

`savePlayQueue`/`getPlayQueue` share the same queue as
`/api/v1/playback/queue`, so a Subsonic app and this app stay in sync.
</details>
