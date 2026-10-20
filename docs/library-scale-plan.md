# Large libraries: what comes next

From the first real NAS test (`docs/SYNOLOGY.md`): 9,351 files, 88 GB. In
order.

## 1. Albums split by partial tags

A track with a MusicBrainz release id is filed under the release; one without
under album artist + title. A folder tagged partly with Picard therefore
becomes two albums with the same name (53 in the test library).

Open decision: should a track without a release id join an existing album
with the same title and album artist, or only one from the same folder? The
first is simple but merges a second, untagged copy of an album into the
tagged one; the second needs the folder known to the linking trigger. Both
need a one-time repair of the albums already split.

## 2. MusicBrainz mirror as a server setting

Only 6% of the test library carried MusicBrainz ids, so the first identify
run looks up about 8,800 tracks. At one request a second against
musicbrainz.org, and about 10 requests per album, that is hours.

- Server settings stored in the database, editable while it runs; the
  environment stays the default.
- The MusicBrainz source: musicbrainz.org, or a mirror's `/ws/2` address with
  its own request rate. "Test connection" checks it answers like MusicBrainz.
- A mirror on the LAN is allowed for that address only; outbound requests to
  private addresses stay blocked otherwise.
- A "Server" section in the console's Settings, for admins.

## 3. Cheaper identify runs

Even without a mirror:

- A release id on some of an album's files means one direct release lookup
  instead of a search (about 10 requests).
- The same album twice in one run (a copy in another folder) is looked up
  once.
- Release details are kept for the run (done for 15 minutes in beta.31).
- Albums of many different artists (unofficial compilations) go straight to
  song-by-song lookups instead of an album search that cannot succeed.
- "Apply" for a whole album in one request.

## 4. A faster scan

The scan waits on one file at a time. Instead:

- Several files at once (`LIBRARY__SCAN_CONCURRENCY`, 4 by default; 1–2 on a
  single USB disk).
- Read each file once: the length and the tags are two reads today.
- One transaction per folder instead of ~20 per file.

## 5. Duplicates

Of 4,054 hashed files, 69 were byte-identical copies, while 239 groups had the
same title, artist and length: other rips, other bitrates, compilations.
Skipping files at scan time would cut holes in compilations and make the
result depend on scan order. Instead:

- Index every file; group copies of one recording (MusicBrainz recording id,
  else artist + title + length within 2 seconds; fingerprints later).
- Play the best copy (FLAC, then the highest bitrate), keep the others as
  alternatives.
- One entry in songs, search, shuffle, radio and smart playlists; albums stay
  complete.
- Stars, ratings and plays belong to the recording.
- The organizer lists the groups, with "not the same" and "hide this copy",
  kept across rescans.

## 6. The console with 10,000 songs

`frontend/e2e/library-scale.mjs` measures it (results in `docs/SYNOLOGY.md`);
run it after each change. What it still shows:

- The music pages download the whole track list (6.6 MB for 9,351 songs) and
  filter it in the browser. An album's or an artist's songs should come from
  the server on their own.
- Long lists render a part at a time and keep what they rendered: after the
  whole song list, 6,400 rows and about 460 MB. Render only the rows on screen.
- Covers count against the API rate limit like any request; the console paces
  them at 10 a second. Cheap cached reads could have their own limit.

## Also seen

- 41% of the tracks have no cover at all; identify can fetch them.
