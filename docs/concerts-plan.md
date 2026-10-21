# Concerts — implementation plan

A whole concert as one track with a chapter mark where each song starts, shown in Music
under its own **Concerts** section next to Albums and Playlists. One file keeps the show as
it was played (no gaps between songs, the talk and the crowd in between), and the chapters
let a listener skip to a song. Any live recording with chapter marks works: a bootleg, a
radio broadcast, a recording a family member made, a concert bought as one file.

Written 2026-10-21. Legend: ✅ done · 🚧 in progress · ⬜ not started.

Where the files come from is the uploader's business; the server only reads what it is
given (`SCOPE.md` decision 15).

---

## 1. Facts this builds on

- Every album gets a release group when its first track is indexed (trigger in migration
  0100); `release_groups.kind` exists (`album` by default) and v2 already returns it
  (`ReleaseGroup.kind`: `album`, `single`, `live`…). Nothing sets it today.
- Chapter marks inside M4B, M4A and MP3 files are read with `ffprobe -show_chapters`
  (`library_folders::embedded_chapters`, beta.34), so far only for folder books.
- `POST /api/v2/tracks` downloads the upload to a temp file to read its tags
  (`music::create_track_from_upload`), so the file is on disk when chapters can be read.
- Music already keeps a resume position per track (`music_progress`,
  `PUT /api/v1/music/tracks/{id}/progress`). The web console saves it but never reads it
  back; the iOS music app saves a local position only.
- The web console reads music through v1 (`/api/v1/music/*`); the iOS music app through v2.
- The player's chapter list in the web console is really the audiobook's list of files
  (`ChaptersPopover`); nothing seeks to a chapter by time yet.

## 2. Decisions

| # | Decision | Value |
|---|---|---|
| 1 | What a concert is | An album whose release group has `kind = 'live'`. No new table for concerts; credits, covers, favourites, playlists and the artist page work as for any album. |
| 2 | The file | One track per concert, AAC in an `.m4a` with chapter marks. Same bytes as an `.m4b`, but `.m4a` keeps other players (Finder folder, Plex, Navidrome) treating it as music, not as an audiobook. |
| 3 | Chapters' source | The file, until someone edits them. The server reads the marks when the track is created and when a library folder is scanned (two or more, as for books). Chapters edited by hand stay: the file is not read for that track again, and the file itself is not rewritten. |
| 4 | Marking a concert | `POST /api/v2/tracks` takes `release_kind` (`live`), which sets the kind of the new track's release group. `PATCH /api/v2/release-groups/{id}` with `{kind}` marks an existing album (a live album already in the library). |
| 5 | Resume | A concert opens where you stopped, on any device: the position is the server's (`PUT`/`GET /api/v2/me/progress/track/{id}`, already there for every track), saved on pause, on leaving and every 15 s while playing, and read before playing. Starting a two-hour show from the top is the bug. Other music keeps today's behaviour. |
| 5a | Editing | The track's owner edits the songs (titles, start times, add, remove) in the web console and the iOS app; one `PUT` with the whole list. Same rule as a track's credits. |
| 6 | Previous / next | On a track with chapters, previous/next (player buttons, lock screen, keyboard) move between chapters; past the last chapter, next goes to the next track in the queue. Previous within the first 3 s of a chapter goes to the one before, as for tracks. |
| 7 | Subsonic, Jellyfin, gPodder | A concert is one long track there; they have no chapters for music. Nothing changes in those APIs. |
| 8 | Contract | Additive. Discovered as `features.track_chapters` in `GET /api/v1/server` (`API_REVISION` +1); v2 changes bump `V2_REVISION`. |

## 3. API

```
v2
POST  /api/v2/tracks                 + release_kind?: "live"     (any release-group kind)
GET   /api/v2/tracks/{id}            + chapters: [{position, title, start_ms}]   (only here)
PUT   /api/v2/tracks/{id}/chapters   {chapters: [{title, start_ms}]}  owner; [] removes
      Track (every listing)          + chapter_count: int
GET   /api/v2/albums                 + kind=live                 (filter by release-group kind)
PATCH /api/v2/release-groups/{id}    {kind}

v1 (what the web console reads)
GET   /api/v1/music/albums           + kind=live;  AlbumSummary + release_kind?
GET   /api/v1/music/tracks/{id}/chapters  → [{position, title, start_time_secs}]
      MusicTrack                     + chapter_count?
```

## 4. Phases

### C1 — Server ✅ (2026-10-21, uncommitted)
Verified on a compose stack built from the working tree: 303 conformance checks pass
(v2, library, smoke, entities, families), 13 of them new for concerts. A real 1:37:45 concert
uploaded through v2 came back with its 27 chapters, its cover, its length and under
`albums?kind=live`. Found on the way and fixed: v2 uploads had no duration.

- Migration 0118: `music_track_chapters (track_id, position, title, start_ms)`, unique
  `(track_id, position)`, cascade on track delete; `music_tracks.chapters_read_at`,
  `chapters_edited_at`.
- Move `embedded_chapters` out of `library_folders` into a shared module; books and tracks
  use the one reader.
- Read chapters in `create_track_from_upload` (temp file already there) and in the folder
  scanner for music files (pending ones only, like books' `chapters_read_at`).
- `release_kind` on create; `PATCH /release-groups/{id}`. Release groups are not in the
  organizer's undo history (0114), so marking one is undone by marking it back.
- The endpoints and fields in §3, `features.track_chapters`, revisions, OpenAPI regenerated,
  `CHANGELOG.md`, `API_COMPATIBILITY.md` revision table.
- Tests: unit tests for the reader on a generated fixture (ffmpeg writes a 3-chapter m4a);
  conformance: upload a chaptered m4a with `release_kind: live`, see it under
  `albums?kind=live`, read its chapters through v1 and v2, mark and unmark an album.

### C2 — Web console ⬜
- **Concerts** chip in `MusicPage` (`music.mode.concerts`, cs "Koncerty"): albums with
  `kind=live`, the album view on select. i18n in `i18n/strings/music.json`.
- Player: chapters generalised to `{title, startSecs}` so books and tracks share them;
  `ChaptersPopover` lists a music track's chapters and seeks to one; the current chapter's
  title in the player bar and in Media Session metadata; previous/next per decision 6.
- Resume per decision 5: read the saved position before playing a track with chapters, and
  show it on the concert ("Continue at 47:12 – Numb").
- "Mark as concert" on an album's menu (the PATCH), behind `FeatureGate`.
- Edit the songs: on a concert's page, a list of rows (time, title) with add/remove, saved
  with one `PUT` (v2); "now" fills the time from the playing position, so a song is marked
  while listening. Owner only.

### C3 — iOS music app (`music-ios`) ⬜
Planned in that repo's `IMPLEMENTATION_PLAN.md` when C1 is released; it is a public,
multi-server app, so concerts appear only on own.audio servers that report
`features.track_chapters`. Outline: a Concerts library section, a chapter list in Now
Playing, lock-screen previous/next per decision 6, resume per decision 5, and editing the
songs (rename, move to the current position, add, delete) through the same `PUT`.

### Later
Android, Mac, tvOS and Windows music clients.
