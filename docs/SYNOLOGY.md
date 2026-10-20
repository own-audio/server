# Running the server on a Synology NAS

Tested on 2026-10-20 on a Synology with DSM 7.4.1, an x86-64 CPU with 4 cores,
8 GB of RAM and four disks in RAID 5, next to Audiobookshelf, Navidrome,
Ampache, Jellyfin and a dozen other containers. Server version 1.0.0-beta.30.

## Setup

Container Manager runs the image like any other compose project. What is
specific to a Synology:

- **Folder permissions.** Shared folders often have no "everyone" read
  permission. Run the server as a user and group that can read the music,
  usually your DSM user and the `administrators` group:

  ```yaml
  environment:
    PUID: "1026"   # id -u on the NAS
    PGID: "101"    # administrators
  ```

  The server drops every other group when it starts, so the group in `PGID`
  must be the one the folder's permissions name.

- **An existing PostgreSQL.** If another container already runs PostgreSQL 16
  or newer, give the server its own role and database there instead of a
  second PostgreSQL:

  ```sql
  CREATE ROLE ownaudio LOGIN PASSWORD '…';
  CREATE DATABASE ownaudio OWNER ownaudio;
  ```

  and point `DATABASE_URL` at it, e.g.
  `postgres://ownaudio:…@192.168.1.20:5433/ownaudio`.

- **The music folder, read-only:**

  ```yaml
  environment:
    LIBRARY__MUSIC: /music
  volumes:
    - /volume1/music:/music:ro
    - /volume1/docker/own-audio/media:/data/media
  ```

- **Secrets outside the compose file.** Compose projects on a NAS are often
  readable by every user. Keep `DATABASE_URL` and `AUTH__SESSION_SECRET` in an
  `env_file` only you can read.

- **Docker needs root.** `docker compose` has to be run with `sudo` over SSH,
  and under `sudo` `/usr/local/bin` is not on the `PATH`.

## The first scan

The library: 9,351 audio files, 88 GB (8,895 MP3, 368 FLAC, 84 WMA, 3 WAV,
1 M4A) in 1,038 albums.

| | |
|---|---|
| Time | **1 h 22 min** (21:03–22:25), all files, no errors |
| Speed | 2–253 files a minute, median 121 |
| Server memory | 46 MB while scanning, **68 MB at most** |
| Server CPU | 7 min 14 s of CPU time in 82 minutes |
| Database | 69 MB at the end; every read served from memory (cache hit 100%) |
| Database work | about 20 transactions and 25 written rows per file |
| Covers stored | 477 MB (see "Problems found") |

Nothing was a bottleneck. The disks were about 15% busy, half the CPU was
idle and the server mostly waited: it handles one file at a time, and each
file is a chain of small reads and database calls. The scan was slowest
while Synology Drive's sync client used 150% CPU and while the server's own
checksum jobs read every file a second time.

**Container Manager shows more memory than the server uses.** It counts the
container's share of the page cache: the music files the scan has read. At
387 MB shown, the server process itself used 46 MB; the rest is cache Linux
hands back whenever another program needs it.

## Problems found, and what changed

Fixed in 1.0.0-beta.31:

- **Covers stored once per track.** Every track of an album carried its own
  copy of the album's art: 1,702 stored covers for 379 different pictures in
  the first 2,000 tracks. Embedded covers are now stored once per picture.
- **Checksums competed with the scan.** Each checksum reads the whole file;
  they now wait until a scan is done.
- **WMA had no tags and no length.** The tag reader does not open WMA, so 84
  files had only a title made from the file name. Tags and length now come
  from `ffprobe` when the tag reader cannot open a file. Files indexed before
  are read again when they change, or after marking them changed.
- **WMA did not play in the web console.** No browser plays it; the console
  now asks the server for an MP3 of anything the browser cannot play.
- **Two albums with one name were one in the console.** A folder whose files
  were tagged partly with MusicBrainz release ids became two albums (53 such
  pairs in this library); the console selected both and showed their songs
  together. The console now selects albums by id. The split itself is still
  open, see `docs/library-scale-plan.md`.
- **Applying an identified album took about 4 seconds a song.** Every song
  fetched the same release from MusicBrainz and downloaded the same cover.
  Releases are kept for a quarter of an hour and covers are stored per
  release: about 1 second a song against musicbrainz.org.
- **Console pages answered 404.** Every console address but `/` came with
  status 404 and the page; browsers showed it anyway. Now 200.
- **The profile picture did not show.** It was saved, but the console showed
  it only on the Family page.

Still open, planned in `docs/library-scale-plan.md`: scanning several files
at once, duplicates, the album split, cheaper identify runs, a MusicBrainz
mirror setting, and a browser test of the console with 10,000 songs.
