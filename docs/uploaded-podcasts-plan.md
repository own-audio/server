# Uploaded podcasts — implementation plan

A podcast whose episodes its owner uploads, instead of a feed the server fetches: lecture
series, a family's own recordings, a show bought as files, anything a person has as audio
and wants to hear as episodes, with progress, sharing and sync like every other show. Added
through API v2 with the same upload flow as tracks and books (`POST /uploads`, `PUT` the
file, then create the item).

Written 2026-10-18. Legend: ✅ done · 🚧 in progress · ⬜ not started.

Where the files come from is the uploader's business. The server never fetches anything for
these shows; it only stores what it is given (see `SCOPE.md` decision 15 for YouTube).

---

## 1. Facts this builds on

- `POST /api/v2/uploads` presigns one `PUT` per file for a `kind`
  (`uploads::presign`, folder per kind); `POST /tracks` and `POST /books` turn uploads into
  items (`v2/uploads.rs`). Size is declared up front, quota checked before a byte is sent;
  creates honour `Idempotency-Key`.
- Podcasts are `podcast_feeds` (`feed_url NOT NULL`, unique per owner), episodes
  `podcast_episodes` (unique `(feed_id, guid)`), stored audio is `audio_object_id`, family
  sharing via `family_id`, visibility rule `audio2_can_access`.
- `podcast_episodes.transcript_url`/`transcript_type` exist (migration 0050); `has_transcript`
  gates translation (hosted) and, later, summaries (hosted).
- Refresh, auto-store and gPodder all work on `source_type = 'rss'` feeds.

## 2. Decisions

| # | Decision | Value |
|---|---|---|
| 1 | Model | A feed with `source_type = 'upload'` and `feed_url = 'upload:<feed id>'` (unique, never fetched). No new table for shows. |
| 2 | Episodes | Rows in `podcast_episodes` with `audio_object_id` set at creation; `guid` given by the uploader (idempotent re-upload: same guid answers the existing episode), `audio_url` null. |
| 3 | Transcript | New column `transcript_object_id` (migration 0116); `transcript_type` set from the upload (`text/vtt`, `application/srt`, `text/plain`). `has_transcript` is true for either source. v2 `GET /episodes/{id}` adds a signed `transcript` link. |
| 4 | Never fetched | Refresh is a no-op for upload feeds (excluded from the scheduler like YouTube leftovers); auto-store and store-all answer 409; gPodder does not list them as subscriptions (they have no URL another app could fetch). |
| 5 | Permissions | Creating a show or an episode needs `can_upload`; adding to a shared show: its owner, or a family admin. Deleting an episode trashes it (30 days) like other items. |
| 6 | Artwork | Show cover and per-episode image from uploads (`podcast_cover`, `episode_image`), stored like track covers. |
| 7 | Contract | v2 only (v2 is still `beta`, `V2_REVISION` bumped). v1 lists the shows and episodes as it lists any feed (`source_type: "upload"`, a new value v1 clients already tolerate per `API_COMPATIBILITY.md` §4). |

## 3. API (v2)

```
POST /api/v2/uploads            kind: episode | episode_transcript | podcast_cover | episode_image
POST /api/v2/podcasts           {title, author?, description?, language?, visibility?, cover_upload_id?}
                                → 201 Podcast (source_type "upload")
POST /api/v2/podcasts/{id}/episodes
                                {upload_id, guid, title, description?, published_at?,
                                 duration_ms?, episode_number?, image_upload_id?,
                                 transcript_upload_id?, transcript_type?}
                                → 201 Episode; same guid again → 200 the existing episode
PATCH /api/v2/podcasts/{id}     title, author, description, language, cover_upload_id
PATCH /api/v2/episodes/{id}     title, description, published_at, transcript_upload_id
DELETE /api/v2/episodes/{id}    → trash
```

## 4. Phases

### U1 — Model and migration ⬜
Migration 0116: `transcript_object_id`, a check that `source_type IN ('rss','youtube','upload')`,
scheduler exclusion, refresh no-op, 409s, gPodder filter. Upload kinds in `uploads::presign`.

### U2 — Create show and episodes ⬜
`POST /podcasts`, `POST /podcasts/{id}/episodes` with idempotent guid, duration from the
client or the `duration` job, `ensure_episode` path in the own.audio folder, "new episodes"
notification to the family.

### U3 — Transcripts and edits ⬜
Transcript link in v2 episodes, `has_transcript` for uploads, `PATCH` routes, delete → trash.

### U4 — Console ⬜
"New podcast" next to "Add a feed"; upload episodes by drag and drop (title from the file
name, guid = file hash); edit; a badge "Uploaded" instead of a refresh button.

### U5 — Conformance and docs ⬜
Conformance suite `uploaded_podcasts` (create, upload, idempotent guid, transcript link, 409s,
family visibility); client guide and API spec.

**Verify (whole plan):** a script creates a show, uploads three M4A episodes with VTT
transcripts, shares it with the family; another family member sees it in own.audio
Podcasts and the Mac app, plays with progress sync, `has_transcript` is true and the
transcript opens; re-running the script changes nothing.
