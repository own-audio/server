# Album artist — an album is a release, not an artist

Written 2026-09-24. `CLAUDE.md` governs everything here.

## 1. The problem

Every album grouping in the backend keys on **track artist + album name**. That
splits a release the moment one track has a different artist credit:

- **A guest on one track.** George Ezra's *Staying at Tamara's* on canary shows
  as two albums: ten tracks by "George Ezra", and "Saviour" by "George Ezra
  feat. First Aid Kit" on its own. Identifying the album could not find Saviour
  either, because it was never on that album's screen to be sent.
- **A compilation.** "Hits of the 80s and 90s" with twenty artists becomes
  twenty one-track albums.

Music players that get this right (Apple Music, Navidrome, Plex) all do it the
same way: an **album artist**, separate from each track's artist. The album is
filed under the album artist ("George Ezra", "Various Artists"); each track
keeps its own credit ("George Ezra feat. First Aid Kit").

## 2. The rules

### 2.1 What the album artist is

Decided per track, first match wins:

1. **MusicBrainz release** — once a track is identified, the release's artist
   credit. Tracks on the same release are one album whatever their own credits.
2. **File tags** — the album-artist tag (ID3 `TPE2`, MP4 `aART`, Vorbis
   `ALBUMARTIST`). The compilation flag (`TCMP`, `cpil`, `COMPILATION`) without
   an album-artist tag means "Various Artists".
3. **The track artist without its guests** — "George Ezra feat. First Aid Kit"
   → "George Ezra". Only `feat.`, `ft.` and `featuring` are cut. `&` and `and`
   are not: "Simon & Garfunkel" is one act.

A manual edit can set it directly.

### 2.2 What is deliberately not done

- **No grouping by album name alone.** "Greatest Hits" by ABBA and by Queen are
  two albums. An untagged compilation with no album-artist tag and no
  compilation flag therefore stays split until it is identified or its tags are
  fixed. The server keeps no record of which files arrived together, so there is
  no safe "same folder, same upload" rule to fall back on.
- **Track artist is untouched.** Search, the artist image and smart-playlist
  rules keep matching on it.

### 2.3 What clients show

- The album header shows the album artist; each track row its own artist.
- The artist screen lists albums where the artist is the album artist, and
  separately **Appears on** — albums where they only have a track.

## 3. Phases

**Legend:** ✅ done · 🚧 in progress · ⬜ not started

### ✅ A1 — the album artist in the data (`audio2`) — done 2026-09-24

- Migration 0075: `music_tracks.album_artist`, `musicbrainz_release_group_id`,
  `is_compilation`. **No backfill:** `album_artist` holds only explicit values
  and rule 3 is applied at query time (`db::music::ALBUM_ARTIST_SQL`, Rust twin
  `MusicTrack::effective_album_artist`), so existing albums merge immediately
  and a later edit of the track artist carries the album along.
- Upload reads the album-artist tag and the compilation flag (rule 2), else
  rule 3.
- Manual edit takes an optional `album_artist`; when it is absent, a derived
  value follows the track artist and an explicit one is kept.
- `GET /music/albums` and `GET /music/artists` group by album artist; the
  artist list also keeps artists who only appear on another's album
  (`album_count` 0), which grouping by album artist alone had dropped. The
  `artist` field of an album summary becomes the album artist — additive for
  clients, whose album screens keep working.
- `album_artist` added to the track response, `library/changes` and the file
  tags view.
- Guide and API spec updated in the same commit.

Verified locally: a guest credit on one Queen track keeps *A Kind of Magic* at
nine tracks and adds no "feat." artist; set / keep-when-absent / clear through
`PUT`; uploads with `TPE2` and with `TCMP` alone; SQL and Rust agree on the
same ten names (unit tests `music::models::tests`).

After A1 the album list already shows one *Staying at Tamara's*. Today's apps
still filter an album's tracks by track artist themselves, so they show ten of
the eleven until A3.

### ✅ A2 — the release artist from MusicBrainz (`music-metadata`, `audio2`) — done 2026-09-24

- `GET /v1/recordings/{id}` returns the chosen release's artist credit
  (`album_artist`), `mb_release_group_id` and `disc_number`.
- Apply writes the album artist and release group (rule 1); without them (an
  older service) it leaves the album artist alone.
- Backfill: job `music_release_backfill`, enqueued by an hourly sweep for
  identified tracks with no `release_checked_at` (migration 0076). A reply
  without a release group means the service is older than the field; the track
  stays unmarked and is asked again later, so the deploy order doesn't matter.

Verified locally against the real mirror (the new service run on the Mac,
pointed at the NUC's database): Saviour → album artist "George Ezra"; all 71
identified local tracks backfilled, five hits compilations now one album each
under "Various Artists"; a fresh apply stores album artist and release group.

**Deploy:** `music-metadata` on the NUC, then the backend. Until the service
is updated, the backend keeps rule 3 and the sweep waits.

### 🚧 A3 — clients

**Done 2026-09-24 for `audio2-ios-music` and `audio2-mac`** (shared code in
`audio2-mac`: `AlbumArtist` in `Audio2Core`, `album_artist` on the track, sync
and cache models): album screens, Home's recent albums, offline grouping and
the Mac's whole-album actions go by album artist, with the rule-3 fallback on
the client for tracks cached before the server sent the field; Identify Album
therefore sends the guest track too; the artist screen has **Appears On**.
UI test `LibraryUITests.testArtistOnlyOnACompilationShowsAppearsOn`.
Not yet: the web console, `audio2-android-music`, `audio2-tvos`. Known gap: a
compilation opened from Appears On shows a placeholder cover in its header.

`audio2-ios-music` and `audio2-mac` first, then the web console,
`audio2-android-music` and `audio2-tvos`:

- An album's tracks are the ones whose album artist and album match.
- Album header shows the album artist.
- Identify Album sends every track of the album.
- Artist screen gains **Appears on**.

### ⬜ A4 — identifying compilations (`music-metadata`)

`POST /v1/albums/identify` needs a common artist today and returns nothing for
a compilation. It needs a path that searches by album title and track titles
without an artist.

### ⬜ A5 — the Subsonic surface (`audio2`)

`subsonic/` groups albums and artists by track artist too, and its album ids
are derived from the artist and album strings. Moving it to the album artist
changes the id of every album that splits today. Third-party clients may hold
those ids in stars or playlists, so this is its own step, done deliberately.
