#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds a tiny library for the `library` conformance suite: three tagged
# songs on one album and an audiobook of three files, all generated tones,
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
# Files named so that only natural order is right: 2 before 10.
tone "$out/audiobooks/Test Author/Test Book/2.mp3" 2 500 -metadata title="Part two"
tone "$out/audiobooks/Test Author/Test Book/10.mp3" 2 600 -metadata title="Part ten"
tone "$out/audiobooks/Test Author/Test Book/1.mp3" 2 400 -metadata title="Part one"
echo "fixtures in $out"
