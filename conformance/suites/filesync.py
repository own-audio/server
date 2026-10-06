# SPDX-License-Identifier: AGPL-3.0-or-later
"""File sync: default paths, from-upload, the sync feed and cursor, shortcuts, holdings, auto-stored episodes, companion files, organise, fork.

Ported from audio2/scripts/filesync_test.py (docs/file-sync-plan.md §5.2–§5.9).
Builds a family (owner = family admin, adult = member) plus an outsider, then
checks paths, the sync feed and its cursor, reconciliation ids, family
shortcuts, device holdings and auto-stored podcast episodes. Serves a small
RSS feed from this machine for the podcast part, reached from the backend
container as host.docker.internal — hence REQUIRES local (loopback base URL
plus --compose-dir for SQL).
"""
from __future__ import annotations

import base64
import datetime
import email.utils
import http.server
import json
import subprocess
import threading
import time
import unicodedata
import urllib.parse
import urllib.request
import uuid

from core import Ctx

NAME = "filesync"
REQUIRES = {"local"}
PW = "SyncTest123!"


def upload(ctx: Ctx, tok: str, title: str, visibility: str = "family", artist: str = "Sync Test"):
    # Not ctx.upload_track: that always sends an album, and the default-path
    # checks below rely on the server filling in "Unknown Album".
    return ctx.call("POST", "/api/v1/music/tracks/upload", tok, multipart={
        "title": title, "artist": artist, "visibility": visibility,
        "file": (f"{uuid.uuid4().hex}.mp3", ctx.audio())})


def pull(ctx: Ctx, tok: str, cursor: str | None = None, limit: int = 500):
    """Every page of one round: (items by id, removed by id, final cursor, reset seen)."""
    items, removed, reset = {}, {}, False
    while True:
        q = f"?limit={limit}" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else "")
        page = ctx.call("GET", "/api/v1/sync/tree" + q, tok)
        reset = reset or page["reset"]
        for i in page["items"]:
            items[i["id"]] = i
            removed.pop(i["id"], None)
        for r in page["removed"]:
            removed[r["id"]] = r
            items.pop(r["id"], None)
        cursor = page["cursor"]
        if not page["has_more"]:
            return items, removed, cursor, reset


class _FeedServer:
    """A tiny RSS feed + enclosure server the backend container reaches as host.docker.internal."""

    def __init__(self, ctx: Ctx, episodes: list[tuple[str, str, datetime.datetime]]) -> None:
        sfx = ctx.sfx
        outer = self

        class Feed(http.server.BaseHTTPRequestHandler):
            def do_GET(self):
                if self.path.startswith("/rss"):
                    entries = "".join(
                        f"<item><guid>{g}-{sfx}</guid><title>{t}</title><pubDate>{email.utils.format_datetime(d)}</pubDate>"
                        f"<enclosure url='http://host.docker.internal:{outer.port}/{g}.mp3' type='audio/mpeg' length='1'/></item>"
                        for g, t, d in episodes)
                    body = (f"<?xml version='1.0'?><rss version='2.0'><channel><title>Show: {sfx}</title>"
                            f"<link>http://example.com</link><description>x</description>{entries}</channel></rss>").encode()
                    ctype = "application/rss+xml"
                else:
                    body, ctype = ctx.audio(), "audio/mpeg"
                self.send_response(200)
                self.send_header("Content-Type", ctype)
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body)

            def log_message(self, *a):
                pass

        self.server = http.server.ThreadingHTTPServer(("0.0.0.0", 0), Feed)
        self.port = self.server.server_address[1]
        self._stopped = False
        threading.Thread(target=self.server.serve_forever, daemon=True).start()

    def stop(self) -> None:
        if self._stopped:
            return
        self._stopped = True
        self.server.shutdown()
        self.server.server_close()


def run(ctx: Ctx) -> None:
    sfx = ctx.sfx
    check, call, sql, log = ctx.check, ctx.call, ctx.sql, ctx.log

    def presigned(tok, kind, filename, content=None):
        return ctx.presign_upload(tok, kind, filename, content)

    owner_id, owner_email, owner_tok = ctx.make_user("owner", PW)
    adult_id, adult_email, adult_tok = ctx.make_user("adult", PW)
    out_id, out_email, out_tok = ctx.make_user("outsider", PW)
    # ctx.make_user names users "<label> <sfx>"; three checks compare against it.
    owner_name = f"owner {sfx}"
    server: _FeedServer | None = None

    try:
        inv = call("POST", "/api/v1/family/invites", owner_tok, {"email": adult_email, "role": "member"})
        call("POST", "/api/v1/family/invites/accept", adult_tok, {"code": inv["code"]})

        # ── Default paths ─────────────────────────────────────────────────────
        log("\n[default paths]")
        t1 = upload(ctx, owner_tok, f"Song: One {sfx}")
        items, _, cursor, _ = pull(ctx, owner_tok)
        check("a multipart upload gets its default path at once",
              items.get(t1["id"], {}).get("path") == f"Music/Sync Test/Unknown Album/Song - One {sfx}.mp3")
        row = items.get(t1["id"], {})
        check("a track has one file with an empty relative path",
              len(row.get("files", [])) == 1 and row["files"][0]["relative_path"] == "" and row["files"][0]["size_bytes"] > 0)
        check("owner and rights are set", bool(row.get("is_owner") and row.get("can_delete") and row.get("shared_with_family")))
        check("album identity is sent for shortcuts", row.get("album", {}).get("album_artist") == "Sync Test")

        call("PUT", f"/api/v1/music/tracks/{t1['id']}", owner_tok, {"title": "Renamed", "artist": "Other"})
        items, _, _, _ = pull(ctx, owner_tok)
        check("a metadata edit never moves the file",
              items[t1["id"]]["path"] == f"Music/Sync Test/Unknown Album/Song - One {sfx}.mp3")

        # A track from before paths existed gets one when the feed first sees it.
        old = upload(ctx, owner_tok, f"Old {sfx}")
        sql(f"DELETE FROM sync_paths WHERE item_id = '{old['id']}'")
        items, _, _, _ = pull(ctx, owner_tok)
        check("the backfill gives an old item its default path",
              items.get(old["id"], {}).get("path") == f"Music/Sync Test/Unknown Album/Old {sfx}.mp3")

        # ── Finder uploads keep the user's path ───────────────────────────────
        log("\n[from-upload]")
        try:
            key = presigned(owner_tok, "music_track", "track01.mp3")
            presign_ok = True
        except Exception as e:  # noqa: BLE001
            ctx.skip("from-upload", f"presigned PUT not reachable from here: {e}")
            presign_ok = False
        if presign_ok:
            decomposed = unicodedata.normalize("NFD", f"Music/Moje oblíbené {sfx}/track01.mp3")
            r = call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                     {"object_key": key, "path": decomposed, "original_filename": "track01.mp3", "visibility": "private"})
            check("from-upload keeps the path, stored composed (NFC)",
                  r["path"] == unicodedata.normalize("NFC", f"Music/Moje oblíbené {sfx}/track01.mp3"))
            check("from-upload returns the track", bool(r["title"] and r["visibility"] == "private" and r["is_owner"]))
            key2 = presigned(owner_tok, "music_track", "TRACK01.mp3")
            r2 = call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                      {"object_key": key2, "path": f"Music/moje OBLÍBENÉ {sfx}/TRACK01.mp3", "original_filename": "TRACK01.mp3"})
            check("the same path in other case gets ' (2)'", r2["path"] == f"Music/moje OBLÍBENÉ {sfx}/TRACK01 (2).mp3")
            key3 = presigned(owner_tok, "music_track", "x.mp3")
            call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                 {"object_key": key3, "path": "Audiobooks/x.mp3", "original_filename": "x.mp3"}, expect=(400,))
            check("a track path outside Music/ is refused", True)
            call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                 {"object_key": key3, "path": "Music/../x.mp3", "original_filename": "x.mp3"}, expect=(400,))
            check("a '..' component is refused", True)
            r3 = call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                      {"object_key": key3, "original_filename": "x.mp3"})
            check("without a path it gets the default", r3["path"] == "Music/Unknown Artist/Unknown Album/x.mp3"
                  or r3["path"].startswith("Music/Unknown Artist/Unknown Album/x"))
            call("POST", "/api/v1/music/tracks/from-upload", out_tok,
                 {"object_key": key3, "original_filename": "x.mp3"}, expect=(401, 404))
            check("another family's key is refused", True)

            # Books.
            log("\n[book from-uploads]")
            k1, k2 = presigned(owner_tok, "audiobook_file", "01.mp3"), presigned(owner_tok, "audiobook_file", "02.mp3")
            book = call("POST", "/api/v1/audiobooks/from-uploads", owner_tok, {
                "title": f"Mort {sfx}", "author": "Pratchett", "visibility": "family",
                "path": f"Audiobooks/Pratchett {sfx}/Mort",
                "files": [{"object_key": k2, "relative_path": "CD1/02 Two.mp3"},
                          {"object_key": k1, "relative_path": "CD1/01 One.mp3"}]})
            check("a Finder book keeps its folder", book["path"] == f"Audiobooks/Pratchett {sfx}/Mort")
            items, _, _, _ = pull(ctx, owner_tok)
            files = [f["relative_path"] for f in items.get(book["id"], {}).get("files", [])]
            check("its files keep their names, in play order", files == ["CD1/01 One.mp3", "CD1/02 Two.mp3"])

            # More files for the same book: batches from Finder, or a chapter added later.
            k0, k9 = presigned(owner_tok, "audiobook_file", "00.mp3"), presigned(owner_tok, "audiobook_file", "09.mp3")
            added = call("POST", f"/api/v1/audiobooks/{book['id']}/files/from-uploads", owner_tok,
                         {"files": [{"object_key": k9, "relative_path": "CD2/09 Nine.mp3"},
                                    {"object_key": k0, "relative_path": "CD1/00 Intro.mp3"}]})
            check("files are added to an existing book", len(added) == 2)
            again = call("POST", f"/api/v1/audiobooks/{book['id']}/files/from-uploads", owner_tok,
                         {"files": [{"object_key": k9, "relative_path": "cd2/09 nine.mp3"}]})
            check("adding the same path again returns the file already there", again[0]["id"] == added[0]["id"])
            items, _, _, _ = pull(ctx, owner_tok)
            files = [f["relative_path"] for f in items.get(book["id"], {}).get("files", [])]
            check("play order follows the paths again",
                  files == ["CD1/00 Intro.mp3", "CD1/01 One.mp3", "CD1/02 Two.mp3", "CD2/09 Nine.mp3"])
            call("POST", f"/api/v1/audiobooks/{book['id']}/files/from-uploads", adult_tok,
                 {"files": [{"object_key": k9, "relative_path": "x.mp3"}]}, expect=(401, 404))
            check("only the owner adds files", True)

            k3 = presigned(owner_tok, "audiobook_file", "a.mp3")
            nested = call("POST", "/api/v1/audiobooks/from-uploads", owner_tok, {
                "title": "Outer", "path": f"Audiobooks/Pratchett {sfx}",
                "files": [{"object_key": k3, "relative_path": "a.mp3"}]})
            check("a book folder around another book gets ' (2)'", nested["path"] == f"Audiobooks/Pratchett {sfx} (2)")

            k4 = presigned(owner_tok, "audiobook_file", "loose.m4b")
            loose = call("POST", "/api/v1/audiobooks/from-uploads", owner_tok, {
                "title": "Loose", "path": f"Audiobooks/Loose {sfx}.m4b", "files": [{"object_key": k4}]})
            items, _, _, _ = pull(ctx, owner_tok)
            lf = items.get(loose["id"], {}).get("files", [])
            check("one loose file: the path is the file, relative path empty",
                  loose["path"] == f"Audiobooks/Loose {sfx}.m4b" and len(lf) == 1 and lf[0]["relative_path"] == "")

            k5, k6 = presigned(owner_tok, "audiobook_file", "p1.mp3"), presigned(owner_tok, "audiobook_file", "p2.mp3")
            web = call("POST", "/api/v1/audiobooks/from-uploads", owner_tok, {
                "title": f"Web Book {sfx}", "author": "Anon: Writer",
                "files": [{"object_key": k5, "relative_path": "Folder/p1.mp3", "title": "Part One"},
                          {"object_key": k6, "relative_path": "Folder/p2.mp3"}]})
            items, _, _, _ = pull(ctx, owner_tok)
            files = [f["relative_path"] for f in items.get(web["id"], {}).get("files", [])]
            check("a web upload gets the default folder", web["path"] == f"Audiobooks/Anon - Writer/Web Book {sfx}")
            check("and default file names", files == ["01 - Part One.mp3", "02 - p2.mp3"])

            k7 = presigned(owner_tok, "audiobook_file", "d.mp3")
            call("POST", "/api/v1/audiobooks/from-uploads", owner_tok, {
                "title": "Dup", "path": f"Audiobooks/Dup {sfx}",
                "files": [{"object_key": k7, "relative_path": "a.mp3"}, {"object_key": k7, "relative_path": "A.mp3"}]},
                expect=(400,))
            check("two files with the same name (any case) are refused", True)

        # ── The feed: changes, trash, restore, sharing ────────────────────────
        log("\n[sync feed]")
        _, _, adult_cursor, _ = pull(ctx, adult_tok)
        _, _, owner_cursor, _ = pull(ctx, owner_tok)
        t2 = upload(ctx, owner_tok, f"Delta {sfx}")
        items, removed, owner_cursor, _ = pull(ctx, owner_tok, owner_cursor)
        check("a new track arrives in the next delta", t2["id"] in items)
        items, removed, owner_cursor, _ = pull(ctx, owner_tok, owner_cursor)
        check("and not again after that", t2["id"] not in items and not removed)

        items, _, adult_cursor, _ = pull(ctx, adult_tok, adult_cursor)
        check("the member gets the owner's shared track", t2["id"] in items)
        row = items.get(t2["id"], {})
        check("... as not theirs, not deletable", row.get("is_owner") is False and row.get("can_delete") is False)
        check("... with the owner's name", row.get("owner", {}).get("display_name") == owner_name)

        call("DELETE", f"/api/v1/music/tracks/{t2['id']}", owner_tok)
        _, removed, adult_cursor, _ = pull(ctx, adult_tok, adult_cursor)
        check("trash reaches the member as removed/trashed", removed.get(t2["id"], {}).get("reason") == "trashed")
        same = upload(ctx, owner_tok, f"Delta {sfx}")
        items, _, _, _ = pull(ctx, owner_tok)
        check("a new file may take the trashed file's path", items[same["id"]]["path"] == f"Music/Sync Test/Unknown Album/Delta {sfx}.mp3")
        call("POST", f"/api/v1/trash/music_track/{t2['id']}/restore", owner_tok)
        items, _, adult_cursor, _ = pull(ctx, adult_tok, adult_cursor)
        check("a restore brings it back", t2["id"] in items)
        check("... next to the newer file, as ' (2)'", items.get(t2["id"], {}).get("path") == f"Music/Sync Test/Unknown Album/Delta {sfx} (2).mp3")

        call("PUT", f"/api/v1/music/tracks/{t2['id']}/visibility", owner_tok, {"visibility": "private"})
        _, removed, adult_cursor, _ = pull(ctx, adult_tok, adult_cursor)
        check("unsharing reaches the member as removed/hidden", removed.get(t2["id"], {}).get("reason") == "hidden")

        # A transaction that commits after the feed was read is still delivered.
        log("\n[late commit]")
        slow = subprocess.Popen(
            ["docker", "compose", "exec", "-T", "postgres", "psql", "-U", "audio2", "-d", ctx.database, "-c",
             f"BEGIN; UPDATE music_tracks SET family_id = family_id WHERE id = '{same['id']}'; SELECT pg_sleep(4); COMMIT;"],
            cwd=ctx.compose_dir, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        time.sleep(1.5)
        _, _, adult_cursor, _ = pull(ctx, adult_tok, adult_cursor)
        slow.wait()
        items, _, adult_cursor, _ = pull(ctx, adult_tok, adult_cursor)
        check("a change committed after the read comes in the next round", same["id"] in items)

        # ── Paging, ids, cursors ──────────────────────────────────────────────
        log("\n[paging and ids]")
        paged, _, _, _ = pull(ctx, owner_tok, limit=2)
        whole, _, _, _ = pull(ctx, owner_tok)
        check("paging by 2 returns the same snapshot", set(paged) == set(whole) and len(whole) >= 5)
        all_ids = call("GET", "/api/v1/sync/tree/ids", owner_tok)
        check("tree/ids lists exactly the snapshot", {i["id"] for i in all_ids} == set(whole))
        adult_ids = {i["id"] for i in call("GET", "/api/v1/sync/tree/ids", adult_tok)}
        check("the member's ids hold shared items only", same["id"] in adult_ids and t2["id"] not in adult_ids)
        call("GET", "/api/v1/sync/tree?cursor=garbage", owner_tok, expect=(400,))
        check("a broken cursor is 400", True)
        ancient = base64.urlsafe_b64encode(json.dumps({"from": 1, "seq": 0, "next": 1, "at": 0}).encode()).decode().rstrip("=")
        items, _, _, reset = pull(ctx, owner_tok, ancient)
        check("a cursor past retention resets to a full snapshot", reset and set(items) == set(whole))

        # ── Family shortcuts ──────────────────────────────────────────────────
        log("\n[shortcuts]")
        s = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "music"})
        check("a member+kind shortcut is created", s["kind"] == "music" and s["container_kind"] is None
              and s["member_name"] == owner_name)
        again = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "music"}, expect=(200,))
        check("adding it again returns the same one (200)", again["id"] == s["id"])
        alb = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "music",
                   "container": {"kind": "album", "album_artist": "Sync Test", "album": "Unknown Album"}}, expect=(404,))
        check("an album with no visible tracks under that name is 404 (unknown album has no name)", True)
        call("PUT", f"/api/v1/music/tracks/{same['id']}", owner_tok, {"title": f"Delta {sfx}", "artist": "Sync Test", "album": f"Album {sfx}"})
        alb = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "music",
                   "container": {"kind": "album", "album_artist": "sync test", "album": f"album {sfx}"}})
        check("an album shortcut by name (any case)", alb["container_kind"] == "album" and alb["label"] == f"album {sfx}")
        found = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"kind": "music",
                     "container": {"kind": "album", "album_artist": "Sync Test", "album": f"Album {sfx}"}}, expect=(200,))
        check("without a member the album's owner is found", found["id"] == alb["id"] and found["member_id"] == owner_id)
        call("POST", "/api/v1/sync/shortcuts", adult_tok, {"kind": "music"}, expect=(400,))
        check("without a member or a container it is 400", True)
        if presign_ok:
            bk = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "audiobook",
                      "container": {"kind": "book", "id": book["id"]}})
            check("a book shortcut carries the book's title", bk["label"] == f"Mort {sfx}")
            bk2 = call("POST", "/api/v1/sync/shortcuts", adult_tok, {"kind": "audiobook",
                       "container": {"kind": "book", "id": book["id"]}}, expect=(200,))
            check("a book shortcut finds the book's owner", bk2["id"] == bk["id"])
            call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "music",
                 "container": {"kind": "book", "id": book["id"]}}, expect=(400,))
            check("a container that does not match the kind is 400", True)
        call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": adult_id, "kind": "music"}, expect=(400,))
        check("a shortcut to yourself is 400", True)
        call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": out_id, "kind": "music"}, expect=(404,))
        check("a shortcut to someone outside the family is 404", True)
        priv = upload(ctx, owner_tok, f"Private {sfx}", visibility="private")
        call("POST", "/api/v1/sync/shortcuts", adult_tok, {"member_id": owner_id, "kind": "music",
             "container": {"kind": "album", "album": "Unknown Album", "album_artist": "Sync Test"}}, expect=(404,))
        check("a private album of someone else cannot be targeted", True)
        listed = call("GET", "/api/v1/sync/shortcuts", adult_tok)
        check("the list holds the member's shortcuts", {x["id"] for x in listed} >= {s["id"], alb["id"]})
        check("the owner's list is separate", call("GET", "/api/v1/sync/shortcuts", owner_tok) == [])
        call("DELETE", f"/api/v1/sync/shortcuts/{alb['id']}", owner_tok, expect=(404,))
        check("someone else's shortcut cannot be removed", True)
        call("DELETE", f"/api/v1/sync/shortcuts/{alb['id']}", adult_tok)
        check("removing a shortcut", alb["id"] not in {x["id"] for x in call("GET", "/api/v1/sync/shortcuts", adult_tok)})

        # ── Device holdings ───────────────────────────────────────────────────
        log("\n[holdings]")
        call("PUT", "/api/v1/sync/holdings", owner_tok, {"items": [{"kind": "music_track", "id": t1["id"]},
                                                                    {"kind": "music_track", "id": same["id"]}]})
        held = call("GET", "/api/v1/sync/holdings", owner_tok)
        check("the device's set is stored", {h["id"] for h in held} == {t1["id"], same["id"]})
        devs = call("GET", f"/api/v1/sync/holdings?kind=music_track&id={t1['id']}", owner_tok)
        check("the item shows this device as current", len(devs) == 1 and devs[0]["current"])
        call("PUT", "/api/v1/sync/holdings", owner_tok, {"removed": [{"kind": "music_track", "id": t1["id"]}],
                                                          "added": [{"kind": "music_track", "id": old["id"]}]})
        held = {h["id"] for h in call("GET", "/api/v1/sync/holdings", owner_tok)}
        check("added/removed change the set", held == {same["id"], old["id"]})
        call("PUT", "/api/v1/sync/holdings", owner_tok, {"added": [{"kind": "playlist", "id": t1["id"]}]}, expect=(400,))
        check("an unknown kind is 400", True)
        check("another user sees none of it",
              call("GET", f"/api/v1/sync/holdings?kind=music_track&id={same['id']}", adult_tok) == [])
        call("POST", "/api/v1/auth/logout", owner_tok, expect=(200, 204))
        check("signing out drops the device's holdings",
              sql(f"SELECT count(*) FROM device_holdings WHERE user_id = '{owner_id}'") == "0")
        owner_tok = call("POST", "/api/v1/auth/login", body={"email": owner_email, "password": PW})["token"]

        # ── Podcasts: auto-store ──────────────────────────────────────────────
        log("\n[auto-store]")
        now = datetime.datetime.now(datetime.timezone.utc)
        episodes = [("old", "Old Episode", now - datetime.timedelta(days=3))]
        server = _FeedServer(ctx, episodes)
        ctx.on_cleanup("rss feed server", server.stop)
        port = server.port
        feed = call("POST", "/api/v1/podcasts/subscribe", owner_tok,
                    {"feed_url": f"http://host.docker.internal:{port}/rss", "visibility": "family"})
        check("auto-store is off by default", feed["auto_store"] is False)
        call("PUT", f"/api/v1/podcasts/{feed['id']}/auto-store", adult_tok, {"enabled": True}, expect=(403,))
        check("a member cannot switch it on someone else's show", True)
        on = call("PUT", f"/api/v1/podcasts/{feed['id']}/auto-store", owner_tok, {"enabled": True})
        check("the subscriber switches it on", on["auto_store"] is True)

        episodes.append(("new", "New Episode", datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=5)))
        call("POST", f"/api/v1/podcasts/{feed['id']}/refresh", owner_tok)
        new_id = None
        for _ in range(40):
            eps = call("GET", f"/api/v1/podcasts/{feed['id']}/episodes", owner_tok)
            eps = eps.get("episodes", eps) if isinstance(eps, dict) else eps
            new = next((e for e in eps if e["title"] == "New Episode"), None)
            if new and (new.get("has_local") or new.get("is_downloaded") or new.get("downloaded")):
                new_id = new["id"]
                break
            stored = sql(f"SELECT id FROM podcast_episodes WHERE feed_id = '{feed['id']}' AND title = 'New Episode' AND audio_object_id IS NOT NULL")
            if stored:
                new_id = stored
                break
            time.sleep(1)
        check("a new episode is stored by the server", new_id is not None)
        old_stored = sql(f"SELECT count(*) FROM podcast_episodes WHERE feed_id = '{feed['id']}' AND title = 'Old Episode' AND audio_object_id IS NOT NULL")
        check("the backlog is not", old_stored == "0")

        episodes.append(("older", "Older Episode", now - datetime.timedelta(days=4)))
        episodes.append(("oldest", "Oldest Episode", now - datetime.timedelta(days=5)))
        call("POST", f"/api/v1/podcasts/{feed['id']}/refresh", owner_tok)
        call("POST", f"/api/v1/podcasts/{feed['id']}/store-all", adult_tok, {"preview": True}, expect=(403,))
        check("a member cannot store someone else's back catalogue", True)
        pv = call("POST", f"/api/v1/podcasts/{feed['id']}/store-all", owner_tok, {"preview": True})
        check("a preview counts the episodes not stored and sizes them", pv["episodes"] == 3 and pv["estimated_bytes"] > 0)
        one = call("POST", f"/api/v1/podcasts/{feed['id']}/store-all", owner_tok, {"latest": 1})
        queued = sql(f"SELECT title FROM podcast_episodes WHERE feed_id = '{feed['id']}' AND auto_stored_at IS NOT NULL AND title <> 'New Episode'")
        check("latest: 1 queues the newest one only", one["episodes"] == 1 and queued == "Old Episode")
        rest = call("POST", f"/api/v1/podcasts/{feed['id']}/store-all", owner_tok, {})
        check("then the rest, never one twice", rest["episodes"] == 2)
        check("and nothing is left", call("POST", f"/api/v1/podcasts/{feed['id']}/store-all", owner_tok, {"preview": True})["episodes"] == 0)
        found = call("GET", f"/api/v1/podcasts/{feed['id']}/episodes?q=OLDER", owner_tok)
        check("episodes can be searched by title, any case", [e["title"] for e in found] == ["Older Episode"])
        check("a % is text, not a wildcard", call("GET", f"/api/v1/podcasts/{feed['id']}/episodes?q=%25", owner_tok) == [])
        items, _, _, _ = pull(ctx, adult_tok)
        ep = items.get(new_id, {})
        check("the stored episode is in the member's tree",
              ep.get("kind") == "podcast_episode" and ep.get("show", {}).get("id") == feed["id"])
        day = (datetime.datetime.now(datetime.timezone.utc) + datetime.timedelta(seconds=5)).strftime("%Y-%m-%d")
        check("at Podcasts/<Show>/<date> - <title>", ep.get("path") == f"Podcasts/Show - {sfx}/{day} - New Episode.mp3")

        call("DELETE", f"/api/v1/podcasts/{feed['id']}/episodes/{new_id}/download", owner_tok)
        call("DELETE", f"/api/v1/trash/podcast_episode/{new_id}", owner_tok)
        call("POST", f"/api/v1/podcasts/{feed['id']}/refresh", owner_tok)
        time.sleep(3)
        check("a deleted episode is not stored again",
              sql(f"SELECT audio_object_id IS NULL FROM podcast_episodes WHERE id = '{new_id}'") == "t")
        check("its path is gone", sql(f"SELECT count(*) FROM sync_paths WHERE item_id = '{new_id}'") == "0")

        # ── Companion files (§2 item 16) ──────────────────────────────────────
        if presign_ok:
            log("\n[companion files]")
            alb = f"Music/Companion {sfx}"
            tk = presigned(owner_tok, "music_track", "01.mp3")
            tr = call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                      {"object_key": tk, "path": f"{alb}/01 Song.mp3", "original_filename": "01 Song.mp3", "visibility": "family"})
            check("a track without art has no cover yet", tr["cover_url"] is None)

            def companion(name, content, visibility="private", expect=(201,)):
                key = presigned(owner_tok, "companion_file", name, content)
                return call("POST", "/api/v1/sync/files", owner_tok,
                            {"object_key": key, "path": f"{alb}/{name}", "visibility": visibility}, expect=expect)

            cover = companion("Cover.JPG", b"\xff\xd8cover")
            check("an image called cover becomes the album's cover", cover["used_as"] == "cover")
            t = call("GET", f"/api/v1/music/tracks/{tr['id']}", owner_tok)
            check("... on the track", t["cover_url"] is not None)
            back = companion("back.jpg", b"\xff\xd8back")
            check("a second image is kept, not used", back["used_as"] is None)
            chosen = call("POST", f"/api/v1/sync/files/{back['id']}/use-as-cover", owner_tok)
            check("the user can pick another image as the cover", chosen["tracks"] == 1)
            lrc = companion("01 Song.lrc", "[00:01.00]Hello\n".encode())
            check("a .lrc named like a track becomes its lyrics", lrc["used_as"] == "lyrics")
            lyr = call("GET", f"/api/v1/music/tracks/{tr['id']}/lyrics", owner_tok)
            check("... served as the track's lyrics", "Hello" in (lyr.get("lyrics") or ""))
            booklet = companion("booklet.pdf", b"%PDF-1.4", visibility="family")
            companion("x.zip", b"PK", expect=(400,))
            check("other types are refused", True)
            key = presigned(owner_tok, "companion_file", "p.jpg", b"img")
            call("POST", "/api/v1/sync/files", owner_tok, {"object_key": key, "path": "Podcasts/Show/p.jpg"}, expect=(400,))
            check("nothing is kept under Podcasts", True)

            items, _, _, _ = pull(ctx, owner_tok)
            check("companion files are in the owner's tree at their paths",
                  items.get(cover["id"], {}).get("path") == f"{alb}/Cover.JPG"
                  and items[cover["id"]]["kind"] == "companion_file")
            page = call("GET", "/api/v1/sync/tree?limit=1", owner_tok)
            check("the feed names the caller for Family/<Me>", page["me"]["display_name"] == owner_name)
            items, _, _, _ = pull(ctx, adult_tok)
            check("the member sees the shared booklet, not the private images",
                  booklet["id"] in items and cover["id"] not in items)
            call("PUT", f"/api/v1/sync/files/{cover['id']}/visibility", owner_tok, {"visibility": "family"})
            items, _, _, _ = pull(ctx, adult_tok)
            check("sharing an image shows it to the member", cover["id"] in items)
            stream = call("GET", f"/api/v1/sync/files/{booklet['id']}/stream", adult_tok)
            with urllib.request.urlopen(stream["url"], timeout=30) as r:
                check("its bytes download unchanged", r.read() == b"%PDF-1.4")
            call("DELETE", f"/api/v1/sync/files/{booklet['id']}", adult_tok, expect=(403,))
            check("a member cannot delete someone else's file", True)
            call("DELETE", f"/api/v1/sync/files/{booklet['id']}", owner_tok)
            trash = call("GET", "/api/v1/trash", owner_tok)
            row = next((r for r in trash if r["id"] == booklet["id"]), None)
            check("a deleted file waits in the trash under its name",
                  row is not None and row["kind"] == "companion_file" and row["title"] == "booklet.pdf")
            call("POST", f"/api/v1/trash/companion_file/{booklet['id']}/restore", owner_tok)
            items, _, _, _ = pull(ctx, owner_tok)
            check("and comes back where it was", items.get(booklet["id"], {}).get("path") == f"{alb}/booklet.pdf")

            bk1 = presigned(owner_tok, "audiobook_file", "c1.mp3")
            bookc = call("POST", "/api/v1/audiobooks/from-uploads", owner_tok, {
                "title": "Covered", "path": f"Audiobooks/Cov {sfx}/Covered",
                "files": [{"object_key": bk1, "relative_path": "01.mp3"}]})
            key = presigned(owner_tok, "companion_file", "front.png", b"\x89PNGfront")
            fr = call("POST", "/api/v1/sync/files", owner_tok,
                      {"object_key": key, "path": f"Audiobooks/Cov {sfx}/Covered/front.png"})
            b = call("GET", f"/api/v1/audiobooks/{bookc['id']}", owner_tok)
            check("an image in a book folder becomes the book's cover", fr["used_as"] == "cover" and b["cover_url"] is not None)

        # ── Organise: default paths on request ─────────────────────────────
        if presign_ok:
            log("\n[organise]")
            items, _, _, _ = pull(ctx, owner_tok)
            before = {i: v["path"] for i, v in items.items()}
            preview = call("POST", "/api/v1/sync/paths/organise", owner_tok, {"kind": "music", "preview": True})
            mv = {m["id"]: m for m in preview}
            check("a preview lists the album track's move", tr["id"] in mv and mv[tr["id"]]["from"] == f"{alb}/01 Song.mp3"
                  and mv[tr["id"]]["to"].startswith("Music/") and mv[tr["id"]]["to"] != mv[tr["id"]]["from"])
            check("... with the album's companion files going along", mv.get(tr["id"], {}).get("companions", 0) >= 2)
            items, _, _, _ = pull(ctx, owner_tok)
            check("a preview changes nothing", {i: v["path"] for i, v in items.items()} == before)
            applied = call("POST", "/api/v1/sync/paths/organise", owner_tok, {"kind": "music", "ids": [tr["id"]]})
            check("apply moves only the chosen items", [m["id"] for m in applied] == [tr["id"]]
                  and applied[0]["to"] == mv[tr["id"]]["to"])
            items, _, _, _ = pull(ctx, owner_tok)
            new_dir = applied[0]["to"].rsplit("/", 1)[0]
            check("the track is at its new path, same id", items[tr["id"]]["path"] == applied[0]["to"])
            moved_companions = [v for v in items.values() if v.get("kind") == "companion_file" and v["path"].startswith(new_dir + "/")]
            check("its companions moved into the new folder", len(moved_companions) >= 2)
            others = [i for i in mv if i != tr["id"]]
            check("other tracks stayed", bool(others) and all(items[i]["path"] == before[i] for i in others))
            again = call("POST", "/api/v1/sync/paths/organise", owner_tok, {"kind": "music", "preview": True})
            check("organised items drop out of the next preview", tr["id"] not in {m["id"] for m in again})
            books = call("POST", "/api/v1/sync/paths/organise", owner_tok, {"kind": "audiobook"})
            bm = {m["id"]: m for m in books}
            check("a book goes to Audiobooks/<Author>/<Title>", bm.get(book["id"], {}).get("to") == f"Audiobooks/Pratchett/Mort {sfx}")
            items, _, _, _ = pull(ctx, owner_tok)
            files = [f["relative_path"] for f in items.get(book["id"], {}).get("files", [])]
            check("... its files keep their names", files == ["CD1/00 Intro.mp3", "CD1/01 One.mp3", "CD1/02 Two.mp3", "CD2/09 Nine.mp3"])
            box = f"Music/Box {sfx}"
            bk_ = presigned(owner_tok, "music_track", "01 a.mp3")
            bt = call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                      {"object_key": bk_, "path": f"{box}/CD 01/01 a.mp3", "original_filename": "01 a.mp3"})
            ck = presigned(owner_tok, "companion_file", "cover.jpg", b"\xff\xd8box")
            call("POST", "/api/v1/sync/files", owner_tok, {"object_key": ck, "path": f"{box}/cover.jpg"})
            boxed = call("POST", "/api/v1/sync/paths/organise", owner_tok, {"kind": "music", "ids": [bt["id"]]})
            items, _, _, _ = pull(ctx, owner_tok)
            box_dir = boxed[0]["to"].rsplit("/", 1)[0] if boxed else ""
            check("an album's cover follows tracks that sat in CD subfolders",
                  any(v.get("kind") == "companion_file" and v["path"].startswith(f"{box_dir}/cover") for v in items.values()))
            dk = presigned(owner_tok, "music_track", "01 d.mp3")
            dt = call("POST", "/api/v1/music/tracks/from-upload", owner_tok,
                      {"object_key": dk, "path": f"Music/Discs {sfx}/CD 2/01 d.mp3", "original_filename": "01 d.mp3"})
            t = call("GET", f"/api/v1/music/tracks/{dt['id']}", owner_tok)
            check("a file in a 'CD 2' folder without a disc tag is on disc 2", t.get("disc_number") == 2)
            dm = {m["id"]: m for m in call("POST", "/api/v1/sync/paths/organise", owner_tok, {"kind": "music", "preview": True})}
            check("organise puts a multi-disc track into its 'CD <n>' folder", "/CD 2/" in dm.get(dt["id"], {}).get("to", ""))
            discs = call("POST", "/api/v1/music/tracks/discs", owner_tok)
            check("reading disc numbers reports what is left", "left" in discs and "checked" in discs)
            other = call("POST", "/api/v1/sync/paths/organise", adult_tok, {"kind": "music", "preview": True})
            check("nobody organises someone else's items", tr["id"] not in {m["id"] for m in other})

        # ── A second session for a helper process ────────────────────────────
        log("\n[fork]")
        login = call("POST", "/api/v1/auth/login", body={"email": out_email, "password": PW, "device_name": "app"})
        forked = call("POST", "/api/v1/auth/fork", body={"refresh_token": login["refresh_token"], "device_name": "own.audio Finder", "device_kind": "macos"})
        check("a fork answers with its own pair", forked["refresh_token"] != login["refresh_token"] and forked["user"]["id"] == out_id)
        a = call("POST", "/api/v1/auth/refresh", body={"refresh_token": login["refresh_token"]})
        b = call("POST", "/api/v1/auth/refresh", body={"refresh_token": forked["refresh_token"]})
        check("both refresh on their own", bool(a["token"]) and bool(b["token"]))
        names = {s["device_name"] for s in call("GET", "/api/v1/auth/sessions", b["token"])}
        check("the helper shows as its own device", {"app", "own.audio Finder"} <= names)
        call("POST", "/api/v1/auth/fork", body={"refresh_token": login["refresh_token"]}, expect=(401,))
        check("forking with a rotated token is refused", True)
        call("POST", "/api/v1/auth/refresh", body={"refresh_token": a["refresh_token"]}, expect=(401,))
        check("... and ends the chain it came from, as reuse does", True)
        call("POST", "/api/v1/auth/refresh", body={"refresh_token": b["refresh_token"]})
        check("the fork lives on", True)

        # ── Leaving the family ends shortcuts ─────────────────────────────────
        log("\n[leaving]")
        sql(f"DELETE FROM family_members WHERE user_id = '{adult_id}'")
        check("a member who leaves loses their shortcuts",
              sql(f"SELECT count(*) FROM sync_shortcuts WHERE user_id = '{adult_id}'") == "0")

    finally:
        # Users are removed by ctx.make_user's cleanup; the feed server must stop
        # here too so a crash mid-suite never leaves a listening thread behind.
        if server:
            server.stop()
