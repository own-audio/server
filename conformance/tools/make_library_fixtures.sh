#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds a tiny library for the `library` conformance suite: three tagged
# songs on one album, one song carrying MusicBrainz ids, and an audiobook of
# three files, all generated tones,
# so nothing copyrighted lands in CI. Needs ffmpeg.
#
#   conformance/tools/make_library_fixtures.sh /tmp/oa-library
set -euo pipefail
out=${1:-/tmp/oa-library}
rm -rf "$out"
mkdir -p "$out/music/Test Artist/Test Album" "$out/audiobooks/Test Author/Test Book"

tone() { # file seconds frequency [metadata...]
  local file=$1 secs=$2 freq=$3; shift 3
  ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=$freq:duration=$secs" -ac 1 -ar 22050 -b:a 32k "$@" "$file"
}

for n in 1 2 3; do
  tone "$out/music/Test Artist/Test Album/0$n Song $n.mp3" 3 $((300 + n * 100)) \
    -metadata title="Song $n" -metadata artist="Test Artist" -metadata album="Test Album" \
    -metadata track="$n" -metadata genre="Conformance"
done
# A song as MusicBrainz Picard leaves it: its MusicBrainz ids in its own tags
# (Vorbis MUSICBRAINZ_TRACKID is the recording id). Made-up ids, valid UUIDs.
mkdir -p "$out/music/Test Artist/Tagged Album"
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=800:duration=3" -ac 1 -ar 22050 -c:a flac \
  -metadata title="Tagged Song" -metadata artist="Test Artist" -metadata album="Tagged Album" \
  -metadata track=1 -metadata MUSICBRAINZ_TRACKID=6f1a7e3c-0b5d-4c1e-9a2f-3d4b5c6d7e01 \
  -metadata MUSICBRAINZ_ALBUMID=6f1a7e3c-0b5d-4c1e-9a2f-3d4b5c6d7e02 \
  -metadata MUSICBRAINZ_RELEASEGROUPID=6f1a7e3c-0b5d-4c1e-9a2f-3d4b5c6d7e03 \
  "$out/music/Test Artist/Tagged Album/01 Tagged Song.flac"
# WMA, which the tag reader cannot open: its tags and length come through ffprobe.
mkdir -p "$out/music/Test Artist/Windows Album"
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=900:duration=3" -ac 1 -ar 22050 -c:a wmav2 -b:a 64k \
  -metadata title="Windows Song" -metadata artist="Test Artist" -metadata album="Windows Album" \
  "$out/music/Test Artist/Windows Album/01 Windows Song.wma"
# A NAS's own folder inside the share (Synology thumbnails and index data):
# never part of the library, whatever it holds.
mkdir -p "$out/music/@eaDir/01 Song 1.mp3"
tone "$out/music/@eaDir/01 Song 1.mp3/SYNO_PREVIEW.mp3" 1 200 -metadata title="Not Music"
# Files named so that only natural order is right: 2 before 10.
tone "$out/audiobooks/Test Author/Test Book/2.mp3" 2 500 -metadata title="Part two"
tone "$out/audiobooks/Test Author/Test Book/10.mp3" 2 600 -metadata title="Part ten"
tone "$out/audiobooks/Test Author/Test Book/1.mp3" 2 400 -metadata title="Part one"
# One file with its chapter marks inside, as M4B audiobooks come.
mkdir -p "$out/audiobooks/Test Author/Marked Book"
meta=$(mktemp)
printf ';FFMETADATA1\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=0\nEND=2000\ntitle=Opening\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=2000\nEND=4000\ntitle=Middle\n[CHAPTER]\nTIMEBASE=1/1000\nSTART=4000\nEND=6000\ntitle=Ending\n' > "$meta"
ffmpeg -loglevel error -y -f lavfi -i "sine=frequency=700:duration=6" -i "$meta" -map 0:a -map_metadata 1 -map_chapters 1 \
  -ac 1 -ar 22050 -c:a aac -b:a 32k -metadata title="Marked Book" "$out/audiobooks/Test Author/Marked Book/Marked Book.m4b"
rm -f "$meta"
echo "fixtures in $out"
