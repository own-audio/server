# Metadata organizer — plan

A screen in the web console for looking through the music library's metadata and fixing it,
in the spirit of MusicBrainz Picard: filter the songs, see their tags next to what
MusicBrainz says, look things up, change them one by one or many at once, and undo any of
it. Unlike Picard, **nothing is ever written to a file**: the metadata lives in the database,
as identify already does.

Decided (2026-10-14):

- **Open-source core and hosted alike.** The hosted edition's edge is its own MusicBrainz
  mirror (no one-request-a-second limit), not a locked feature.
- **Every member** edits their own songs; an album is the household's, so any member who sees
  it edits it (the rule `v2/edit.rs` already has); merges and splits stay a family admin's.
- **Every change is kept** and can be undone — see O0, which does not exist yet.
- In the console under **Create**, next to Translate an episode.

## What exists to build on

- Identify runs, proposals, editions, search in the owner's words, single-track identify and
  undo (`v2/identify.rs`, revisions 10–14).
- Corrections on entities (`v2/edit.rs`): credits, contributor rename/merge/split, album
  details, release group, album merge.
- `music_track_identity.before`: the state before the *first* identify, nothing else. Plain
  edits (`PATCH` a track's title, v1 `update_track`) keep no history today.

## Phases

### O0 — Edit history (core, prerequisite)

- `metadata_edits`: one row per change of one item — `item_kind` (track, album, contributor),
  `item_id`, `user_id`, `at`, `source` (`manual`, `identify`, `run`, `bulk`), `batch_id`,
  `before` and `after` (only the fields that changed, JSONB).
- Every write path goes through it: v1/v2 track edits, identify (single and runs), album and
  contributor corrections. `music_track_identity.before` stays for revert-to-file.
- Undo one edit, or a whole batch, when the fields still hold what the edit wrote (otherwise
  `409 changed_since`, listing the fields).
- Kept indefinitely; a track deleted takes its history with it.
- `GET /api/v2/edits?item=…|batch=…|user=…` (cursor), `POST /api/v2/edits/{id}/undo`,
  `POST /api/v2/edits/batches/{id}/undo`.

### O1 — Organizer API (core)

- `GET /api/v2/organize/tracks`: the caller's tracks with every editable field, the file's
  path and format, identity source, cover yes/no; filters — text, folder, artist, album,
  `unidentified`, `no_cover`, `no_year`, `no_genre`, `duplicates`, `changed_since`; sort by
  any column; cursor pages sized for a table (≤ 500).
- `PATCH /api/v2/tracks` (bulk): the same fields for many tracks in one batch, one edit
  history batch; a field may be set, cleared or left alone.
- Facets for the filter tree: folders, artists, albums with counts.

### O2 — The screen (web console)

Three panes, Picard's layout:

- **Left — library**: folders / artists / albums tree with counts, plus the smart filters
  (unidentified, no cover, no year, recently changed).
- **Middle — songs**: a table, multi-select, editable cells (title, artist, album, album
  artist, track/disc, year, genre), a bulk-edit dialog for the selection, unsaved changes
  marked until saved as one batch.
- **Right — MusicBrainz**: search in your own words for the selection (a song → recordings, an
  album's songs → releases with their editions), the release's track list matched to the
  selected files with a tag-by-tag comparison (same / changed / missing, like Picard's
  colours), apply to all or some; cover from the Cover Art Archive.
- **History** tab: the batches, newest first, each with Undo.

### O3 — Albums as releases

Picard's clustering: select a folder or an album's files, pick a release, and see which file
goes on which track (by number, title and length — `match_tracks` already does it), drag to
fix the pairs, songs the release lacks and tracks no file covers listed. Applies through the
same proposals path as runs.

### O4 — Covers and the rest

Replace or fetch an album's cover (CAA, or an uploaded image); genres from MusicBrainz tags;
an artist's MusicBrainz identity on the contributor (name, sort name, merge
"Beatles"/"The Beatles").

## Testing

Conformance: history written by every write path and undone exactly; bulk patch one batch;
`changed_since` refusal; filters and facets on the fixture library; permissions (member own
tracks, album any member, merge admin). Browser tests for the screen.

## Open questions

- Show the file's own tags beside the database's (needs reading them on demand, as
  `get_track_file_tags` does in v1)?
- A "write tags to files" export, ever? (Today's answer: no — files are never changed.)
- Mobile: none planned; the Mac app could mirror the screen later.
