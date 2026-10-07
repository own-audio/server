# SPDX-License-Identifier: AGPL-3.0-or-later
"""Read-only library folders: the scan, what it creates, streaming from a folder, hiding.

Runs only against a server with folders configured (features.library_folders),
e.g. the compose stack with conformance/compose.library.yml and the fixtures
from tools/make_library_fixtures.sh: one album of three songs, one book of
three files named 1, 2 and 10.
"""
import time
import urllib.request

NAME = "library"


def _items(x):
    return x if isinstance(x, list) else (x or {}).get("items", [])


def run(ctx):
    if not ctx.feature("library_folders"):
        ctx.skip("library folders", "none configured on this server")
        return

    # The first scan starts once the first admin exists; wait for it.
    deadline = time.time() + 120
    folders = []
    while time.time() < deadline:
        status = ctx.call("GET", "/api/v1/library/folders", ctx.admin_token)
        folders = status.get("folders") or []
        if folders and all(f.get("scan_finished_at") for f in folders) and not status.get("scanning"):
            break
        time.sleep(2)
    ctx.check("every configured folder finished a scan", bool(folders) and all(f.get("scan_finished_at") for f in folders),
              str([(f.get("path"), f.get("scan_error")) for f in folders]))
    ctx.check("no folder reports a scan error", all(not f.get("scan_error") for f in folders))

    tracks = [t for t in _items(ctx.call("GET", "/api/v1/music/tracks?limit=500", ctx.admin_token)) if t.get("album") == "Test Album"]
    if not tracks:
        # Folders with other content (a demo, a real library): the scan was
        # checked above; the fixture-specific checks have nothing to look at.
        ctx.skip("fixture library", "tools/make_library_fixtures.sh output is not mounted here")
        return
    ctx.check("the album's three songs are in the library", len(tracks) == 3, str(len(tracks)))
    ctx.check("songs keep their tags", sorted(t.get("title") for t in tracks) == ["Song 1", "Song 2", "Song 3"]
              and all(t.get("artist") == "Test Artist" for t in tracks))
    ctx.check("folder songs are read-only", all(t.get("source") == "folder" and t.get("read_only") is True for t in tracks))
    ctx.check("songs know their length", all((t.get("duration_secs") or 0) >= 2 for t in tracks))

    books = [b for b in _items(ctx.call("GET", "/api/v1/audiobooks", ctx.admin_token)) if b.get("title") == "Test Book"]
    ctx.check("the book folder is one book, its parent the author",
              len(books) == 1 and books[0].get("author") == "Test Author", str(books[:1]))
    if books:
        files = _items(ctx.call("GET", f"/api/v1/audiobooks/{books[0]['id']}/files", ctx.admin_token))
        ctx.check("chapters in natural order (1, 2, 10) with titles from the tags",
                  [f.get("title") for f in files] == ["Part one", "Part two", "Part ten"], str([f.get("title") for f in files]))
        ctx.check("the folder book is read-only", books[0].get("read_only") is True)

    if tracks:
        stream = ctx.call("GET", f"/api/v1/music/tracks/{tracks[0]['id']}/stream", ctx.admin_token)
        url = stream["url"]
        ctx.check("a folder song streams through the server's media route", "/api/v1/media?" in url, url[:60])
        req = urllib.request.Request(url, headers={"Range": "bytes=0-9"})
        with urllib.request.urlopen(req, timeout=30) as r:
            ctx.check("the media route answers a range", r.status == 206 and r.headers.get("Content-Range", "").startswith("bytes 0-9/"),
                      f"{r.status} {r.headers.get('Content-Range')}")

        # Removing a folder song hides it; a rescan must not bring it back.
        victim = tracks[0]
        ctx.call("DELETE", f"/api/v1/music/tracks/{victim['id']}", ctx.admin_token, expect=(200, 204))
        ctx.call("POST", "/api/v1/library/folders/scan", ctx.admin_token, expect=(202,))
        time.sleep(5)
        deadline = time.time() + 60
        while time.time() < deadline and ctx.call("GET", "/api/v1/library/folders", ctx.admin_token).get("scanning"):
            time.sleep(1)
        after = [t for t in _items(ctx.call("GET", "/api/v1/music/tracks?limit=500", ctx.admin_token)) if t.get("album") == "Test Album"]
        ctx.check("a removed folder song stays removed after a rescan",
                  len(after) == 2 and all(t["id"] != victim["id"] and t.get("title") != victim.get("title") for t in after),
                  str([t.get("title") for t in after]))
