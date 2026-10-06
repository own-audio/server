# File sync — own.audio as a folder, with a 30-day trash

Written and reviewed with the user 2026-09-25. **Nothing here is
implemented yet.**
`CLAUDE.md` of each touched repo governs its part.

Scope of this document: the backend, the shared Rust sync core, the macOS app
(Finder), and the web app. Docker, Windows, Linux and iOS come later and are
listed in §9 only so the core is designed for them.

## 1. The problem

The Mac app's download/mirror subsystem works, but people cannot understand or
manage it. A user meets about 25 terms (Download, Mirror, Mirror Everything,
Automatic Mirroring, Pinned, Evict, Local Only, Upload to Cloud, Both, Verify
Library, Scan for New Files, Transfers, Storage Limits, Remove After…), an item
can be in about eight states, and there are five ways for a file to end up on
the Mac. The library folder is both the app's cache and a place users are
invited to drop files into. For a product whose whole promise is "trust us with
your files", that reads as cheap and risky.

People already understand one model: **iCloud Drive**. A folder in Finder;
every file is either in the cloud or also on this Mac; "Keep Downloaded";
the system frees space by itself; delete means delete everywhere, with a
"Recently Deleted" safety net. This plan moves sync and upload there, and
leaves the app to do what it is good at: playback and managing the library.

## 2. Decisions

Settled with the user on 2026-09-25:

1. **Sync lives in Finder**, via a File Provider extension shipped inside the
   app bundle — one install. The app keeps playback and library management
   (metadata, playlists, Identify, family sharing). Download/mirror/transfer UI
   leaves the app.
2. **One shared Rust sync core** drives every platform (Mac now; Docker,
   Windows, Linux later), so sync behaves the same everywhere. It is the only
   deliberate exception to "no shared code between repos".
3. **Adding = put a file in the folder. Deleting = delete everywhere.** No
   second kind of delete.
4. **Members add and delete only their own content. Family admins can delete
   anything shared with the family.** (Private items of others stay invisible
   to admins, as today.)
5. **A 30-day trash** for every deletion, per user, with a family-wide view for
   family admins. After 30 days the item is purged from storage.
6. **Local Only stays.** It means "the file is not in the own.audio folder".
   The app can still play such files (Local Folders, §7.6); sync never touches
   them.
7. **Podcasts are in Finder** — but only the episodes **stored in own.audio**
   (paid for, kept once, listened to by the family later), not the whole RSS
   list, which stays browsable in the app. Folder per show, readable names.
   The server, not the Mac, stores new episodes of shows that ask for it.
   Deleting in Finder trashes stored copies; the subscription itself is
   managed in the app.
8. **Family content works like OneDrive shortcuts.** `Family/` is empty until
   the user adds something with **Add to own.audio folder** — either a
   member's whole kind ("Petr's music") or one book, album or show. Shortcuts
   belong to the account and appear on all of its devices. Deleting a
   shortcut's root folder removes only the shortcut; inside it, our delete
   rights apply (members nothing, family admins trash).
9. **Folder names on disk are fixed English** (`Audiobooks`, `Music`,
   `Podcasts`, `Family`, and the `Unknown …` placeholders), whatever the app's
   language — the tree is a contract. Finder shows the four top-level names in
   the user's language via `.localized` if File Provider allows it (spike);
   otherwise the English names show, as with macOS's own home folders.
10. **The system never changes or moves a user's file.** The bytes stay
    exactly as uploaded, and the **path belongs to the user**: a file stays
    where and under the name it was put, and appears at the same path on every
    device (the OneDrive/Dropbox model). Metadata lives only on the server —
    the app shows the library by metadata, Finder shows the user's own
    structure. Items created without a path (app or web upload, import, a
    stored episode) get a default path from their metadata **once**, at
    creation; editing metadata or Identify never renames or moves anything.
    Renames and moves in Finder are not offered; an audiobook folder is added
    and deleted whole.
11. **Imports from Navidrome and Audiobookshelf go through the app and the
    API**, as they do today: the app fetches the file, the core uploads it
    with the source's metadata (series, narrator, description, cover), and it
    appears in Finder at its default path. Copying a file into the folder by
    hand still works as an ordinary upload, without that metadata.
12. **The trash is free unless the item is restored.** What is purged after
    30 days was never billed while it waited. **Restoring charges the days it
    spent in the trash**, as if it had never been deleted — so trashing
    everything just before the daily billing run and restoring it after gains
    nothing. User-facing line: "You don't pay for what's in the trash. If you
    restore it, you pay as if you had never deleted it." Trash size and
    restore patterns are monitored (§5.1).
13. **Deleting one of your own items needs no confirmation**, in the web and
    the Mac app alike: it moves to the trash at once with a "Moved to Trash ·
    Undo" toast (Undo within seconds costs nothing — zero whole days in the
    trash). A **dialog** appears for bulk deletes and when a **family admin
    deletes another member's item** ("This belongs to Petr. It moves to the
    trash and Petr is notified."). Finder keeps its own native behaviour.
14. **Finder deletes are held only for catastrophes**: more than **100 items**
    in one burst (10 s), or a whole top-level folder (`Audiobooks`, `Music`,
    `Podcasts`). On the Mac **Finder itself asks** before anything happens (a
    File Provider alert, §7.2). Deletes that bypass Finder (Terminal, a
    script, later the Docker agent) are held by the core and confirmed in the
    app; "Keep" brings the items back. A held delete is **never confirmed
    automatically**, even if the question is ignored. Ordinary cleanup goes through without asking — the safety net is
    the trash, where **a whole deletion is restored with one click**
    ("Restore 120 items deleted at 14:32").
15. **"Where is this file" comes later, but its groundwork now.** Finder shows
    cloud vs this Mac natively. The server records which of a user's devices
    hold which items offline, and the Mac reports from day one; the UI and the
    other clients' reporting come in a later phase. A user sees only their
    own devices.
16. **Files that belong with the audio are kept** (decided 2026-09-25):
    images, PDF booklets, `.lrc`, `.cue`, `.txt`/`.nfo` put next to music or
    in a book folder are stored as they are, at their path, synced like the
    audio. Other types are still refused. The cover defaults to the art
    embedded in the audio, else an image called `cover`/`folder`/`front`;
    when a folder has several images the user picks the cover in the app —
    no image is ever removed or changed. A `.lrc` named like a track becomes
    its lyrics.
17. **Where a file sits decides who sees it** (decided 2026-09-25). The
    top-level `Music/`, `Audiobooks/`, `Podcasts/` hold the user's private
    items; `Family/<Me>/…` holds what they share, always shown;
    `Family/<Member>/…` holds others' shared items via shortcuts (item 8).
    Putting a file into `Family/<Me>/…` uploads it shared. Dragging between
    `Music/x` and `Family/<Me>/Music/x` is the one move Finder allows: it
    shares or unshares, the path inside stays. Sharing in the app moves the
    file the same way. Who in the family sees it is still the admins' call.
18. **"Organise Music folder" — the one move the system makes, and only when
    asked** (decided 2026-09-25). Files dropped in Finder keep the place the
    user gave them (item 10), so a Music folder can end up as
    `Music/Queen - A Kind of Magic/…` next to `Music/Queen/Jazz/…`. The app
    offers **Organise Music folder**: it shows every file that would move to
    `Music/<Album artist>/<Album>/<NN> - <Title>.<ext>` (from the tags and
    Identify, the same default as §4.1), the user confirms all or some, and
    only then do the paths change; Finder follows through the feed. Never
    automatic, never on upload. Shared items move on the shared side
    (`Family/<Me>/Music/…`). Other members' items are not theirs to move.

## 3. What exists today (verified 2026-09-25)

Backend (`audio2/backend`):

- **Sync feed**: `GET /library/changes?since=` (`library/mod.rs:149`) returns
  untyped summaries only — no files, sizes or sha256 — plus tombstones from
  `deleted_items` (migration 0021). Unsharing or a grant change produces **no
  tombstone**; the item just stops appearing.
- **Tombstone pruning never runs**: `prune_tombstones(90)` is inside the
  `stats_rollup` job, which nothing enqueues (`jobs/worker.rs:1011`).
- **Every delete is a hard delete; there is no trash anywhere.** Audiobook
  delete leaves its S3 objects orphaned (`audiobooks/mod.rs:673`); music track
  delete removes unreferenced objects immediately (`music/mod.rs:1507`). The
  `storage_sweep` job exists but is not in prod's `WORKER_JOB_TYPES`.
  `db/billing.rs:5`: "Nothing deletes storage objects today" — this includes
  account deletion.
- **Only the owner can delete** (`find_*_owned … WHERE user_id = $2`). Family
  admins cannot delete another member's items.
- **Uploads**: presign (`uploads/mod.rs`, 5 GiB cap, kinds `audiobook_file`,
  `audiobook_cover`, `music_track`, `music_cover`) + `POST
  /audiobooks/from-uploads` creates a multi-file book in one call. **Music has
  no from-upload endpoint**: only multipart `POST /music/tracks/upload`,
  capped at 100 MB by Cloudflare in prod, which also skips
  `require_can_upload`. The server reads music tags (lofty, incl. album
  artist); it reads **no** audiobook tags.
- `size_bytes` + `sha256` (filled by the `media_checksum` job) are on audiobook
  files, tracks and episodes.
- **The backend never writes into audio files.** Tags are only read (lofty,
  `music/mod.rs:1679`), Identify and metadata edits change the database only;
  the server re-encodes only cover images and its own generated narration.
- **Uploads are not refused when credit is exhausted** (`depleted` is
  informational).
- **Podcasts**: a feed belongs to its subscriber (`podcast_feeds.user_id`,
  unique per user + URL) and can be shared with the family
  (`family_id`, migration 0019). An episode is stored only on request
  (`POST /podcasts/{id}/episodes/{ep}/download` sets `audio_object_id`;
  `DELETE …/download` removes the object at once). Stored episodes are billed
  (`PODCAST_REFS`). **No server-side auto-download** — today only the Mac
  auto-downloads, locally. Users cannot upload episodes.
- Device-code login (RFC 8628 shape) exists: `POST /auth/device/start|poll` —
  reusable for the Docker agent later.
- Notifications: `db::sync::queue_notification` (`db/sync.rs:283`), clients
  poll `GET /devices/notifications`.
- Single crate, no workspace; DTOs live in handlers and depend on axum —
  nothing a separate crate could import. Latest migration: 0076.

Mac (`audio2-mac`):

- Two targets, **no extensions, no App Group, no keychain access group**.
  Bundle `app.audio2.mac`, macOS 14.0, team `Z53P4WUKK2`, Apple Development
  signing. No Rust/FFI anywhere.
- The mirror/download subsystem spans `Audio2Core`, `Audio2Downloads`,
  `Audio2Persistence`, `FeatureDownloads` and ~1,500 lines of
  `AppContainer.swift` (inventory in §7.8). `Audio2*` and `Features/` are
  shared with the iOS apps.
- Playback asks `LocalFileProvider.localFileURL(mediaKind:itemId:partId:)`
  before streaming; `DownloadManager` is the only conformer.
- Tokens: `AuthTokenStore` (Keychain service `app.audio2.book.tokens`).

Web (`audio2/frontend`): React 19, react-router 7, axios, TanStack Query,
Tailwind, **English only (no i18n)**. Three different delete-confirmation
patterns. Family admin check via `isFamilyAdmin(my_role)`. Notifications are a
popover (`NotificationsPanel.tsx`), no inbox page. Vitest only; no e2e suite.

## 4. The model

### 4.1 The tree

```
own.audio/
  Audiobooks/…     the user's own structure; each book is a folder
  Music/…          the user's own structure
  Podcasts/<Show>/<YYYY-MM-DD> - <Episode title>.<ext>
  Family/<Member>/Audiobooks|Music|Podcasts/…   the owner's own paths;
                                                only shortcuts (§4.4)
```

- **The server stores every item's path** — a book's folder and its files'
  relative paths, a track's file path, a stored episode's path. The core and
  every client use it as given; nothing re-derives a path later (§2 item 10).
- Only the **top-level folder is fixed** (§2 item 9). It tells the server what
  kind an item is.
- **Default path**, used only when an item is created without one (app or web
  upload, import, a stored episode, and the backfill of existing items),
  computed once on the server — the layout the Mac's download folder already
  uses (`LibraryPathBuilder`):
  - `Audiobooks/<Author>/<Title>/<NN> - <File title>.<ext>`
  - `Music/<Album artist>/<Album>/<NN> - <Title>.<ext>` (multi-disc
    `<D>-<NN>`; `Unknown Album`, `Unknown Author` when a tag is missing)
  - `Podcasts/<Show>/<YYYY-MM-DD> - <Episode title>.<ext>`, date from
    `published_at`, else the day it was stored
  Default names are Windows-safe (no `<>:"/\|?*`, no trailing dot or space, no
  reserved names like `CON`).
- **Names the user gives are kept as given** (NFC-normalised, ≤255 bytes per
  component). The later Windows client maps characters Windows forbids for
  display only (§9).
- **Paths are unique per owner.** If two devices create the same path while
  offline, the later one gets ` (2)` — the only case where the system changes
  a name, like Dropbox's conflicted copies.
- **Tags inside the files stay as uploaded.** A tool reading the folder
  directly (e.g. Navidrome on the Docker folder later) sees the original tags,
  not edits made in own.audio. An export with tags written is a later option
  (§9).
- An empty folder the user creates stays on that device until something is
  put in it; the server stores item paths, not folders.
- Playlists are not in the tree.

### 4.2 Identity — the local DB is the source of truth

Every file and folder has a stable identifier that never depends on its name:
`ab:<book uuid>`, `abf:<file uuid>`, `mt:<track uuid>`, `pe:<episode uuid>`,
`pf:<feed uuid>` for a show folder; other folders are `dir:<owner>/<path>`
and exist because items live under them. The DB maps identifier ↔ server
path ↔ size/sha256 ↔ local state. The filesystem is a projection of
the DB, never the other way round — exactly what Navidrome gets wrong when a
file moves behind its back.

### 4.3 What each action does

| In Finder | Result |
|---|---|
| Put an audio file anywhere under `Music/` | Uploaded as a track. It stays exactly where and as you put it; the server reads its tags for the app. |
| Put a folder (or one loose file) under `Audiobooks/` | Uploaded as one book once the folder stops changing; the files keep their names and order. Title/author for the app from tags, else from the folder names. Cover from embedded art or an image in the folder. |
| Put anything at the root, in `Family/`, in `Podcasts/`, or inside an existing book | Finder refuses before copying, with an alert giving the reason (§7.2). Episodes cannot be uploaded; they are stored from the app. |
| Delete an episode | Its stored copy goes to the trash. The episode stays in the show's list in the app and can be stored again while the feed still offers it. |
| Delete a show folder | All its stored episodes go to the trash. The subscription stays (managed in the app). |
| Image, PDF, `.lrc`, `.cue`, `.txt`/`.nfo` next to music or in a book | Kept as a companion file at its path (§2 item 16). |
| Any other non-audio file | Refused, listed. |
| Put a file into `Family/<Me>/…` | Uploaded shared with the family (§2 item 17). |
| Drag between `Music/x` and `Family/<Me>/Music/x` (or the same for Audiobooks, Podcasts) | Share / unshare; nothing else moves. |
| File > 5 GiB | Refused, listed. |
| Delete / Move to Trash (a file or any folder) | Every item inside that the user may delete goes to the own.audio trash; it disappears from every device. |
| Delete something you may not delete | Not offered (item capabilities). |
| Delete a family shortcut's root folder | The shortcut is removed from this account. Nothing is trashed. |
| Rename or any other move | Not offered (§2 item 10). |
| "Keep Downloaded" / "Remove Download" | Native Finder; the system evicts non-kept files when space runs low. |
| Copy a file out | Nothing changes in own.audio. |
| Drag a file out of the folder | = delete from own.audio (trash), file stays where it was dropped. |

### 4.4 Family shortcuts

- A shortcut targets either **a member + kind** (all of Petr's music, new
  albums included) or **one container** of a member: a book, an album or a
  show.
- Its items appear at the **owner's own paths** under `Family/<Member>/…` —
  never inside the user's own folders — and cannot be moved. An album shortcut
  shows the album's tracks wherever they sit in the owner's folders.
- Added and removed with **Add to own.audio folder** / **Remove from own.audio
  folder** in the app or the web, on a member's kind or on a single book,
  album or show. The name is platform-neutral on purpose: the same shortcut
  shows in Finder, later in Explorer and the Docker folder.
- Deleting the shortcut's root folder in Finder = remove the shortcut. Nothing
  is trashed.
- Overlaps are fine: an album shortcut inside a member+kind shortcut adds
  nothing; removing the broader one leaves the album.
- A target that stops being visible (unshared, member left) shows nothing;
  the server drops shortcuts of a member who leaves the family.
- An album has no id. The shortcut stores the MusicBrainz release group when
  the album is identified, else album artist + album name — a later retitle of
  an unidentified album loses the shortcut. Accepted for v1.

### 4.5 Safety rules

- **Tombstones + DB beat resurrection.** A device offline for a week must not
  re-upload what was deleted meanwhile. The core re-uploads only files its DB
  has **never** seen with a server id. A known file the server reports as
  removed is removed locally.
- **Periodic full reconciliation.** On start and every 6 h, the core fetches
  all visible ids (§5.4) and removes local items the server no longer shows.
  This covers revoked sharing (no tombstone today), missed tombstones, and a
  cursor older than tombstone retention.
- **Offline longer than the trash period** → an unknown file is uploaded as
  new. Worst case the user sees something again; nothing is lost.
- **Catastrophe hold** (§2 item 14): over 100 items in 10 s or a whole
  top-level folder. On the Mac, Finder asks first (§7.2). For deletes that
  bypass Finder, the core holds the batch, the app posts a notification, the
  user confirms or keeps; "Keep" restores the items in Finder. Never
  auto-confirmed. Thresholds are core settings per platform (the Docker agent
  will add its unmounted-share guard on the same mechanism).
- **Deletion batches**: every delete burst carries one batch id to the server,
  so the trash can restore the whole deletion at once.
- **Credit exhausted** → uploads wait ("Waiting for credit"), nothing is
  dropped and nothing is charged without the user seeing it.
- Sync never touches anything outside the own.audio folder.

## 5. Backend (`audio2`)

Contract changes land with `docs/android-client-guide.md` and
`docs/mobile-backend-api-spec.md` in the same commit (CLAUDE.md §3). Everything
is additive for shipped clients: `DELETE` keeps its route and still makes the
item disappear for them.

### 5.1 Trash

- Migration: `trashed_at TIMESTAMPTZ NULL`, `trashed_by UUID NULL REFERENCES
  users ON DELETE SET NULL` on `audiobook_books`, `music_tracks`,
  `music_playlists`;
  partial indexes on `trashed_at IS NOT NULL`.
- A third column `trash_batch UUID NULL` groups one deletion. `DELETE` routes
  accept an optional `X-Trash-Batch` header (the core sends one per burst,
  the web's `BatchBar` one per bulk action); without it each delete is its own
  batch. `GET /trash` returns it so clients can group rows.
- **Stored podcast episodes** get the same columns on `podcast_episodes`,
  meaning "the stored copy is in the trash": the episode row stays (it is part
  of the RSS list), `audio_object_id` stays linked until purge.
  `DELETE /podcasts/{id}/episodes/{ep}/download` trashes instead of deleting
  the object; restoring re-links nothing because nothing was unlinked.
- **Trashed rows are invisible everywhere except the trash endpoints.**
  Centralise in `audio2_can_access` where possible, then audit every query in
  `db/` that reads those tables — library lists, search, continue, albums and
  artists aggregates, playlists (entries of a trashed track are hidden, not
  removed), stats, recommendations, smart-playlist resolution, and the
  **Subsonic** surface (`TRACK_COLS`, `db::music`). One test per surface.
- The existing `DELETE /audiobooks/{id}`, `/music/tracks/{id}`,
  `/music/playlists/{id}`, `/podcasts/{id}/episodes/{ep}/download` now
  **trash**: set `trashed_*`, bump `updated_at`,
  write the tombstone (`record_deletion`). The routes still answer `204` with no
  body, so shipped clients are unaffected; `purge_at` comes from `GET /trash`.
- Permission: owner, **or** family admin (`FamilyContext::is_family_admin`)
  when the item is shared with that family. Anything else stays 404.
- New endpoints:
  - `GET /trash?scope=mine|family` — `family` requires family admin. Row:
    kind, id, title, owner, trashed_by, trashed_at, purge_at, size, batch,
    restore_charge_micro. No cover: cover routes go through the views, so a
    client shows a kind icon.
  - `POST /trash/{kind}/{id}/restore` — clears `trashed_*`, bumps
    `updated_at` so both feeds deliver it again.
  - `POST /trash/batches/{batch_id}/restore` — restores a whole deletion
    (only the items the caller may restore).
  - `DELETE /trash/{kind}/{id}` — purge now.
  - `POST /trash/empty?scope=…`
- Notification `item_trashed` to the owner when the actor is someone else
  (`data: {kind, id, title, by}`).
- **`trash_purge` job**, daily ("exists for today" pattern like
  `storage_billing`): hard-delete rows past `purge_at`, delete their media
  objects when unreferenced — this also fixes the audiobook orphan leak. Add it
  to `WORKER_JOB_TYPES` in **both** compose files (CLAUDE.md §6).
- **Tombstone pruning** actually scheduled (same job), retention **180 days**.
- **Billing** (§2 item 12):
  - The daily `storage_billing` run excludes trashed items: add the filter to
    the `*_REFS` queries in `db/billing.rs`.
  - **Restore charges the trash days**: one ledger charge at restore time,
    `size × whole days in trash × daily rate`, with the item named in the
    ledger line so the family sees what it paid for. Same clamping to the
    balance as the daily charge.
  - Purge after 30 days: nothing to charge.
- **Monitoring** (instance admin only):
  - Per family: bytes in the trash now, bytes restored and restore count in
    the last 30 days, share of the library that is in the trash.
  - A **flag** on a family whose restored bytes in 30 days exceed its library
    size, or that trashes and restores the same items more than 3 times in 30
    days — patterns that suggest someone is testing the billing. The
    back-charge already makes it pointless; the flag is there so we notice.
  - Exposed through the existing `require_admin` endpoints (`GET /admin/…`)
    for the admin page (§8).
- **Account deletion purges storage objects** immediately (GDPR; today it
  leaves them).
- Find out why `storage_sweep` is off in prod; turn it on as a safety net if
  there is no reason.

### 5.2 Music upload via presign

- `POST /music/tracks/from-upload {object_key, path?, original_filename,
  visibility?}`:
  `require_can_upload`; read tags from the stored object with the same code as
  the multipart handler (factor it out); create the track; return
  `TrackResponse`. Lifts the 100 MB limit to the presign's 5 GiB.
- Add the missing `require_can_upload` to the multipart route.

### 5.3 Sync feed v2

A new endpoint rather than extending `/library/changes`, which every client
reads as untyped summaries.

`GET /sync/tree?cursor=<opaque>&limit=500` →

```json
{
  "cursor": "…", "has_more": false, "reset": false,
  "items": [{
    "kind": "audiobook | music_track | podcast_episode",
    "id": "…", "updated_at": "…",
    "owner": {"id": "…", "display_name": "…"},
    "is_owner": true, "can_delete": true, "shared_with_family": false,
    "title": "…",
    "path": "Music/Moje oblíbené/track01.mp3",
    "files": [{"id": "…", "relative_path": "…", "size_bytes": 0,
               "sha256": "…|null"}]
  }],
  "removed": [{"kind": "…", "id": "…", "reason": "trashed | deleted"}]
}
```

- `path` is the item's stored path (§5.8): a book's folder, a track's or
  episode's file. `files` is used by books (relative paths inside the folder);
  a track or episode has one entry with an empty `relative_path`. `title` is
  only for messages in the attention list — the core needs no other metadata.
- A `podcast_episode` item adds `show: {id, title}` and is present only while
  the episode is stored (not trashed). Owner and `can_delete` come from the
  feed.
- `reset: true` when the cursor is older than tombstone retention → the client
  must reconcile the whole tree.
- Only what the caller can see; trashed items appear only in `removed`.

### 5.4 Reconciliation ids

`GET /sync/tree/ids` → `[{kind, id, updated_at}]` for everything visible.
Cheap (thousands of rows), and it catches revoked sharing without a new
tombstone mechanism.

### 5.5 Family shortcuts

- Table `sync_shortcuts (id, user_id, member_id, kind, container_kind NULL,
  container_ref NULL, created_at)`; `container_kind` is `book | album | show`,
  `container_ref` the book or feed id, or the album's release group /
  `album_artist + album` (§4.4). Unique per user + target.
- `GET /sync/shortcuts`, `POST /sync/shortcuts`, `DELETE /sync/shortcuts/{id}`
  — own shortcuts only; a target must be visible to the caller.
- Removing a member from the family deletes shortcuts pointing at them.
- The feed (§5.3) still returns everything visible; the **core** decides which
  family items are in the tree, so the rule lives in one place for every
  platform.

### 5.6 Storing new episodes on the server

- `podcast_feeds.auto_store BOOLEAN NOT NULL DEFAULT false`, set via the
  existing feed update route by the feed owner or a family admin.
- When a feed refresh finds new episodes on an `auto_store` feed, it enqueues
  the existing `episode_download` for each — so a paid feed's episode is kept
  while its link is valid, whether or not any Mac is running.
- Only episodes published after the switch was turned on; no backlog.

### 5.7 Stream/download URL for a file

Confirm the existing per-file stream URL endpoints return a presigned URL the
core can fetch with `Range` for audiobook files and tracks; if not, add
`GET /sync/files/{kind}/{file_id}/url`.

### 5.8 Paths

- Migration: `audiobook_books.folder_path`, `audiobook_files.relative_path`
  (today `relative_path` is only used to sort at upload and then dropped),
  `music_tracks.path`, `podcast_episodes.stored_path`. Unique per owner
  across kinds (the top-level folder is part of the path).
- **Backfill** every existing item with its default path (§4.1), resolving
  collisions with ` (2)`.
- Every creating route sets the path: `from-upload` / `from-uploads` take the
  client's path (Finder uploads); without one — app, web, iOS, Android,
  imports, multipart uploads, a stored episode — the server computes the
  default once. On a collision the server appends ` (2)` and returns the final
  path.
- Validation: the top-level folder must match the kind; no empty components,
  no `.`/`..`; ≤255 bytes per component; NFC.
- **No route changes a path afterwards.** Metadata edits, Identify, album
  artist changes, a show's new title — none of them touch it.
- Default-path code lives in the backend only; the core never computes a
  path.

### 5.9 Device holdings (groundwork, no UI yet)

- A device is the refresh-token chain of its session (`refresh_tokens.chain_id`,
  with `device_name`/`device_kind`, migration 0017). If the access token does
  not carry the chain id, add it as a claim.
- Table `device_holdings (chain_id, user_id, kind, item_id, since)`.
- `PUT /sync/holdings` — the calling device replaces its own set (full list;
  small enough) or sends `{added, removed}`; debounced by the client.
- `GET /sync/holdings?kind=&id=` — the caller's own devices holding an item.
  Never other members' devices.
- A revoked chain or sign-out deletes its rows; the purge job drops rows of
  purged items.

## 6. The sync core (`audio2-sync`, new repo)

A new repo next to the others, on the Forgejo host. Not inside `audio2`: the
backend is one crate whose DTOs depend on axum, and the core ships to clients
on its own cadence.

### 6.1 Crates

- `sync-core` — engine, DB, API client, tag reader (title/author for books
  dropped in Finder — the server reads no audiobook tags), queues, safety
  rules. `tokio`, `reqwest` (rustls), `rusqlite` (bundled), `lofty`
  (same tag library as the backend), `sha2`, `serde`, `tracing`.
- `sync-ffi` — `uniffi` bindings; builds an `.xcframework` (arm64 + x86_64)
  for the Mac, consumed as a SwiftPM `binaryTarget`.
- `sync-cli` — the test harness: syncs a **plain folder** against a backend
  (full download, no placeholders). This is how the core is tested end-to-end
  and it is the seed of the later Docker agent (§9).

### 6.2 Local DB (SQLite)

- `items` — identifier, kind, server id, server path, parent identifier,
  name, owner,
  `is_owner`, `can_delete`, size, sha256, server `updated_at`, local state
  (`cloud | downloading | local | uploading | error`), kept flag, error text.
- `uploads` — local path, target (track / book), state, object keys so far,
  attempts, error.
- `delete_holds` — batch id, identifiers, state (`held | confirmed | kept`).
- `change_log` — monotonically increasing sequence per change; File Provider
  sync anchors are positions in it.
- `meta` — feed cursor, account id, last reconciliation.

### 6.3 Behaviour

- **Pull**: page `/sync/tree`, apply to DB (server paths as given, family
  items under `Family/<Member>/`), append to `change_log`. Family items enter the tree only when a shortcut covers them
  (`/sync/shortcuts`, refreshed with every pull); adding or removing a
  shortcut adds or removes that subtree locally and never touches the server
  content. Reconcile with `/sync/tree/ids` on start, every 6 h, and on
  `reset`. The poll interval is short while the app is in front and long
  otherwise (no push exists).
- **Download** (`fetch(identifier) → path`): resolve URL, ranged GET with
  resume, verify sha256 (size when sha256 is still null), hand over the file.
- **Upload**: presign → PUT → `from-upload` / `from-uploads` with the path the
  user gave; if the server answers with ` (2)` (conflict), the local item takes
  that name. Audiobook folders
  wait until quiet (no new children for 5 s) before they become one book. A
  failed file restarts from zero (single PUT); the app says so for large files.
- **Delete**: resolve folder → items the user may delete; apply the
  bulk-delete hold; call `DELETE`; mark locally.
- **Import** (`import(files, metadata)`): used by the app for
  Navidrome/Audiobookshelf imports and for Add Book / Add Music (§2 item 11). Same
  queue as Finder uploads, metadata carried through.
- **Credit**: read the family's `depleted` flag before each upload batch.
- **Holdings**: report what is materialised on this device to
  `/sync/holdings` (§5.9) after downloads and evictions, debounced.
- **Auth**: the core never stores tokens. The host passes a token-provider
  callback (Mac: Keychain + existing `AuthSession` refresh; Docker later:
  its own store).

### 6.4 FFI surface (sketch, settled in the spike)

`Engine::open(config)`, `item(id)`, `children(container, page)`,
`changes_since(anchor)`, `fetch(id, dest, progress)`, `create(local_path,
parent, name) → Item | Refusal`, `delete(ids) → Done | Held(batch)`,
`resolve_hold(batch, confirm)`, `import(files, metadata)`, `set_kept(id,
bool)`, `status()`, `attention()`, `pause(bool)`, plus an event callback for
changes.

### 6.5 Tests

Unit tests against a fake server (path conflicts, family paths, resurrection,
holds, reconciliation, `reset`). End-to-end with `sync-cli`
against the local Docker backend: upload a folder, delete on "device A", bring
"device B" back from offline, revoke sharing, kill mid-transfer and resume.

## 7. Mac (`audio2-mac`)

### 7.1 Targets and signing

- New target **`Audio2MacFileProvider`** (`.appex`,
  `NSFileProviderReplicatedExtension`), embedded in the app. No File Provider
  UI extension: every confirmation is a Finder alert (§7.2).
- **App Group** `Z53P4WUKK2.app.audio2.mac` (team-prefixed). Works with the
  current free team and no provisioning profile (S0). The core DB lives there;
  other processes, Terminal included, cannot read that container.
- Finder's sidebar and the folder on disk take the **app bundle's file name**,
  not `CFBundleName` (found in S6: `Audio2Mac.app` gave "Audio2Mac" although
  `CFBundleName` was "own.audio"). The app is built as `own.audio.app`; the
  folder is `~/Library/CloudStorage/own.audio-<domain display name>`.
- **Keychain access group** so the extension reads the tokens:
  `AuthTokenStore` gains an optional access group (nil keeps iOS unchanged —
  it is a shared package, so build and test the iOS apps too).
- Extension entitlements: sandbox, network client, the app group, the keychain
  group. Developer ID enrolment is still blocked; development signing is
  enough to build and test.

### 7.2 Extension

A thin adapter: each `NSFileProviderReplicatedExtension` call maps to one core
call.

- **Never withhold a capability.** Every item gets reading, renaming,
  reparenting, trashing, deleting, and folders adding sub-items. A missing
  rename/reparent/add capability makes the system set `uchg` on the file,
  which blocks even `rm` in Terminal and which **Finder draws as a lock** —
  exactly what CLAUDE.md forbids (S0).
- **Refusals are Finder alerts** declared in `NSFileProviderUserInteractions`
  (Info.plist) with `Continue` off, so Finder stops the action before it
  happens: Rename, Move within the domain, Trash/Delete of an item the user
  may not delete (`sourceItem.userInfo.canDelete == NO`), MoveIn/CopyIn into
  the root, `Family/` or `Podcasts/`. For MoveIn/CopyIn the destination must
  be tested in a **SubInteraction** under a top-level `action == "MoveIn" OR
  action == "CopyIn"` rule — the same predicate combined in one rule never
  matched (S0). Items carry `userInfo` flags for these rules.
- **The extension refuses as the backstop** (Terminal, scripts): a rename is
  answered with the unchanged item, a reparent and a refused create with
  `cannotSynchronize`. A refused create leaves the file on disk with Finder's
  upload-error badge — so the alerts must catch every normal case.
- **Catastrophe hold = a Finder alert** (§2 item 14): `(action == "Trash" OR
  action == "Delete") AND sourceItemsCount > 100`, destructive Continue.
  Confirmed working with a Czech title and item count (S0). The core's own
  hold stays only for bursts that bypass Finder (Terminal, the Docker agent).
- **MoveOut** (dragging out) gets an alert saying it removes the item from
  own.audio; continuing reaches the extension as `deleteItem` → trash.
- **Trash**: `supportsSyncingTrash = YES`. Finder's Move to Trash arrives as a
  reparent to the trash container (→ server trash) and **Put Back** as a
  reparent out of it (→ restore) — Finder's Trash *is* the own.audio trash
  (S0). Emptying Finder's Trash arrives as `deleteItem` → delete forever.
- **Family items** set `isShared` and `ownerNameComponents`; Finder shows
  "Shared by Petr" and a shared-folder icon on its own (S0). No custom
  decoration, **never a lock** (CLAUDE.md).
- **Keep Downloaded** is `contentPolicy = .downloadEagerlyAndKeepDownloaded`
  (macOS 13+); set on a show folder, its episodes download by themselves (S0).
  The app triggers downloads with `requestDownloadForItem` and frees space with
  `evictItem`.
- **Ignore system files**: Finder writes `.DS_Store` (the system keeps it out
  of `createItem`), but `.localized`, `._*` and similar do reach `createItem`
  and must be refused silently. Folder `contentModificationDate` changes
  arrive as `modifyItem` and are ignored.

### 7.3 Domain lifecycle

- Sign-in registers the `own.audio` domain. It appears in Finder's sidebar at
  once, but right after `add` the domain reports **`userEnabled = false`**;
  in S0 it became enabled once the location was opened in Finder, with no
  prompt. The app checks `userEnabled` after sign-in and, if it stays off,
  explains where to switch it on (System Settings → File Providers).
- Sign-out removes the domain with **`preserveDirtyUserData`**: anything never
  uploaded is left in a plain folder
  (`~/Library/CloudStorage/own.audio-… (date)`) instead of being lost — S0
  confirmed the preserve modes keep local files that way. The confirmation
  says so ("Downloaded files will be removed from this Mac; they stay in
  own.audio. Files not yet uploaded are kept in …").
- One domain per signed-in account.

### 7.4 App ↔ extension

The app needs state (is this downloaded? what is uploading? what needs
attention?) and sends commands (keep, import). Channel: an
`NSFileProviderServiceSource` XPC service exported by the extension, reached
with `NSFileProviderManager.service(named:for:)` — works (S0). The core runs
in the extension only, so the Rust library is linked only there (~1 MB per
architecture, S0).

For playback the app reads an item's state without downloading it
(`getUserVisibleURL` + `ubiquitousItemDownloadingStatus`, security-scoped
access) — S0 confirmed this triggers no fetch. **Opening a dataless file
downloads the whole file before the first byte returns**, so the player
streams anything not downloaded and plays local files only when they are.

### 7.5 App changes

- **Playback**: a new `LocalFileProvider` conformer backed by the core — a
  materialised file plays from the domain, anything else streams (§7.4). A
  second conformer
  for Local Folders; both composed.
- **Row badges**: the existing download icon from core state. No other
  content-state icons.
- **Context menus** on books, albums, tracks, playlists: Show in Finder,
  Keep on This Mac / Remove Download, Move to Trash (owner or family admin) —
  same confirmation rules as the web (§2 item 13): Undo toast for your own
  item, a dialog for bulk and for another member's item.
- **Sync status** at the bottom of the sidebar (like Music's "Updating…"):
  Up to date / Uploading 12 items / Needs attention (2). Click → popover with
  current transfers, the attention list, held deletes (Delete / Keep), Show in
  Finder, Pause.
- **Trash** screen in the sidebar: mine, plus a Family scope for admins;
  Restore, Delete Forever, Empty Trash; "deleted forever in N days"; Restore
  states what it will charge for the days in the trash (§2 item 12). Rows
  are grouped by deletion with **Restore all** per group (§2 item 14).
- **Add Book / Add Music / Import Music Files / drop on the window** →
  `core.import`. `MediaStorageChoice` (Local Only / Cloud / Both) is removed.
- **Navidrome / Audiobookshelf**: "Import to own.audio" fetches to a temp
  folder and calls `core.import` with metadata; "Download to this Mac" saves
  into a Local Folder.
- **Family shortcuts**: **Add to own.audio folder** / **Remove from own.audio
  folder** on a member's Audiobooks/Music/Podcasts and on a family book, album
  or show; a small marker on items already in the folder. The Family screen
  lists the account's shortcuts.
- **Podcasts**: an episode's "Download" becomes **Save to own.audio**
  (server-side store); a show gets the **Save new episodes to own.audio**
  switch (§5.6). Offline on this Mac is Finder's Keep Downloaded on the show
  folder. Subscribing and unsubscribing stay in the app.
- **Settings**: one **Sync** pane replaces Downloads and Mirror: the Finder
  location (not choosable — the system decides) with Show in Finder, Pause
  sync, Local Folders list.
- **Localisation**: every new string in Czech, terms per
  `docs/translation-glossary.md` (Koš, Přesunout do koše, Ponechat na tomto
  Macu…).

### 7.6 Local Folders (Local Only)

The user adds folders (security-scoped bookmarks). The app indexes them
(FSEvents + tags), shows them in a **Local Folders** sidebar item, plays them,
and never uploads or deletes anything there. "Upload to own.audio" copies a
file in via `core.import`; the original stays.

### 7.7 Moving off the old folder

One-time sheet for machines that used the old download folder (at least the
developer's, ~3 GB): "N GB are copies of items in own.audio — remove them?" /
"M items exist only on this Mac — kept; this folder is now a Local Folder."
Nothing is removed without the explicit confirmation.

### 7.8 What retires

Mac usage of: `MirrorEngine`, `MirrorPlanner`, `MirrorRuleContext`,
`MirrorJobResumption`, `MirrorLibraryReconciliation`, `EvictionPolicy`,
`LibraryVerifier`, `TombstoneDetector`, `BandwidthBudget`, `UploadSpool`,
`MirrorJob`/`MirrorItem`/`MirrorRuleSet`/`MirrorPin` (+ entities and
repositories), `StorageBreakdownItem`, `LibraryFolderScanner`,
`IgnoredScanPathsStore`, `LibraryFolderStore`, the LocalOnly repositories and
views (replaced by Local Folders), `DownloadsView`, `TransfersView`,
`StorageBreakdownSheet`, `MirrorPreflightSheet`, `MirrorProgressSheet`,
`MirrorRulesSettingsView`, `DownloadsSettingsView`, `OrphanedDownloadsSheet`,
`LibraryScanResultsSheet`, `MediaStorageChoice`, `UploadActivityCenter`,
`SidebarItem.downloads`/`.transfers`, the toolbar indicators, the
`.downloads`/`.mirror` Settings tabs, the local podcast auto-download and
retention rules (`PodcastFeedRulesStore`, Mac episode downloads), and the
matching `AppContainer` sections (≈ lines 855–1010, 1354–1930, 2038–2190,
2478–2700).

Code in shared packages is deleted only after grepping `audio2-ios-book`,
`audio2-ios-music`, `audio2-ios-podcast` and `audio2-tvos` — iOS still uses
`DownloadManager` and `RangeTransfer` for its own downloads.

`audio2-mac/CLAUDE.md` rule "Never delete a user's file without asking. A
tombstone offers deletion" is replaced by: *sync never touches files outside
the own.audio folder; inside it, deletion follows the server, and the trash is
the safety net.* The old mirror plans get a note pointing here.

## 8. Web (`audio2/frontend`)

- `api/trash.ts` + types; `pages/trash/TrashPage.tsx`, route `trash`, sidebar
  item under Storage & credit.
- Tabs **Mine** / **Family** (family admins, `isFamilyAdmin(my_role)`). Row:
  cover, title, kind, owner (Family tab), deleted by, deleted at, "Deleted
  forever in N days". Rows grouped by deletion ("120 items deleted at
  14:32") with **Restore all** per group. Restore; Delete Forever and Empty
  Trash behind a `Dialog`.
- All single-item deletes of your own items (book menu, track menu, edit
  sheets, playlists) become **Move to Trash** + toast with Undo (§2 item 13),
  replacing the three current patterns. A family admin deleting another
  member's item gets a `Dialog` naming the owner. `BatchBar` keeps its dialog
  with trash wording. Check `lib/toast.ts` supports an action button; add it
  if not. Account
  deletion stays as it is and says it is immediate.
- Delete is offered when `is_owner || (isFamilyAdmin(my_role) &&
  shared_with_family)`; the server decides.
- `NotificationsPanel`: `item_trashed` with a Restore button while the item
  is still in the trash.
- Storage & credit: "Trash: X GB — free unless restored"; restore charges
  appear in the ledger by item name. The Restore action says what it will
  cost ("Restoring charges 12 days in the trash: $0.04") before it runs.
- Instance admin page (`pages/admin/AdminPage.tsx`): trash monitoring table
  per family with the flag (§5.1).
- **Add to own.audio folder** / **Remove from own.audio folder** on family
  items (book, album, show) and per member and kind on the Family page — the
  same shortcuts as the Mac, since they belong to the account.
- Browser offline copies (IndexedDB) of a removed item are dropped on the next
  library refresh — check whether that happens today.
- Vitest for the permission helper and the countdown; a manual Playwright
  sweep as in the parity plan. English only, like the rest of the web app.

## 9. Later — designed for, not planned here

- **Docker agent**: `sync-cli` grown into a daemon with a small React UI
  (status, device-code login, which top folders sync on this machine, attention
  list, held deletes). Needs a marker file (`.own-audio`) so an unmounted NAS
  share is never read as "everything was deleted", inotify plus periodic
  rescans (network shares don't deliver inotify), and UI bound to LAN only.
- **Windows** (Cloud Files API) and **Linux** (sync folder, no file-manager
  badges) on the same core.
- **iOS Files** (File Provider on iOS).
- **"Where is this file"** — the UI (Finder context menu via a File Provider
  UI extension, the app, the web) and holdings reporting from the iOS,
  Android, tvOS and web clients, on the groundwork of §5.9.
- Resumable large uploads (S3 multipart presign).
- Renames/moves in Finder and per-file book edits, if ever wanted — only a
  path change now, no metadata involved (§2 item 10).
- **Export with tags**: a copy of a file with own.audio's metadata written
  into it, for tools that read tags (the stored file itself is never changed).
- Windows: display mapping of characters Windows forbids in names that Macs
  allow (`:`, `?`, …).
- Trash UI in the iOS, Android and tvOS apps; until then they keep working,
  deletes simply go to the trash.
- Public site and roadmap (`audio2-www`) once it ships — not before (www
  CLAUDE.md §3).

## 10. Phases

**Legend:** ✅ done · 🚧 in progress · ⬜ not started
**Progress:** 10 / 10

Every phase ends with its repo's gate: tests green, run against the local
backend, manual check, CHANGELOG entry, contract docs where the API changed.

### ✅ S0 — spike — done 2026-09-25

A File Provider extension over a Rust core (uniffi 0.32) with a fake tree,
built in `~/vscode/audio2-fp-spike` (throwaway, not in any repo) on macOS 27 /
Xcode 27 with the free team, driven from Terminal and checked in Finder by
the user. The findings are folded into §7.1–§7.4; in short:

- [x] **Enabling**: the location appears in Finder's sidebar at once, named
      after the app's `CFBundleName`; right after `add` the domain is
      `userEnabled = false`, and it became enabled once opened in Finder, with
      no prompt.
- [x] **Keep Downloaded**: `contentPolicy` (macOS 13+) on an item or folder;
      `requestDownloadForItem` / `evictItem` from the app. A show folder with
      the eager policy downloaded its episode by itself.
- [x] **Trash**: with `supportsSyncingTrash`, Move to Trash and Put Back
      arrive as reparents to/from the trash container. **Dragging out**
      arrives as `deleteItem`.
- [x] **Refused create**: the file stays on disk with Finder's upload-error
      badge. A Finder alert with Continue off stops it before it lands —
      that is the primary mechanism.
- [x] **Capabilities**: withholding rename/reparent/add sets `uchg` and Finder
      draws a **lock**. Replaced by full capabilities + Finder alerts; rename,
      move, deleting another member's item and dropping into
      `Podcasts`/`Family`/root are all refused by alert, no lock anywhere.
- [x] **2 GB create**: `cp` returns at once (APFS clone); uploaded in 57 s.
      Killing the extension mid-upload: the system relaunched it and called
      `createItem` again from zero.
- [x] **App ↔ extension**: `NSFileProviderServiceSource` XPC works.
- [ ] **`.localized`**: not verified — the test Mac's Finder runs in English.
      Fixed English names stand (§2 item 9); retest on a Czech system later.
- [x] **5,000 items** in one folder: first listing 5 s (extension share
      ~0.5 s), then 26 ms.
- [x] **Playback**: state readable without a download; opening a dataless file
      downloads all of it first — stream until downloaded.
- [x] **uniffi + Swift 6**: generated bindings compile clean in Swift 6 mode;
      the FileProvider SDK needs small `@unchecked Sendable` wrappers around
      completion handlers. Release, universal: ~1 MB of Rust per architecture
      per binary; link it into the extension only.
- [x] **App Group / sign-out**: team-prefixed group works without a
      provisioning profile. Removing the domain with a preserve mode leaves
      local and never-uploaded files in a dated plain folder.

### ✅ S1 — backend: trash (`audio2`) — done 2026-09-25

- [x] Migration 0077: `audiobook_books`/`music_tracks`/`music_playlists` →
      `…_all` + views hiding trashed rows (one mechanism instead of auditing
      ~150 queries); stored episodes via `trashed_audio_object_id`. Views
      checked on the real database first: base defaults and `RETURNING` work
      through them, trashed rows are skipped by UPDATE/DELETE, FKs stay on
      `…_all`. CLAUDE.md §6 records the add-a-column rule.
- [x] `DELETE` → trash (books, tracks, playlists, stored episodes, Subsonic
      `deletePlaylist`); owner or family admin for shared items; 204 kept.
- [x] `/trash` endpoints incl. batch restore; `item_trashed` notification.
- [x] `trash_purge` daily job (purges with objects, prunes tombstones at 180
      days) in both compose files.
- [x] Billing excludes the trash (views + book files through a live book);
      restore charges the days in the trash (`trash_restore_charge`, clamped,
      off when charges are off); account deletion removes the objects it
      orphans (snapshot of referenced objects under the user's and family's
      prefixes before the delete).
- [x] `GET /admin/families/trash` with the flag; no daily log line (the
      endpoint is enough).
- [x] `storage_sweep` was never in any `WORKER_JOB_TYPES` — an oversight, not
      a decision (its commit describes the canary backlog it was meant to
      clear). Enabled in both files. ⚠ In production it starts deleting
      unreferenced objects older than 7 days on the first deploy.
- [x] Contract docs: `android-client-guide.md` §8b,
      `mobile-backend-api-spec.md` §4d.
- Verified: `scripts/trash_test.py` (53 checks against the live local stack:
  permissions, lists, Subsonic, tombstones, restore + charge, batch, purge
  now, purge job, books, playlists, a stored episode, account deletion) and
  the existing `family_sharing_test`, `mobile_sync_test`, `stats_test`,
  `backend_smoke_test` — all green. Library intact after the first sweep
  (no item without its object).

### ✅ S2 — backend: sync endpoints (`audio2`) — done 2026-09-25

- [x] `POST /music/tracks/from-upload` (tags read from the stored file,
      streamed to a temp file); `require_can_upload` on the multipart route,
      which shares one `create_track` with it
- [x] `GET /sync/tree` (books, tracks, stored episodes), `GET /sync/tree/ids`;
      §5.7 confirmed — the existing stream routes return a presigned `GET`
      that honours `Range`, so no new URL endpoint
- [x] Auto-store (§5.6): `PUT /podcasts/{id}/auto-store`, queued after every
      refresh, new `episode_download` job in both compose files
- [x] Family shortcuts table + endpoints; a trigger on `family_members` drops
      them when someone leaves (§5.5)
- [x] Paths: default on every creating route, client paths kept, validation,
      case-insensitive ` (2)` among live items, nested book folders refused
      the same way; a restore whose path was taken comes back as ` (2)` (§5.8)
- [x] Device holdings table + endpoints; sign-out and purge drop rows (§5.9)
- [x] Contract docs: `mobile-backend-api-spec.md` §8f (+ §5, §6a, §7),
      `android-client-guide.md` §8a and podcasts
- Deviations from §5:
  - **Paths live in one table, `sync_paths`**, not a column per kind:
    uniqueness is across kinds, and a column on the book/track tables would
    mean recreating the trash views. A trashed item keeps its row.
  - **The feed reads a trigger-fed change log (`sync_changes`)**, not
    `updated_at`: a long upload transaction stamps its rows minutes before
    it commits and a timestamp cursor skips them. The cursor is the oldest
    running transaction id; a row only says "look at this item again", which
    also turns unsharing and per-item grants into `removed: hidden`.
    Whole-kind member policies are left to `/sync/tree/ids`.
  - **Backfill is lazy**: an item gets its default path the first time the
    feed or a creating route sees it, not in the migration (Rust computes the
    names). A first full sync of a big old library pays for it once.
  - `auto_store_since` (a timestamp, not a boolean) decides the "no backlog"
    rule; `episode_download` did not exist and was added.
  - A loose book file is a book whose path is the file and whose only file
    has an empty `relative_path` — the same shape as a track.
  - Found on the way: Subsonic `deletePodcastEpisode` unlinked a stored copy
    for anyone who could see the show; it now trashes with the REST rule.
- Verified: `scripts/filesync_test.py` (67 checks against the local stack,
  incl. a transaction committing after the feed was read, paging, a reset
  cursor, a served RSS feed for auto-store) plus `trash_test`,
  `family_sharing_test`, `mobile_sync_test`, `stats_test`,
  `backend_smoke_test` — all green; 231 unit tests.

### ✅ S3 — web trash (`audio2/frontend`) — done 2026-09-25

- [x] `/trash` page and sidebar item: Mine / Whole family (family admins),
      grouped by deletion with Restore all, Delete Forever, Empty Trash; a
      restore costing a cent or more states the price and asks first, anything
      less runs at once (a fraction of a cent read as "$0.00" and "free").
- [x] Move to Trash + Undo toast for your own items everywhere (track menu,
      edit sheets for tracks, playlists and books, the book menu, duplicates);
      a dialog for bulk (`BatchBar.onTrash`, one `X-Trash-Batch` per gesture)
      and for a family admin deleting someone else's shared item. Podcast
      unfollow keeps its permanent-delete dialog (`onDelete`).
- [x] `item_trashed` notification with a Restore button; Storage & credit
      shows the trash and that it is not charged; restore charges show in the
      ledger through their note.
- [x] Browser offline copies: a song trashed from the web leaves this
      browser's downloads at once, and on start-up saved songs the library no
      longer has are dropped (signed in and online only).
- Deviation: the admin dialog says "another family member", not the name —
  item DTOs carry no owner name. Add `owner_name` to them if it matters.
- Verified: `tsc`, eslint, 51 Vitest tests (incl. `lib/trash.test.ts`), build;
  driven in Chromium against the local stack as a family admin and a member —
  own delete + Undo, admin delete of a member's song with the dialog, batch
  of two + Undo, trash page both scopes, cost dialog, cheap restore without
  one, the member's notification Restore, Storage & credit line, book menu,
  dark mode.

S3 can run in parallel with S4–S5.

### ✅ S4 — core: pull and download (`audio2-sync`) — done 2026-09-25

- [x] Repo (workspace `crates/sync-core`, `crates/sync-cli`), `CLAUDE.md`,
      `.gitignore`, `CHANGELOG.md`; SQLite DB with schema versioning and tests
- [x] Feed paging with the cursor stored only once a round is in; `reset`
      and a first snapshot drop whatever they do not mention; reconciliation
      against `/sync/tree/ids` on first use, every 6 h, and a fresh snapshot
      when the server shows an item never seen; download through the stream
      routes into `*.audio2-partial`, resumed with `Range`, verified by
      sha256 (size until the server has one) — a damaged left-over partial
      gets one clean retry
- [x] Family shortcuts decide which family items are in the tree
      (`project::covers`), under `Family/<Member>/`, members with the same
      name kept apart as `Anna (2)`
- [x] `audio2-sync` CLI, plain-folder mode, download direction; refreshes its
      token and signs in again when the refresh token is refused
- Deviations from §6.2:
  - Two tables instead of one `items`: `entries` holds the feed as received,
    `nodes` the tree built from it and the shortcuts — rebuilt and diffed on
    every pull, so the projection stays a pure, unit-tested function.
  - Folder identifiers are `dir:<owner>:<lowercased path>` (show folders
    included — no `pf:`), so folders differing only in case are one, as on
    the disks they end up on. Deleting a show folder still reaches every
    episode under it.
  - A `materialized` table (plain-folder hosts only) records the size and
    mtime of every file the CLI wrote; it only removes, moves or replaces a
    file still exactly as it left it.
  - `uploads` and `delete_holds` come with S5, with the code that uses them.
- Download-only for now: a file deleted from the CLI's folder is fetched
  again. In S5 that becomes a delete, per §2 item 3.
- Verified: 27 tests (projection, DB, download incl. resume and a damaged
  partial, the engine against a fake server: paging, removals, reset,
  reconciliation, shortcuts, token refresh, account switch), clippy clean;
  `scripts/e2e_download.py` — 16 checks with the CLI against the local
  backend: first sync, trash and restore, an edited file and a foreign file
  kept, a family shortcut added and removed (subtree pruned), an interrupted
  download.

### ✅ S5 — core: upload, delete, safety — done 2026-09-25

- [x] Music and audiobook-folder upload (quiet 5 s, `CD1`/`Disc 2` folders
      part of the book, title/author from tags else folder names, per-file
      durations from tags, an image as cover), refusals with a reason and an
      attention list, credit wait (`WaitingForCredit`, the answer cached 60 s)
- [x] Delete as one batch per gesture; a shortcut's folder removes the
      shortcut; holds for non-Finder bursts (> 100 in 10 s, or a whole
      top-level folder), answered with keep/confirm; resurrection guard
- [x] `import()` with metadata (track or book, server default path)
- [x] `sync-cli` both ways; end-to-end scenarios
- [x] `sync-ffi` + `scripts/build-xcframework.sh` (arm64 + x86_64,
      macOS 14) + `swift/Smoke`
- Deviations and decisions made on the way:
  - **A book folder's identifier is its path** (`dir:<owner>:<path>`), not
    `ab:<book id>`: Finder creates the folder before its files, and the
    files' `createItem` needs the folder's identifier before the book exists.
  - **No persistent `uploads` table.** File Provider calls `createItem` again
    after a crash (S0) and the CLI rescans, so a queue would only duplicate
    that; a book's files wait for one shared upload in memory. A failed
    upload restarts from zero, as planned.
  - Finder deletes are trusted (Finder asked already); the core's hold
    applies to `DeleteSource::Other`. One file of a book is refused — a book
    is deleted as a folder.
  - A file changed locally is not uploaded again (items are immutable on the
    server); the CLI keeps it and reports it.
  - The generated Swift module builds in Swift 5 mode: uniffi's helpers for
    async Swift callbacks (the token source) fail Swift 6's region checks.
    Apps importing it stay in Swift 6. Linked, the core adds ~8 MB per
    architecture (TLS, SQLite, tags, tokio) — the spike's 1 MB had none.
- **Open for the user:**
  - A cover image dropped into a new book folder becomes the book's cover
    and is not kept as a file in the folder (it disappears there). Keep it,
    or refuse images?
  - Files put into the folder are uploaded **private** (the server's
    fail-closed default); sharing stays an app action. Or should Finder
    uploads follow a per-account default?
- Verified: 38 tests (incl. 9 for writing against a fake server: refusals,
  a track taking the server's ` (2)`, credit wait, one book from a quiet
  folder with a disc folder and a cover, a batch delete, a shortcut folder,
  held bulk deletes kept and confirmed, imports), clippy clean;
  `scripts/e2e_two_way.py` — 29 checks with two devices against the local
  backend; `scripts/e2e_download.py` — 16; `swift/Smoke` — a Swift 6 program
  calls the engine, the engine asks Swift for a token, the error comes back
  typed.

### ✅ S5b — companion files and the family part of the tree — done 2026-09-25

Decisions §2 items 16 and 17 (from the S5 questions).

- [x] Backend: companion files — `companion_files_all` behind a trash view
      (migration 0079), paths in `sync_paths`, change-log trigger, trash kind
      `companion_file`, billing, presign kind, `/sync/files` create / stream /
      visibility / delete, in `/sync/tree` and `/ids`
- [x] Backend: cover rule on upload (cover/folder/front, else the only
      image; embedded art wins), `use-as-cover`; `.lrc` → the track's lyrics
- [x] Backend: `/sync/tree` carries `me` for `Family/<Me>`
- [x] Core: own shared items under `Family/<Me>/` (namespace `<me>~family`,
      the three fixed folders always there); companion files in the tree,
      family ones where a shortcut covers what they sit with; a file put in
      `Family/<Me>/` uploads shared; `move_node` shares or unshares, every
      other move refused; companions uploaded instead of refused, a new
      book's companions right after the book
- [x] Web trash lists companion files; contract docs (`mobile-backend-api-spec`
      §8f, §4d, §6a; `android-client-guide` §8b)
- Deviations:
  - Nesting of paths is refused between items only; a companion file may sit
    inside a book folder (it would otherwise have taken the book's ` (2)`).
  - Moving a stored episode shares its whole show — episodes are shared
    with their show on the server.
  - The CLI recognises a move as a file of its own gone from one side and
    the same file (same size) at the same inner path on the other.
- Verified: backend `filesync_test.py` 85 checks (18 new for companion files
  and `me`) and the S1/S2 suites; core 44 tests (new: shared side, local
  folders on either side, family companions, uploads into `Family/<Me>`,
  share/unshare moves); `e2e_two_way.py` 41 checks incl. album and book
  images kept and used as covers, a file uploaded shared, a move sharing and
  unsharing followed by the other device; `e2e_download.py` 16; Swift smoke.

### ✅ S6 — Mac: Finder, read-only — done 2026-09-25

- [x] Extension target `Audio2MacFileProvider`, App Group; no keychain group (see below)
- [x] Domain lifecycle (add on sign-in, remove with `preserveDirtyUserData` on sign-out);
      enumerate (folders, working set, changes from the core's change log), fetch; Keep
      Downloaded as `contentPolicy` from the core's `kept` (`set_kept` over XPC — the app's
      UI for it is S8)
- [x] Holdings reported after downloads and on `materializedItemsDidChange` (evictions)
      — `Engine::report_holdings`, sent only when the set changed
- [x] Family decoration: `isShared`, owner name from `Family/<Member>/`, never a lock
- Deviations:
  - **The extension has its own session**, not a shared Keychain group: a keychain access
    group needs a provisioning profile on macOS, and two processes refreshing one chain
    present each other's rotated tokens, which the server treats as theft. The app forks a
    second chain on sign-in (`POST /auth/fork`, new in the backend) and hands it over through
    the XPC service; the extension stores it under its own Keychain service and refreshes on
    its own. Its device shows as "own.audio in Finder (…)" — which is also the device its
    holdings belong to. The shared Swift packages are unchanged, so iOS is untouched.
  - Read-only means every write is stopped by one Finder alert per kind (add, rename/move,
    delete/move out) until S7; the extension refuses the same as the backstop.
  - Pulls: on first use and whenever a Finder callback finds the tree older than 60 s, plus
    every 2 minutes from the app while it runs; a reconcile first when due.
  - `userEnabled` is not checked yet — its explanation belongs to the S8 settings pane.
- Found in Finder and fixed: default book file names numbered twice (backend 0.1.39,
  migration 0081 — the rename reached Finder on its own, which also proved the change path);
  the location was called "Audio2Mac" (the bundle is now `own.audio.app`, and a location
  registered under the old name is re-added).
- Verified with the user's canary account on this Mac: the location lists the library
  (1,192 files: private books and episodes, the shared side under `Family/Kornel`), a file
  downloads and plays when opened, the delete alert stops a delete of a file and of a folder,
  and a server-side rename arrives in Finder. Backend `/auth/fork` 6 checks; core
  `report_holdings` test; all 81 migrations apply on PostgreSQL 16.14 (production's version).

### ✅ S7 — Mac: Finder, write — done 2026-09-26

- [x] `createItem` → the core's `create_folder` / `create_file` (a new book folder resolves
      once the book is in own.audio); refusals come back as `cannotSynchronize` with the
      reason and go to the attention list; Finder's own `.`-files are `excludedFromSync`;
      no credit → `insufficientQuota`
- [x] Finder's Trash is the own.audio trash (`supportsSyncingTrash`): Move to Trash →
      `trash_from_finder` (one batch, kept in the tree under the trash container with its
      identifiers), Put Back → `put_back` (the whole deletion in one call), Empty Trash →
      `purge_trashed`. Restored elsewhere → leaves the Trash on the next pull
- [x] `deleteItem` outside the Trash: Finder requests trust Finder's alert; anything else
      (Terminal, scripts) goes through the core's hold and is `deletionRejected` while held
- [x] Moves: private ↔ `Family/<Me>` shares/unshares (`move_node`); every other move and
      every rename is answered with the item as it is, which undoes it
- [x] Finder alerts (Info.plist): nothing into the root, `Family`, a member folder,
      another member's tree or Podcasts (SubInteractions, identifiers +
      `userInfo.acceptsFiles`); rename refused; a move asks "Share or stop sharing?";
      someone else's item cannot be trashed; more than 100 items and Move Out confirm
- Deviations:
  - Finder alert texts and the core's refusal reasons are translated in the extension, in
    the language the app runs in (a per-app language does not reach the extension).
  - A domain registered without trash syncing (or under the old app name) is removed and
    added again by the app.
  - File content versions are constant — own.audio never changes a file — so a checksum
    arriving later does not make Finder download a file again.
  - Put Back a day or more later charges those days like any restore; the app says so
    in S8 (Finder cannot show a price).
- Found with the user's first real drop (three books, an album) and fixed:
  - **The system creates at most 16 files at once**, so a book folder arrives in batches;
    everything after the first 16 failed. Now a batch that starts while the book uploads
    waits for it and is added to it, and audio put into an own existing book is added too
    (`POST /audiobooks/{id}/files/from-uploads`, backend 0.1.40) — a change from §4.3,
    which refused it.
  - **Finder gives names decomposed (NFD), the server composes them (NFC)**: a book folder
    with diacritics changed identifier when the book was created, and its later files found
    no parent (`noSuchItem`). Names and folder identifiers are NFC in the core now.
  - **Never re-add the domain or call `reimportItems` casually**: both made the system set
    every local, not-yet-uploaded file aside in copies named "Music 2", "Audiobooks 2"…
    (`domains()` does not report `supportsSyncingTrash`, so a check of it re-added at every
    launch; a reimport at launch did the same). Both removed. The copies were cleaned up:
    99 chapters moved back into their books and uploaded, the rest were duplicates.
- Verified from Terminal on this Mac, canary account, a throwaway folder: a file copied into
  Music is uploaded as a track; `mv` to rename it is undone; `mv` into `Family/Kornel/…` shares
  it (same identifier, "shared by me") and back unshares it; `rm` trashes it and it does not
  come back; a folder of two chapters and a `cover.jpg` under Audiobooks becomes one book
  with the image kept as a companion file; `rm -r` of the book trashes it. Core: 46 tests,
  incl. Finder's Trash (trash, pull keeps it, Put Back by batch, Empty Trash, restored
  elsewhere).
- Checked by the user in Finder, 2026-09-26: the alerts (in Czech), Move to Trash, Put
  Back, Empty Trash, copying books and albums in (several at once, 30–80 MB files), the
  family side, keep downloaded, playback. Found on the way and fixed:
  - **One sync engine per extension.** Opening it awaits and the system calls in many at
    once; each caller opened its own engine over the same database ("database is locked"),
    each with part of a book folder's batches — one folder became several books ("Black
    Swan (2)"…). The book is now also recorded right after it is created, a failed
    companion no longer fails the book, and a companion-only batch is retryable.
  - **Uploads of large files failed after a minute** (one HTTP client with a 60 s read
    timeout, four at once). Storage uploads have their own client, retry a dropped
    connection, and run two at a time.
  - **Move to Trash hung** on files never uploaded: the system asks to create them in the
    Trash. They are now excluded from sync and stay in this Mac's Trash only.
  - **Extensions were hidden** on files not downloaded, and **private folders said "Shared by
    Me"**; items now state their file-system flags and who shares a folder. Such changes
    reach existing items through an item generation carried in the sync anchor — the system
    lists everything once, without a reimport.
  - **Root folders re-created by the system were refused** and listed as "could not be
    added"; an existing folder is now merged.
  - Finder's warning on the location (storage full, sign-in, unreachable) is cleared once
    the problem is over (`signalErrorResolved`).

### ✅ S8 — Mac: app

- [x] Playback providers; badges; context menus — *a file Finder has downloaded (not
      dataless) plays from disk via `getUserVisibleURL`, else it streams; Show in Finder,
      Move to Trash and Add to / Remove from own.audio Folder in the library menus*
- [x] Sync status + popover; Trash screen — *status line + popover; notifications for held
      deletes and for uploads waiting on credit (a click opens the popover); Trash screen
      (grouped by deletion, Restore / Restore All / Delete Forever / Empty, days left,
      restore charge, family scope for admins); keep-downloaded via Download / Remove Download*
- [x] Add/import via core; Navidrome/ABS; Local Folders — *Add Book, Add Music, Navidrome/ABS
      import, Local Folders upload and Local Only promotion go through `core.import` in the
      extension (`FinderControl.importFiles`, files staged in the App Group container), with the
      source's metadata, cover and order (Both keeps it downloaded in
      Finder); Navidrome search; Download to this Mac, Audiobookshelf episodes included, saves
      into the folder on this Mac. **Local Folders** (§7.6): a sidebar item with that folder plus
      any the user adds, indexed and watched (FSEvents), played from, Upload to own.audio as a
      book or as music*
- [x] Podcasts: Save to own.audio, per-show auto-store switch — *show detail's Save New
      Episodes switch (`PUT /podcasts/{id}/auto-store`) replaces the device rules on the Mac;
      an episode's Download stores it on the server and keeps it downloaded in Finder; episode
      search and Save Archive… (`POST /podcasts/{id}/store-all`)*
- [x] Family shortcuts: add/remove, markers, list on the Family screen — *member + kind
      switches and the list with Remove in Settings → Sync → Family in Finder; a single book,
      album or show from its menu (the server finds the owner); a folder marker on a family
      member's book or show in the folder (the extension reports the family items it holds)*
- [x] Sync settings pane; Czech strings — *Settings → Sync done (location, userEnabled note,
      Show in Finder, Sync Now, Pause via disconnect/reconnect, folder on this Mac); core refusals
      translated in the extension in the app's language; Finder alerts done (extension's
      `cs.lproj/Localizable.strings`, keyed by the English text)*
- [x] **Organise** (§2 item 18) — *`POST /sync/paths/organise` for music and audiobooks,
      preview in a rolled-back transaction, companions move along, multi-disc albums into
      `CD <n>`; the app's sheet lists the moves by album or author to confirm all or some.
      Finder moves the files itself (same identifiers)*

### ✅ S9 — Mac: retire and migrate

- [x] Old-folder migration sheet — *once, for the folder on this Mac: files the mirror left
      that are copies of items still in own.audio (its `<item id>/<file id>` store, or paths
      its download records name) go to the Trash on confirmation; the rest stays as a Local
      Folder. Any Local Folder can be checked the same way from its menu*
- [x] Remove §7.8 from the Mac; shared-package cleanup after the iOS grep — *the mirror, the
      download manager, Downloads, Transfers, Settings → Downloads and Mirror, the podcast
      rules, the folder scan and the rename prompt are gone (about 2,300 lines in the app).
      From the packages: the mirror engine, planner, resumption, reconciliation, tombstones,
      upload spool and the Mac's download screens. Kept on purpose: `DownloadManager`,
      `RangeTransfer`, eviction and `DownloadSettingsStore` (iOS uses them); the mirror's
      SwiftData models (dropping them from the shared schema needs a migration on every
      client); `LibraryFolderStore` (the folder on this Mac) and `MediaStorageChoice`
      (Navidrome/ABS import offers Cloud or Both)*
- [x] `audio2-mac/CLAUDE.md` rule change; old plans point here
- [x] Full pass: light/dark, VoiceOver labels, no lock icons, Czech — *new screens use system
      colours; icon-only buttons carry labels; no lock symbols anywhere; the app catalog is
      fully Czech (a duplicate key removed, Siri strings translated)*
