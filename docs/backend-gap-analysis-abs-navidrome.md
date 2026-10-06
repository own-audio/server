# Backend Gap Analysis: audio2 vs Audiobookshelf & Navidrome

Date: 2026-07-18

Scope: backend-only comparison. Frontend/UI parity is out of scope for this
document. Goal is to track what Audiobookshelf (ABS) and Navidrome do that
audio2 doesn't yet, so gaps can be turned into planned work later.

## What audio2 currently has (backend)

- **Podcasts**: subscribe/search, RSS-based feed refresh (single background
  job type: `feed_refresh`), episode download, streaming, per-feed/episode
  images.
- **Audiobooks**: upload-based (no folder scanning), multi-file books,
  authors w/ roles, tags, series, collections, favorites, chapters (listing
  only), progress, bookmarks.
- **Music**: upload-based tracks, manual playlists (CRUD + reorder), cover
  art, progress tracking.
- **Subsonic API**: a real but partial OpenSubsonic surface — `ping`,
  browsing (`getArtists`/`getArtist`/`getAlbumList2`/`getAlbum`/`getSong`/
  `search3`), `stream`/`download`/`getCoverArt`, `scrobble` (internal-only,
  doesn't relay anywhere), playlist CRUD. Token+salt auth is properly
  implemented against a per-user Subsonic API key.
- **Storage**: S3-only (Garage), presigned URLs, no on-the-fly transcoding.
- **Jobs**: only `feed_refresh` actually runs, despite the module comment
  ("feed polling, metadata fetch, thumbnail processing, cleanup, imports")
  claiming a broader scope.
- **Auth**: local login is real; Google/Microsoft OIDC are stubs (literal
  `"TODO"` handlers in `backend/src/auth/mod.rs`).

## Gaps vs Audiobookshelf (audiobook/podcast side)

| Missing | Why it matters |
|---|---|
| Filesystem library scanning / folder watch | ABS auto-discovers books from a folder structure; audio2 is upload-only, no scanner/watcher job. |
| Metadata provider matching (Audible, OpenLibrary, iTunes, "quick match"/"match all") | ABS auto-fills title/author/series/cover from external providers; audio2 requires manual entry. |
| Chapter extraction from embedded tags, and chapter editing | audio2 only *lists* chapters; no parsing embedded M4B chapter atoms or letting users edit/merge them. |
| E-book support (epub/pdf/mobi serving, e-reader progress) | ABS is a combined audiobook+ebook server; audio2 has none. |
| On-the-fly transcoding / HLS for audiobooks | ABS transcodes non-native formats or segments for streaming; audio2 always streams the original file via presigned URL. |
| Podcast auto-download of new episodes | ABS can auto-download on feed refresh; audio2's `feed_refresh` only updates metadata, no auto-download job. |
| ~~Per-library user permissions~~ | **Closed 2026-07-18.** Families now provide private-vs-shared folders plus per-member, per-media-kind policies and item-level allow/deny grants — see the family implementation plan. |
| Listening stats / session history (time listened per day, device) | Not present — audio2 only tracks latest progress, not historical sessions. |
| DB backup/restore scheduling | Not present. |
| Real OIDC login | Google/Microsoft handlers are literal TODO stubs, not implemented. |
| ~~Unified search missing music~~ | **Closed 2026-07-18** — `/library/search` now covers tracks too. |
| Webhook/notification integrations (e.g. on new episode) | Not present. |

## Gaps vs Navidrome (music side)

| Missing | Why it matters |
|---|---|
| Filesystem scanner | Navidrome indexes a music folder tree with tag-based (ID3) ingestion; audio2 requires manual upload per track. |
| Real scrobbling (Last.fm / ListenBrainz) | audio2's Subsonic `scrobble` only updates local progress, doesn't forward anywhere. |
| Star/rating (`star`, `unstar`, `setRating`, `getStarred2`) | Not in the Subsonic router or REST API — no favorites/ratings for music (audiobooks *do* have favorites, music doesn't). |
| Genre & discovery endpoints (`getGenres`, `getSongsByGenre`, `getRandomSongs`, `getTopSongs`, `getSimilarSongs`, `getNowPlaying`) | None implemented — browsing is limited to artists/albums/songs/search. |
| Smart/dynamic playlists | audio2 playlists are static manual lists only. |
| Play queue sync (`savePlayQueue`/`getPlayQueue`) | Not implemented — no cross-device resume-queue. |
| Transcoding profiles / bitrate limiting | audio2 always streams original files; no per-user/per-client transcode settings. |
| ReplayGain | No volume-normalization metadata read/applied. |
| Artist/album image + bio fetching (Last.fm/Spotify-backed) | audio2 fetches MusicBrainz/Cover Art Archive cover art on-demand via `POST /music/tracks/{id}/metadata/apply` (2026-08-20) — a per-track match-and-apply, not automatic bio/similar-artist enrichment, which remains unimplemented. See `music-metadata-sources.md` for which external sources are used and which were evaluated and rejected. |
| Internet radio stations | Not present. `getInternetRadioStations` returns an empty list rather than an error, so a client's radio tab stays quiet instead of looking broken. Deliberately deferred: it is the one media type where audio2 would store nothing and the client would connect straight to a third party. |
| Multi-folder libraries / library management endpoints | audio2 has one storage bucket per install, no multiple named libraries. |
| `getScanStatus`/`startScan` Subsonic endpoints | Answered honestly rather than refused: there is no scanner, so `scanning` is always false and the count is the library's real track count. |
| ~~Podcast support inside the Subsonic API itself~~ | **Closed (2026-08-22).** `/rest` now serves `getPodcasts`, `getNewestPodcasts`, `refreshPodcasts`, `createPodcastChannel`, `deletePodcastChannel`, `downloadPodcastEpisode` and `deletePodcastEpisode`, over the same feeds and episodes as `/api/v1/podcasts`. `status` is `completed` only for episodes whose audio is stored. Streaming follows one rule: **serve it if own.audio has it, redirect to the origin if not.** Proxying the origin through the server was considered and declined (2026-08-22) — it would hide the listener's IP from the podcast host, but costs ~56MB each way per play on this library's average episode and requires `Range` forwarding to keep seeking working. Downloading an episode is the answer for a copy that never leaves own.audio. |
| Shares (`getShares`, `createShare`) | Not implemented. |

## Biggest structural gap common to both

audio2 is fundamentally an **upload/subscribe model**, not a **scan-a-folder
model**. Both ABS and Navidrome are built around watching a filesystem tree
and deriving the entire catalog (metadata, structure, covers) from what's on
disk. That single difference explains most of the missing surface: no
scanner job, no embedded-tag extraction, no "match against external
metadata provider," no re-scan/incremental-index endpoints. Everything else
(ratings, scrobbling, transcoding, smart playlists) is comparatively small,
additive work on top of what's already a cleanly separated backend (auth,
jobs, storage, and per-domain routers are all split by module).

## Suggested next step

Pick one thread to turn into an implementation plan, e.g.:
- Real Last.fm/ListenBrainz scrobbling relay.
- Star/rating support in the Subsonic API + REST API for music.
- A filesystem scanner job as an alternative/companion ingestion path to
  upload.
