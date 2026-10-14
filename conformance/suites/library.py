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
    if ctx.server["api"]["revision"] >= 11:
        quote = ctx.call("POST", "/api/v2/identify/quote", ctx.admin_token, body={"scope": {"kind": "library"}})
        ctx.check("a quote counts the tracks a run would look up",
                  quote.get("tracks", 0) >= 1 and isinstance(quote.get("charge_micro"), int), str(quote))
        if ctx.server.get("edition") != "hosted":
            ctx.check("identify is free in the open-source edition", quote.get("charge_micro") == 0, str(quote))
        ctx.call("POST", "/api/v2/identify/quote", ctx.admin_token, body={"scope": {"kind": "album"}}, expect=(400,))
    run = ctx.call("POST", "/api/v2/identify/runs", ctx.admin_token,
                   body={"scope": {"kind": "library", "label": "conformance"}}, expect=(202,))
    ctx.check("a run starts queued", run.get("status") in ("queued", "running"), str(run.get("status")))
    path = f"/api/v2/identify/runs/{run['id']}"
    if ctx.server["api"]["revision"] >= 12:
        told = identify_event(ctx)
        ctx.check("the event stream tells a run's progress at once", told.get("run_id") == run["id"]
                  and told.get("status") in ("queued", "running", "ready") and "groups_total" in told, str(told)[:160])
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
    one = 0
    if accepted and ctx.server["api"]["revision"] >= 12:
        taken = next(p for p in proposals if p["class"] == "sure")
        track = ctx.call("DELETE", f"/api/v2/tracks/{taken['track']['id']}/identity", ctx.admin_token)
        ctx.check("one track's identify can be undone", track.get("id") == taken["track"]["id"], str(track)[:120])
        again = ctx.call("GET", f"{path}/proposals?decision=pending&limit=500", ctx.admin_token).get("items", [])
        ctx.check("…and its proposal is a decision to make again", any(p["id"] == taken["id"] for p in again))
        ctx.call("DELETE", f"/api/v2/tracks/{taken['track']['id']}/identity", ctx.admin_token, expect=(409,))
        one = 1
    reverted = ctx.call("POST", f"{path}/revert", ctx.admin_token)["reverted"]
    ctx.check("a run's revert undoes what it accepted", reverted == accepted - one, f"{reverted} vs {accepted} - {one}")
    _, _, other = ctx.make_user("stranger", "Stranger123!-long")
    if ctx.server["api"]["revision"] >= 13 and proposals:
        own_words(ctx, proposals[0], other)
    if ctx.server["api"]["revision"] >= 14:
        album_in_own_words(ctx, proposals)
    ctx.call("GET", path, other, expect=(404,))
    ctx.call("DELETE", path, ctx.admin_token, expect=(204,))
    ctx.call("GET", path, ctx.admin_token, expect=(404,))


def own_words(ctx, proposal, other):
    """Revision 13: search again with the owner's words, pick a recording by id
    or link, and identify any track the same way, then undo it."""
    pid, tid = proposal["id"], proposal["track"]["id"]
    alts = ctx.call("GET", f"/api/v2/identify/proposals/{pid}/alternatives", ctx.admin_token)
    ctx.check("a proposal lists its other recordings", isinstance(alts.get("items"), list), str(alts)[:120])
    ctx.call("POST", f"/api/v2/identify/proposals/{pid}/search", ctx.admin_token, body={"title": "  "}, expect=(400,))
    ctx.call("PUT", f"/api/v2/identify/proposals/{pid}/recording", ctx.admin_token,
             body={"recording": "track 02"}, expect=(400,))
    # The fixtures' tags are made up, as a "Track 02" is: the owner types what
    # the song really is.
    words = {"title": "Yesterday", "artist": "The Beatles"}
    found = ctx.call("POST", f"/api/v2/identify/proposals/{pid}/search", ctx.admin_token, body=words).get("items", [])
    stored = ctx.call("GET", f"/api/v2/identify/proposals/{pid}/alternatives", ctx.admin_token).get("items", [])
    ctx.check("a search's results become the alternatives",
              [c["mb_recording_id"] for c in stored] == [c["mb_recording_id"] for c in found])
    if not found:
        ctx.skip("picking a recording", "the MusicBrainz source found nothing for the search")
        return
    pick = found[0]
    link = f"https://musicbrainz.org/recording/{pick['mb_recording_id']}"
    chosen = ctx.call("PUT", f"/api/v2/identify/proposals/{pid}/recording", ctx.admin_token,
                      body={"recording": link, "release": pick.get("mb_release_id")})
    ctx.check("a picked recording is the owner's proposal, to accept",
              chosen.get("chosen_by_owner") is True and chosen.get("reasons") == ["chosen"]
              and chosen.get("decision") == "pending"
              and (chosen.get("proposal") or {}).get("mb_recording_id") == pick["mb_recording_id"], str(chosen)[:200])
    ctx.call("POST", f"/api/v2/tracks/{tid}/identity/search", other, body=words, expect=(403, 404))
    again = ctx.call("POST", f"/api/v2/tracks/{tid}/identity/search", ctx.admin_token, body=words).get("items", [])
    ctx.check("any track can be searched for with the owner's words", isinstance(again, list) and len(again) >= 1)
    ctx.call("PUT", f"/api/v2/tracks/{tid}/identity", other, body={"recording": pick["mb_recording_id"]}, expect=(403, 404))
    track = ctx.call("PUT", f"/api/v2/tracks/{tid}/identity", ctx.admin_token,
                     body={"recording": pick["mb_recording_id"], "release": pick.get("mb_release_id")})
    ctx.check("any track can be identified as a recording", track.get("id") == tid, str(track)[:120])
    if ctx.server["api"]["revision"] >= 15:
        # The release's cover follows in the background, in the identify's batch.
        deadline = time.time() + 30
        cover = []
        while time.time() < deadline and not cover:
            time.sleep(2)
            cover = [e for e in ctx.call("GET", f"/api/v2/edits?item_kind=track&item_id={tid}&limit=10", ctx.admin_token)["items"]
                     if "cover_object_id" in e["after"] and e["source"] == "identify"]
        if cover:
            ctx.check("the release's cover comes with an identify, in its batch", cover[0]["after"]["cover_object_id"] is not None)
        else:
            ctx.skip("release cover", "no cover for the release from the cover sources in time")
    ctx.call("DELETE", f"/api/v2/tracks/{tid}/identity", ctx.admin_token)
    if ctx.server["api"]["revision"] >= 15:
        hist = [e for e in ctx.call("GET", f"/api/v2/edits?item_kind=track&item_id={tid}&limit=10", ctx.admin_token).get("items", [])
                if "musicbrainz_recording_id" in e["after"]]
        ctx.check("identifying and undoing it are in the song's history",
                  [e["source"] for e in hist[:2]] == ["revert", "identify"]
                  and hist[1]["after"].get("musicbrainz_recording_id") == pick["mb_recording_id"], str(hist[:2])[:300])


def album_in_own_words(ctx, proposals):
    """Revision 14: an album group's editions searched again with the owner's
    words."""
    album = next((p for p in proposals if p["group"]["kind"] == "album"), None)
    if not album:
        ctx.skip("an album in your own words", "the run has no album group")
        return
    gid = album["group"]["id"]
    ctx.call("POST", f"/api/v2/identify/groups/{gid}/search", ctx.admin_token, body={"album": " "}, expect=(400,))
    group = ctx.call("POST", f"/api/v2/identify/groups/{gid}/search", ctx.admin_token,
                     body={"album": "Abbey Road", "artist": "The Beatles"})
    ctx.check("an album can be searched for with the owner's words",
              group.get("id") == gid and isinstance(group.get("editions"), list), str(group)[:160])
    single = next((p for p in proposals if p["group"]["kind"] == "track"), None)
    if single:
        ctx.call("POST", f"/api/v2/identify/groups/{single['group']['id']}/search", ctx.admin_token,
                 body={"album": "Abbey Road"}, expect=(404,))


def identify_event(ctx) -> dict:
    """The first `identify` event the caller's stream sends, or {}."""
    import json
    import urllib.request
    req = urllib.request.Request(ctx.url("/api/v2/events"),
                                 headers={"Authorization": f"Bearer {ctx.admin_token}", "Accept": "text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=15) as r:
            kind = None
            for raw in r:
                line = raw.decode().strip()
                if line.startswith("event:"):
                    kind = line.split(":", 1)[1].strip()
                elif line.startswith("data:") and kind == "identify":
                    return json.loads(line.split(":", 1)[1])
    except Exception:  # noqa: BLE001 — a timeout means no event came
        pass
    return {}


def rescan(ctx):
    ctx.call("POST", "/api/v1/library/folders/scan", ctx.admin_token, expect=(202,))
    time.sleep(5)
    deadline = time.time() + 60
    while time.time() < deadline and ctx.call("GET", "/api/v1/library/folders", ctx.admin_token).get("scanning"):
        time.sleep(1)
