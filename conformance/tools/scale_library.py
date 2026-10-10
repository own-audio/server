#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""A data-hoarder library on disk, and a watch on the server while it scans it (issue #1, #2).

    # 600,000 songs (5,000 artists, 50,000 albums) and 1,000 books of tiny real MP3s:
    python3 conformance/tools/scale_library.py generate /library --tracks 600000 --books 1000

    # every 10 s: files the scan has seen, server memory, until the scan finishes
    python3 conformance/tools/scale_library.py watch --base-url http://localhost:8099 \\
        --admin-email … --admin-password … --compose-dir <dir> --project <name>

Each file is an ID3v2.3 tag (title, artist, album, track, genre) and a few silent MPEG
frames, about 2 KB, so the whole library is about 1.3 GB. Generate it on the disk the
server reads (a Docker volume on macOS: a bind mount would measure Docker Desktop's file
sharing, not the server).
"""
import argparse, json, os, subprocess, sys, time, urllib.request

GENRES = ["Rock", "Jazz", "Classical", "Electronic", "Folk", "Hip-Hop", "Pop", "Blues"]
# MPEG-1 Layer III, 128 kbps, 44.1 kHz, no padding: 417 bytes; zeros decode as silence.
FRAME = bytes([0xFF, 0xFB, 0x90, 0x64]) + bytes(413)


def id3(frames):
    body = b""
    for fid, text in frames:
        data = b"\x03" + text.encode("utf-8")  # UTF-8
        body += fid.encode() + len(data).to_bytes(4, "big") + b"\x00\x00" + data
    size = len(body)
    synchsafe = bytes([(size >> 21) & 0x7F, (size >> 14) & 0x7F, (size >> 7) & 0x7F, size & 0x7F])
    return b"ID3\x04\x00\x00" + synchsafe + body


def song(path, title, artist, album, track, genre, frames=4):
    tag = id3([("TIT2", title), ("TPE1", artist), ("TALB", album), ("TRCK", str(track)), ("TCON", genre)])
    with open(path, "wb") as f:
        f.write(tag + FRAME * frames)


def generate(a):
    music, books = os.path.join(a.dir, "music"), os.path.join(a.dir, "audiobooks")
    per_album, albums_per_artist = 12, 10
    albums = (a.tracks + per_album - 1) // per_album
    n, start = 0, time.time()
    for al in range(albums):
        artist = f"Artist {al // albums_per_artist:05d}"
        album = f"Album {al:06d}"
        d = os.path.join(music, artist, album)
        os.makedirs(d, exist_ok=True)
        for t in range(1, per_album + 1):
            if n >= a.tracks:
                break
            song(os.path.join(d, f"{t:02d} Song {n:06d}.mp3"), f"Song {n:06d}", artist, album, t, GENRES[al % len(GENRES)])
            n += 1
        if al % 5000 == 0:
            print(f"{n} songs, {time.time() - start:.0f} s", flush=True)
    for b in range(a.books):
        d = os.path.join(books, f"Author {b // 5:04d}", f"Book {b:04d}")
        os.makedirs(d, exist_ok=True)
        for c in range(1, a.chapters + 1):
            song(os.path.join(d, f"{c:02d}.mp3"), f"Chapter {c}", f"Author {b // 5:04d}", f"Book {b:04d}", c, "Audiobook")
    print(f"done: {n} songs, {a.books} books, {time.time() - start:.0f} s", flush=True)


def call(a, method, path, body=None, token=None):
    req = urllib.request.Request(a.base_url + path, method=method, data=json.dumps(body).encode() if body is not None else None,
                                 headers={"Content-Type": "application/json", "User-Agent": "own-audio-scale/1.0",
                                          **({"Authorization": f"Bearer {token}"} if token else {})})
    with urllib.request.urlopen(req, timeout=60) as r:
        return json.load(r) if r.length != 0 else None


def compose(a, *cmd):
    return subprocess.run(["docker", "compose", "-p", a.project, *cmd], cwd=a.compose_dir, capture_output=True, text=True).stdout


def rss_mib(a):
    out = compose(a, "exec", "-T", "server", "grep", "VmRSS", "/proc/1/status")
    return int(out.split()[1]) // 1024 if out else -1


def db_mib(a):
    out = compose(a, "exec", "-T", "postgres", "psql", "-U", "ownaudio", "-d", "ownaudio", "-At", "-c",
                  "SELECT pg_database_size(current_database()) / 1048576")
    return int(out.strip() or -1)


def watch(a):
    token = call(a, "POST", "/api/v1/auth/login", {"email": a.admin_email, "password": a.admin_password})["token"]
    start, peak, finished_seen = time.time(), 0, False
    print("minutes,files_scanned,files_added_last_scan,rss_mib,peak_mib,db_mib,scanning", flush=True)
    while True:
        try:
            status = call(a, "GET", "/api/v1/library/folders", token=token)
        except Exception as e:  # a busy server may time out; keep watching
            print(f"# {e}", flush=True)
            time.sleep(a.every)
            continue
        folders = status.get("folders") or []
        seen = status.get("files_scanned") or 0
        added = sum(f.get("files_added") or 0 for f in folders)
        rss = rss_mib(a)
        peak = max(peak, rss)
        scanning = bool(status.get("scanning"))
        print(f"{(time.time() - start) / 60:.1f},{seen},{added},{rss},{peak},{db_mib(a)},{scanning}", flush=True)
        if folders and all(f.get("scan_finished_at") for f in folders) and not scanning:
            if finished_seen:
                break
            finished_seen = True
        time.sleep(a.every)
    print(json.dumps({"folders": folders}, indent=1), flush=True)


def main():
    ap = argparse.ArgumentParser()
    sub = ap.add_subparsers(dest="cmd", required=True)
    g = sub.add_parser("generate")
    g.add_argument("dir")
    g.add_argument("--tracks", type=int, default=600_000)
    g.add_argument("--books", type=int, default=1_000)
    g.add_argument("--chapters", type=int, default=10)
    w = sub.add_parser("watch")
    w.add_argument("--base-url", required=True)
    w.add_argument("--admin-email", required=True)
    w.add_argument("--admin-password", required=True)
    w.add_argument("--compose-dir", default=".")
    w.add_argument("--project", default="own-audio")
    w.add_argument("--every", type=int, default=10)
    a = ap.parse_args()
    generate(a) if a.cmd == "generate" else watch(a)


if __name__ == "__main__":
    sys.exit(main())
