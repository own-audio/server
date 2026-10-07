#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Family-style load: N listeners browsing and playing at once, for a while.

Each listener signs in once (so the per-IP login limit is not the bottleneck),
then loops: open home, a list or two, stats, search; start a song or an
audiobook chapter and read it in 256 KB ranges as a player buffers; report
progress and a listening session. `--heavy` adds the whole track list, the
album and artist aggregations and Subsonic browsing to every loop, which no
real client does that often — the full-load case.

    python3 conformance/tools/loadtest.py --base-url http://192.168.88.40:8080 \
        --email guest@demo.own.audio --password own-audio-demo --listeners 4 --seconds 120
"""
import argparse, json, random, statistics, threading, time, urllib.parse, urllib.request, uuid
from datetime import datetime, timedelta, timezone

UA = "own-audio-load/1.0"


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", required=True)
    ap.add_argument("--email", required=True)
    ap.add_argument("--password", required=True)
    ap.add_argument("--listeners", type=int, default=4)
    ap.add_argument("--seconds", type=int, default=120)
    ap.add_argument("--heavy", action="store_true")
    a = ap.parse_args()
    base = a.base_url.rstrip("/")

    def req(path, token=None, body=None, method=None, headers=None):
        h = {"User-Agent": UA, "Accept": "application/json", **(headers or {})}
        data = None
        if token:
            h["Authorization"] = f"Bearer {token}"
        if body is not None:
            data = json.dumps(body).encode()
            h["Content-Type"] = "application/json"
        url = path if path.startswith("http") else base + path
        r = urllib.request.Request(url, data=data, headers=h, method=method)
        t0 = time.time()
        with urllib.request.urlopen(r, timeout=120) as resp:
            raw = resp.read()
        return raw, time.time() - t0

    lat = {}
    lock = threading.Lock()
    bytes_streamed = [0]
    errors = []

    def record(name, secs):
        with lock:
            lat.setdefault(name, []).append(secs)

    def items(raw):
        d = json.loads(raw)
        return d if isinstance(d, list) else d.get("items", [])

    # One sign-in shared by all listeners: the per-IP login limit (30 a minute)
    # is not what this measures.
    shared_token = json.loads(req("/api/v1/auth/login", body={"email": a.email, "password": a.password})[0])["token"]

    def listener(n):
        token = shared_token
        tracks = items(req("/api/v1/music/tracks", token)[0])
        books = items(req("/api/v1/audiobooks", token)[0])
        deadline = time.time() + a.seconds
        rng = random.Random(n)
        while time.time() < deadline:
            try:
                for name, path in [("home: continue", "/api/v1/library/continue"), ("stats", "/api/v1/stats/me?range=30d"),
                                   ("search", "/api/v1/library/search?" + urllib.parse.urlencode({"q": rng.choice(["the", "jazz", "alice", "run"])})),
                                   ("albums", "/api/v1/music/albums"), ("audiobooks", "/api/v1/audiobooks")]:
                    record(name, req(path, token)[1])
                if a.heavy:
                    for name, path in [("track list", "/api/v1/music/tracks"), ("artists", "/api/v1/music/artists"),
                                       ("genres", "/api/v1/music/genres")]:
                        record(name, req(path, token)[1])
                if rng.random() < 0.6 and tracks:
                    t = rng.choice(tracks)
                    url_raw, s = req(f"/api/v1/music/tracks/{t['id']}/stream", token)
                    kind, item_id, part = "music", t["id"], None
                else:
                    b = rng.choice(books)
                    files = items(req(f"/api/v1/audiobooks/{b['id']}/files", token)[0])
                    f = rng.choice(files)
                    url_raw, s = req(f"/api/v1/audiobooks/{b['id']}/files/{f['id']}/stream", token)
                    kind, item_id, part = "audiobook", b["id"], f["id"]
                record("stream link", s)
                url = json.loads(url_raw)["url"]
                # Media links carry the server's public address; stream from the
                # address under test instead, or a LAN test would measure the
                # internet connection (the link is signed over its path only).
                u, bu = urllib.parse.urlsplit(url), urllib.parse.urlsplit(base)
                url = urllib.parse.urlunsplit((bu.scheme, bu.netloc, u.path, u.query, ""))
                # A player buffers the first megabytes, then a seek somewhere in the middle.
                size = None
                for start in (0, 262144, 524288, 1048576, None):
                    if start is None:
                        if not size or size <= 262144:
                            break
                        start = rng.randrange(0, size - 262144)
                    h = {"Range": f"bytes={start}-{start + 262143}"}
                    r = urllib.request.Request(url, headers={"User-Agent": UA, **h})
                    t1 = time.time()
                    with urllib.request.urlopen(r, timeout=120) as resp:
                        raw = resp.read()
                        cr = resp.headers.get("Content-Range", "")
                    record("range read", time.time() - t1)
                    if "/" in cr:
                        size = int(cr.rsplit("/", 1)[1])
                    with lock:
                        bytes_streamed[0] += len(raw)
                    if size and start + 262144 >= size:
                        break
                now = datetime.now(timezone.utc)
                if kind == "audiobook":
                    record("save progress", req(f"/api/v1/playback/books/{item_id}/progress", token,
                                                {"position_secs": rng.randint(0, 600), "completed": False, "file_id": part, "device_kind": "web"}, "PUT")[1])
                record("report session", req("/api/v1/playback/sessions", token, {"sessions": [{
                    "media_kind": kind, "item_id": item_id, "part_id": part,
                    "started_at": (now - timedelta(seconds=30)).isoformat().replace("+00:00", "Z"),
                    "ended_at": now.isoformat().replace("+00:00", "Z"), "seconds_listened": 30,
                    "device_kind": "web", "client_session_id": str(uuid.uuid4()), "ended_reason": "stopped"}]}, "POST")[1])
            except Exception as e:  # noqa: BLE001 — count and carry on, like a client would retry
                with lock:
                    errors.append(f"{type(e).__name__}: {e}"[:120])
                time.sleep(1)

    threads = [threading.Thread(target=listener, args=(i,)) for i in range(a.listeners)]
    t0 = time.time()
    for t in threads:
        t.start()
    for t in threads:
        t.join()
    took = time.time() - t0
    total = sum(len(v) for v in lat.values())
    print(f"{a.listeners} listeners, {took:.0f} s, {total} requests ({total / took:.1f}/s), "
          f"{bytes_streamed[0] / 1048576:.0f} MB streamed, {len(errors)} errors")
    for name, v in sorted(lat.items()):
        v.sort()
        p95 = v[min(len(v) - 1, int(len(v) * 0.95))]
        print(f"  {name:16} n={len(v):5}  median {statistics.median(v) * 1000:6.0f} ms  p95 {p95 * 1000:6.0f} ms")
    for e in sorted(set(errors))[:5]:
        print("  error:", e)


if __name__ == "__main__":
    main()
