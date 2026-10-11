# SPDX-License-Identifier: AGPL-3.0-or-later
"""Read-only library folders: the scan, what it creates, streaming from a folder, hiding.

Runs only against a server with folders configured (features.library_folders),
e.g. the compose stack with conformance/compose.library.yml and the fixtures
from tools/make_library_fixtures.sh: one album of three songs, one book of
three files named 1, 2 and 10.
"""
import os
import shutil
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

    tagged(ctx)
    identify_run(ctx)

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

        # A file that disappears hides its song; when it is back, the same song returns. Needs
        # the fixture folder on this machine (the CI job and a local compose stack have it).
        fixtures = os.environ.get("LIBRARY_FIXTURES", "/tmp/oa-library")
        album_dir = os.path.join(fixtures, "music", "Test Artist", "Test Album")
        survivor = next((t for t in after if os.path.exists(os.path.join(album_dir, f"0{t['title'][-1]} {t['title']}.mp3"))), None)
        if survivor is None:
            ctx.skip("missing folder files", f"{album_dir} is not on this machine")
            return
        name = f"0{survivor['title'][-1]} {survivor['title']}.mp3"
        held = os.path.join(fixtures, "held")
        os.makedirs(held, exist_ok=True)
        shutil.move(os.path.join(album_dir, name), os.path.join(held, name))
        try:
            rescan(ctx)
            gone = _items(ctx.call("GET", "/api/v1/music/tracks?limit=500", ctx.admin_token))
            ctx.check("a song whose file is gone is hidden", all(t["id"] != survivor["id"] for t in gone))
            ctx.call("GET", f"/api/v1/music/tracks/{survivor['id']}", ctx.admin_token, expect=(404,))
        finally:
            shutil.move(os.path.join(held, name), os.path.join(album_dir, name))
        rescan(ctx)
        back = _items(ctx.call("GET", "/api/v1/music/tracks?limit=500", ctx.admin_token))
        ctx.check("the same song returns when its file is back", any(t["id"] == survivor["id"] for t in back))


TAGGED_RECORDING = "6f1a7e3c-0b5d-4c1e-9a2f-3d4b5c6d7e01"


def tagged(ctx):
    """A file carrying its own MusicBrainz ids arrives identified; a revert takes the ids
    away, and re-reading the file brings them back (revision 9)."""
    if (ctx.server.get("api") or {}).get("revision", 0) < 9:
        ctx.skip("MusicBrainz ids from tags", "server older than revision 9")
        return
    song = next((t for t in _items(ctx.call("GET", "/api/v1/music/tracks?limit=500", ctx.admin_token))
                 if t.get("album") == "Tagged Album"), None)
    if song is None:
        ctx.skip("MusicBrainz ids from tags", "fixtures older than the tagged song")
        return
    ctx.check("a file's own MusicBrainz ids are taken without a lookup",
              song.get("musicbrainz_recording_id") == TAGGED_RECORDING, str(song.get("musicbrainz_recording_id")))
    path = f"/api/v1/music/tracks/{song['id']}/metadata"
    reverted = ctx.call("POST", f"{path}/revert", ctx.admin_token)
    ctx.check("a revert takes the ids away and keeps the tags",
              reverted.get("musicbrainz_recording_id") is None and reverted.get("title") == "Tagged Song",
              str((reverted.get("musicbrainz_recording_id"), reverted.get("title"))))
    ctx.call("POST", f"{path}/revert", ctx.admin_token, expect=(400,))
    again = ctx.call("POST", f"{path}/rescan", ctx.admin_token)
    ctx.check("re-reading the file brings its ids back",
              again.get("musicbrainz_recording_id") == TAGGED_RECORDING, str(again.get("musicbrainz_recording_id")))


def identify_run(ctx):
    """An identify run over the library (revision 10): it finishes, every
    proposal has a known class, sure ones accept in bulk, and a revert undoes
    exactly what was accepted. Asks the configured MusicBrainz source, so it is
    skipped when that does not answer in time."""
    if (ctx.server.get("api") or {}).get("revision", 0) < 10 or not ctx.feature("identify_library"):
        ctx.skip("identify runs", "not offered by this server")
        return
    run = ctx.call("POST", "/api/v2/identify/runs", ctx.admin_token,
                   body={"scope": {"kind": "library", "label": "conformance"}}, expect=(202,))
    ctx.check("a run starts queued", run.get("status") in ("queued", "running"), str(run.get("status")))
    path = f"/api/v2/identify/runs/{run['id']}"
    deadline = time.time() + 180
    while time.time() < deadline and run.get("status") not in ("ready", "failed"):
        time.sleep(2)
        run = ctx.call("GET", path, ctx.admin_token)
    if run.get("status") != "ready":
        ctx.skip("identify runs", f"the MusicBrainz source did not finish in time: {run.get('status')} {run.get('error')}")
        ctx.call("DELETE", path, ctx.admin_token, expect=(204,))
        return
    ctx.check("the run looked at every group", run["groups_done"] == run["groups_total"] and run["tracks"] >= 1,
              str((run["groups_done"], run["groups_total"], run["tracks"])))
    proposals = ctx.call("GET", f"{path}/proposals?limit=500", ctx.admin_token).get("items", [])
    ctx.check("every track in scope has a proposal", len(proposals) == run["tracks"], f"{len(proposals)} of {run['tracks']}")
    ctx.check("proposals have a known class and say why",
              all(p["class"] in ("sure", "review", "none") and (p["class"] == "sure") == (not p["reasons"]) for p in proposals))
    ctx.check("the tagged song, identified from its file, is not in the run",
              all(p["track"]["album"] != "Tagged Album" for p in proposals))
    sure = sum(1 for p in proposals if p["class"] == "sure")
    accepted = ctx.call("POST", f"{path}/accept", ctx.admin_token, body={"class": "sure"})["accepted"]
    ctx.check("accepting the sure ones accepts exactly those", accepted == sure, f"{accepted} vs {sure}")
    ctx.call("POST", f"{path}/accept", ctx.admin_token, body={"class": "review"}, expect=(400,))
    reverted = ctx.call("POST", f"{path}/revert", ctx.admin_token)["reverted"]
    ctx.check("a run's revert undoes what it accepted", reverted == accepted, f"{reverted} vs {accepted}")
    _, _, other = ctx.make_user("stranger", "Stranger123!-long")
    ctx.call("GET", path, other, expect=(404,))
    ctx.call("DELETE", path, ctx.admin_token, expect=(204,))
    ctx.call("GET", path, ctx.admin_token, expect=(404,))


def rescan(ctx):
    ctx.call("POST", "/api/v1/library/folders/scan", ctx.admin_token, expect=(202,))
    time.sleep(5)
    deadline = time.time() + 60
    while time.time() < deadline and ctx.call("GET", "/api/v1/library/folders", ctx.admin_token).get("scanning"):
        time.sleep(1)
