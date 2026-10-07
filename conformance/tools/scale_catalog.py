#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Fill a TEST database with a data-hoarder catalog and time the server against it.

Rows only — no audio — so 600,000 tracks take a minute to create. Then it
measures what docs/CAPACITY.md promises: server memory that does not grow with
the catalog, and browse calls that stay fast. Never point it at a real
library: it inserts thousands of fake tracks and books into the admin's family.

    python3 conformance/tools/scale_catalog.py --base-url http://localhost:8096 \
        --admin-email admin@example.com --admin-password … \
        --compose-dir . --tracks 600000 --books 1000

Without --tracks it only measures (a catalog created earlier stays).
"""
import argparse, json, os, subprocess, sys, time, urllib.error, urllib.parse, urllib.request

UA = "own-audio-scale/1.0"


def psql(args, sql):
    cmd = ["docker", "compose", "exec", "-T", "postgres", "psql", "-U", args.database_user, "-d", args.database, "-v", "ON_ERROR_STOP=1", "-At", "-c", sql]
    return subprocess.run(cmd, cwd=args.compose_dir, check=True, capture_output=True, text=True).stdout.strip()


def rss_mib(args):
    out = subprocess.run(["docker", "compose", "exec", "-T", "server", "grep", "VmRSS", "/proc/1/status"],
                         cwd=args.compose_dir, capture_output=True, text=True).stdout
    return int(out.split()[1]) // 1024 if out else -1


class Peak:
    """Samples the server's RSS every 200 ms while a call runs."""

    def __init__(self, args):
        import threading
        self.args, self.peak, self.on = args, 0, True
        self.thread = threading.Thread(target=self._run, daemon=True)

    def _run(self):
        while self.on:
            self.peak = max(self.peak, rss_mib(self.args))
            time.sleep(0.2)

    def __enter__(self):
        self.thread.start()
        return self

    def __exit__(self, *exc):
        self.on = False
        self.thread.join()


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", required=True)
    ap.add_argument("--admin-email", required=True)
    ap.add_argument("--admin-password", required=True)
    ap.add_argument("--compose-dir", default=".")
    ap.add_argument("--database", default="ownaudio")
    ap.add_argument("--database-user", default="ownaudio")
    ap.add_argument("--tracks", type=int, default=0, help="generated tracks to add")
    ap.add_argument("--books", type=int, default=0, help="generated audiobooks to add (10 files each)")
    ap.add_argument("--report", help="write the measurements as JSON here")
    a = ap.parse_args()
    base = a.base_url.rstrip("/")

    def call(path, token=None, body=None, method=None):
        h = {"User-Agent": UA, "Accept": "application/json"}
        data = None
        if token:
            h["Authorization"] = f"Bearer {token}"
        if body is not None:
            data = json.dumps(body).encode()
            h["Content-Type"] = "application/json"
        req = urllib.request.Request(base + path, data=data, headers=h, method=method)
        start = time.time()
        with urllib.request.urlopen(req, timeout=600) as r:
            raw = r.read()
        return raw, time.time() - start

    token = json.loads(call("/api/v1/auth/login", body={"email": a.admin_email, "password": a.admin_password})[0])["token"]
    owner = psql(a, f"SELECT u.id || ' ' || fm.family_id FROM users u JOIN family_members fm ON fm.user_id = u.id WHERE lower(u.email) = lower('{a.admin_email}')")
    user_id, family_id = owner.split()

    if a.tracks:
        t0 = time.time()
        psql(a, f"""
            WITH objs AS (
                INSERT INTO media_objects (bucket, object_key, content_type, size_bytes)
                SELECT 'scale', 'scale/t/' || g || '.mp3', 'audio/mpeg', 7700000 FROM generate_series(1, {a.tracks}) g
                RETURNING id, object_key)
            INSERT INTO music_tracks_all (user_id, family_id, title, artist, album, genre, track_number, duration_secs, audio_object_id)
            SELECT '{user_id}', '{family_id}',
                   'Track ' || n, 'Artist ' || (n % 20000), 'Album ' || (n % 50000), 'Genre ' || (n % 40),
                   1 + n % 12, 120 + n % 300, id
            FROM (SELECT id, split_part(split_part(object_key, '/', 3), '.', 1)::int AS n FROM objs) s""")
        print(f"added {a.tracks} tracks in {time.time() - t0:.0f} s")
    if a.books:
        t0 = time.time()
        psql(a, f"""
            WITH books AS (
                INSERT INTO audiobook_books_all (user_id, family_id, title, author, total_duration_secs)
                SELECT '{user_id}', '{family_id}', 'Scale Book ' || g, 'Author ' || (g % 400), 36000
                FROM generate_series(1, {a.books}) g RETURNING id),
            files AS (
                SELECT b.id AS book_id, p FROM books b, generate_series(1, 10) p),
            objs AS (
                INSERT INTO media_objects (bucket, object_key, content_type, size_bytes)
                SELECT 'scale', 'scale/b/' || book_id || '/' || p || '.mp3', 'audio/mpeg', 33000000 FROM files
                RETURNING id, object_key)
            INSERT INTO audiobook_files (book_id, position, title, duration_secs, audio_object_id)
            SELECT split_part(object_key, '/', 3)::uuid, split_part(split_part(object_key, '/', 4), '.', 1)::int,
                   'Chapter ' || split_part(split_part(object_key, '/', 4), '.', 1), 3600, id
            FROM objs""")
        print(f"added {a.books} books in {time.time() - t0:.0f} s")
    psql(a, "ANALYZE")

    counts = psql(a, f"SELECT (SELECT count(*) FROM music_tracks WHERE family_id = '{family_id}') || ' ' || (SELECT count(*) FROM audiobook_books WHERE family_id = '{family_id}')")
    tracks, books = counts.split()
    print(f"catalog: {tracks} tracks, {books} books")
    results = {"tracks": int(tracks), "books": int(books), "idle_rss_mib": rss_mib(a), "calls": {}}
    print(f"server idle: {results['idle_rss_mib']} MiB")

    probes = [
        ("GET /music/tracks (whole list)", "/api/v1/music/tracks"),
        ("GET /music/albums", "/api/v1/music/albums"),
        ("GET /music/artists", "/api/v1/music/artists"),
        ("GET /music/genres", "/api/v1/music/genres"),
        ("GET /audiobooks", "/api/v1/audiobooks"),
        ("GET /library/search?q=Track 1234", "/api/v1/library/search?" + urllib.parse.urlencode({"q": "Track 1234"})),
        ("GET /library/continue", "/api/v1/library/continue"),
        ("GET /library/changes (full sync)", "/api/v1/library/changes"),
        ("GET /sync/tree/ids", "/api/v1/sync/tree/ids"),
    ]
    for label, path in probes:
        try:
            with Peak(a) as p:
                raw, secs = call(path, token)
            results["calls"][label] = {"seconds": round(secs, 2), "bytes": len(raw), "peak_rss_mib": p.peak}
            print(f"{label:40} {secs:7.2f} s  {len(raw) / 1048576:8.1f} MB  peak {p.peak} MiB")
        except Exception as e:  # noqa: BLE001 — a probe that fails is a finding, not a crash
            results["calls"][label] = {"error": str(e)}
            print(f"{label:40} ERROR {e}")

    sub = urllib.parse.urlencode({"u": a.admin_email, "p": json.loads(call("/api/v1/users/me/subsonic-key", token)[0])["api_key"],
                                  "v": "1.16.1", "c": "scale", "f": "json"})
    for label, method, extra in [
        ("Subsonic getAlbumList2 newest 50", "getAlbumList2", "&type=newest&size=50"),
        ("Subsonic getArtists", "getArtists", ""),
        ("Subsonic search3 'Track 99'", "search3", "&query=Track%2099"),
    ]:
        try:
            with Peak(a) as p:
                raw, secs = call(f"/rest/{method}.view?{sub}{extra}")
            results["calls"][label] = {"seconds": round(secs, 2), "bytes": len(raw), "peak_rss_mib": p.peak}
            print(f"{label:40} {secs:7.2f} s  {len(raw) / 1048576:8.1f} MB  peak {p.peak} MiB")
        except Exception as e:  # noqa: BLE001
            results["calls"][label] = {"error": str(e)}
            print(f"{label:40} ERROR {e}")

    # Lookups by id: the first one may have to learn the catalog's ids, the
    # rest should be key reads. A client's album grid asks for many covers.
    try:
        listed = json.loads(call(f"/rest/getAlbumList2.view?{sub}&type=newest&size=50")[0])
        albums = listed["subsonic-response"]["albumList2"]["album"]
        lookups = [("Subsonic getAlbum (first lookup)", "getAlbum", albums[0]["id"]),
                   ("Subsonic getAlbum (another)", "getAlbum", albums[1]["id"]),
                   ("Subsonic getArtist", "getArtist", albums[2]["artistId"])]
        lookups += [(f"Subsonic getCoverArt album {i}", "getCoverArt", al["id"]) for i, al in enumerate(albums[3:23])]
        cover_secs = []
        for label, method, ident in lookups:
            if method == "getCoverArt":
                # Generated rows have no pictures, so a 404 is the expected answer;
                # the time to find that out is what counts.
                start = time.time()
                try:
                    call(f"/rest/{method}.view?{sub}&id={ident}")
                except urllib.error.HTTPError:
                    pass
                cover_secs.append(time.time() - start)
                continue
            with Peak(a) as p:
                raw, secs = call(f"/rest/{method}.view?{sub}&id={ident}")
            results["calls"][label] = {"seconds": round(secs, 3), "bytes": len(raw), "peak_rss_mib": p.peak}
            print(f"{label:40} {secs:7.3f} s  {len(raw) / 1048576:8.1f} MB  peak {p.peak} MiB")
        if cover_secs:
            avg = sum(cover_secs) / len(cover_secs)
            results["calls"]["Subsonic getCoverArt (20 albums, mean)"] = {"seconds": round(avg, 3)}
            print(f"{'Subsonic getCoverArt (20 albums, mean)':40} {avg:7.3f} s")
    except Exception as e:  # noqa: BLE001
        print(f"lookups by id: ERROR {e}")

    results["after_rss_mib"] = rss_mib(a)
    print(f"server after: {results['after_rss_mib']} MiB")
    if a.report:
        with open(a.report, "w") as f:
            json.dump(results, f, indent=2)


if __name__ == "__main__":
    main()
