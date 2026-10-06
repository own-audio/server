# Music Metadata Sources

Date: 2026-08-21

Scope: which external services audio2 uses to identify music tracks, and which
were evaluated and rejected. Backend-only — the Mac client's identify UI is
described in `audio2-mac/MUSIC_IMPLEMENTATION_PLAN.md` P7.

## In use: a self-hosted MusicBrainz mirror

`backend/src/metadata/mirror.rs` calls the `music-metadata` service, behind
`POST /music/tracks/{id}/metadata/search` and `.../metadata/apply`.
**The public MusicBrainz API is no longer called** (swapped 2026-08-21) — see
`music-metadata-plan.md` for the mirror itself.

- **Recording-oriented.** One search per track, matched on title/artist/album,
  now also passing the track's own **duration** — evidence the public API had no
  way to use, and the thing that separates a studio recording from the many live
  and compilation versions of a popular track filed under the same name.
- **Supplies** title, artist, album, track number, and three stable ids stored
  on the track: `musicbrainz_recording_id`, `musicbrainz_release_id`,
  `musicbrainz_artist_id`. Same MBIDs as before — the swap is a transport
  change, so every row already written stays valid.
- **Cover art** comes from the cascade in `metadata/cover_art.rs`
  (Cover Art Archive → iTunes → Deezer), still over HTTP to third parties.
- **No rate limiter.** The semaphore, the 503 backoff and the Lucene query
  builder are gone with the API they existed for. Measured on the dev stack:
  **8 sequential lookups in 3.2s (~0.4s each)**, against a floor of 8.8s on the
  public API's 1 req/s limit.
- **No fallback to the public API**, deliberately. Falling back would restore
  the 1 req/s limiter silently, under exactly the load that makes it hurt, and
  present as an unexplained slowdown rather than a clear failure. An unset or
  unreachable mirror is an error.

### Configuration

`METADATA__BASE_URL` and `METADATA__API_KEY` on the backend (`METADATA_SERVICE_URL`
and `METADATA_API_KEY` in `.env`). Unset means identification returns an error.

### Known limits

- **Genre is often absent.** It comes from community tags, and plenty of
  recordings have none — Queen's "Somebody to Love" returns `tags: []`.
- **The top match is not always the obvious one.** Searching *A Day at the
  Races* returns "Tie Your Mother Down (1991 remix by Matt Wallace)" at
  **score 100** for the plain album track. Any batch flow must show proposals
  for review rather than applying the top match.
- ~~**One request per track.**~~ **Resolved** by the mirror — this was the whole
  reason for it. Still one request per track, but at ~0.4s rather than ~1s of
  enforced delay, and no longer serialized process-wide behind one semaphore.
- **Genre is now always absent**, not merely often: the mirror is imported
  without `mbdump-derived`, which is where community tags live. A regression
  against the public API in this one field, accepted knowingly — see the plan's
  §4.2.

## Artist images: Wikimedia Commons, via Wikidata

`backend/src/metadata/wikimedia.rs`, behind `GET /music/artists/image`.

`musicbrainz_artist_id` → Wikidata `P434` → `P18` → a Commons file, falling back
to an exact English label match **restricted to entities carrying a MusicBrainz
artist id** when no track of that artist has been identified. That constraint is
what stops it behaving like a search engine: "Unknown Artist" matches nothing,
confirmed live.

Commons resizes server-side (`iiurlwidth`), so this asks for 640px rather than
downloading multi-megabyte originals.

**Attribution is stored with the image and displayed**, because most Commons
licences require it. `music_artist_images` carries author, licence, licence URL
and the file's Commons page (migration `0034`); the Mac shows a credit line under
the artist header. An image whose attribution was discarded could not lawfully
be shown, which is why the two are written together.

## Rejected on licensing — and one removal

Reviewed 2026-08-21 against each provider's actual terms, not their availability.
An earlier draft of the plan recorded only "Auth? none" for iTunes and Deezer,
which answered whether we *could* call them, not whether we may **cache and
redistribute** what they return. This backend does exactly that: every image is
written into Garage and served from our own endpoint, for a paid product.

| Source | Verdict | Why |
|---|---|---|
| **Deezer** | ❌ removed | Terms limit use to "a non-commercial purpose and in a non-commercial environment", state images "are not allowed to be stored", and forbid reproduction without written authorisation. All three apply. |
| **iTunes Search API** | ❌ removed | Artwork may be used only to promote items in the iTunes Store, adjacent to a store link. A personal library is not that. |
| **fanart.tv** | ❌ not adopted | "Do not use our API for commercial use without written consent." Askable, not assumed. |
| **Cover Art Archive** | ⚠️ kept | Grants no licence: images stay their owners' copyright, "use at your own risk". Kept deliberately — it is the archive the MBIDs we already store point at, and where comparable applications source album art. A different position from a source whose terms name this use and prohibit it. |
| **Wikimedia Commons** | ✅ adopted | Free licences (CC BY, CC BY-SA, public domain); commercial use permitted. Cost is mandatory per-file attribution, which is why it is stored and shown. |

**Consequence, stated plainly:** removing iTunes and Deezer from the album-art
cascade leaves only Cover Art Archive, so some albums that previously got a cover
no longer will. That is a real regression in coverage, accepted knowingly in
exchange for not shipping a use two providers explicitly prohibit.

## Evaluated and rejected: OneMusicAPI

<http://www.onemusicapi.com> — an aggregator over Discogs, MusicBrainz and
others. Docs:
<http://www.onemusicapi.com/docs/20220401/tutorials/getting-started.html>

Evaluated 2026-08-21 against a live key. **Not integrated.** No code references
it; the key is parked in `.env` as `ONEMUSIC__API_KEY` purely so it doesn't have
to be found again, with a comment saying it is unused.

### What it does well

**Release-oriented, so one call returns a whole album.** For the batch identify
flow this is structurally better than MusicBrainz's per-track search — one
request instead of N, and no rate-limit pacing to work around:

```
GET http://api.onemusicapi.com/20220401/release
      ?title=A%20Day%20at%20the%20Races&artist=Queen
      &inc=images&maxResultCount=1&user_key=<key>
```

```json
[{"title":"A Day At The Races","artist":"Queen","year":"2011","country":"XE",
  "genres":["Rock","Classic Rock"],
  "media":[{"totalDiscs":"1","position":"1","format":"File",
            "tracks":[{"title":"Tie Your Mother Down","number":"1","duration":287000}]}],
  "types":["Album","FLAC","Deluxe Edition"],"score":1.0}]
```

`inc=images` adds cover URLs with dimensions and a quality score:

```json
"images":[{"url":"http://api.onemusicapi.com/20220401/images/discogs/17394586/1613223743-4082",
           "width":599,"height":592,"score":5}]
```

It also carries **genres where MusicBrainz has none** — "Rock, Classic Rock"
for the Queen release above, versus MusicBrainz's empty tag list.

### Why it was rejected

1. **No stable identifiers, of any kind.** The response's entire key set is
   `artist, country, genres, images, media, score, title, types, year`. There
   is nothing to store. This is disqualifying rather than inconvenient: the
   `musicbrainz_*_id` columns are what make "this track has been identified" a
   fact rather than a guess, what lets a re-identify be idempotent, and what a
   future correction could be traced back through. A source that can only ever
   hand back free text cannot anchor that.

2. **TLS does not validate.** `https://api.onemusicapi.com` connects on 443 but
   the certificate chain fails verification (`unable to get local issuer
   certificate`); only plaintext `http://` works. The key travels in the query
   string, so integrating as-is would put a credential in cleartext on the wire.

3. **Aggregated data is not automatically better data.** The *A Day at the
   Races* lookup above matched a 2011 deluxe edition — 15 tracks including live
   and mono versions — and rendered "Long Away" as "Long Way". It would still
   need the same review-before-apply step MusicBrainz does, so it does not
   simplify the flow it would most have helped.

### If this is revisited

The honest shape would be a **supplement, not a source of identity**:
MusicBrainz continues to establish what a track *is* and supplies the stored
ids; OneMusicAPI fills only the fields it demonstrably does better — genre, and
cover art when Cover Art Archive 404s. That keeps the data model unchanged.

Resolve the TLS problem first. Preferred alternative, per the project owner:
**self-host MusicBrainz**, which removes the rate limit — the actual pain point
— without introducing a second source at all.

That is the chosen route, planned in `music-metadata-plan.md`. It also supplies
the other half of what OneMusicAPI was being considered for: cover art beyond
Cover Art Archive now comes from a cascade of unauthenticated sources
(iTunes, Deezer) cached into Garage, rather than from a paid aggregator. The
cascade is built behind a `CoverArtProvider` trait, so OneMusicAPI remains
addable later as one supplementary provider — the shape described above — with
no change to the data model.
