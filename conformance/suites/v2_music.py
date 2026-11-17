# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 music remainder (completion plan phase 6): track feedback as the listener's state, lyrics written by hand, a file's own tags and reading them again, the owner's duplicates, a sentence to a smart-playlist rule, track and playlist covers, keeping a generated playlist, playlists and smart playlists in the change feed.

Each route at each permission level: the owner, another member (403 on what is shared, 404 on
what is private), another household on a server with several. Also: feedback round-trips and
reaches only its listener's `/sync/changes` as `feedback.track`, and its tombstone when taken
back; a playlist's edits, its covers, a track it holds being shared, its trip through the trash
and its smart sibling all reach `/sync/changes`, a deleted one as a tombstone; snapshot kinds
`playlist`, `smart_playlist`, `feedback`; the duplicates list finds the suite's two copies of
one file and pages without loss; lyrics write then read; a rescan reads the file's tags back
(and refuses an identified track, with `--compose-dir`); an unreadable sentence is `422
not_understood`; every refusal is problem details.
"""
from __future__ import annotations

import json
import struct
import urllib.parse

from core import Ctx
from suites.v2_books import Feed, _last, _png, _problem
from suites.v2_sync import _upload

NAME = "v2_music"
PW = "V2MusicTest123!"


def _tagged_wav(title: str, artist: str, album: str) -> bytes:
    """The suite's tone with a RIFF INFO list: a file that carries its own tags."""
    tone = Ctx.tone()

    def sub(key: bytes, value: str) -> bytes:
        data = value.encode() + b"\x00"
        if len(data) % 2:
            data += b"\x00"
        return key + struct.pack("<I", len(data)) + data

    info = b"INFO" + sub(b"INAM", title) + sub(b"IART", artist) + sub(b"IPRD", album)
    body = tone[12:] + b"LIST" + struct.pack("<I", len(info)) + info
    return b"RIFF" + struct.pack("<I", 4 + len(body)) + b"WAVE" + body


class Suite:
    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx
        self.n = 0

    def track(self, tok: str, visibility: str = "family", content: bytes | None = None, title: str | None = None) -> dict:
        self.n += 1
        sfx = self.ctx.sfx
        if content is None:
            upload_id = _upload(self.ctx, tok, "track", f"m{self.n}.wav")
        else:
            made = self.ctx.call("POST", "/api/v2/uploads", tok, {"kind": "track", "filename": f"m{self.n}.wav",
                                                                  "content_type": "audio/wav", "size_bytes": len(content)}, expect=(201,))
            self.ctx.put_bytes(made["url"], content, made["headers"]["Content-Type"])
            upload_id = made["upload_id"]
        return self.ctx.call("POST", "/api/v2/tracks", tok, {
            "upload_id": upload_id, "filename": f"m{self.n}.wav", "title": title or f"Music Song {sfx} {self.n}",
            "artist": f"Music Band {sfx}", "album": f"Music Album {sfx} {self.n}", "visibility": visibility}, expect=(201,))

    def image(self, tok: str, shade: int = 0, content_type: str = "image/png") -> str:
        content = _png(shade) if content_type.startswith("image/") else b"not a picture"
        made = self.ctx.call("POST", "/api/v2/uploads", tok, {"kind": "track_cover", "filename": "cover.png" if content_type.startswith("image/") else "x.txt",
                                                              "content_type": content_type, "size_bytes": len(content)}, expect=(201,))
        self.ctx.put_bytes(made["url"], content, made["headers"]["Content-Type"])
        return made["upload_id"]

    def walk(self, path: str, tok: str, limit: int) -> tuple[list, int, dict]:
        items, cursor, pages, included = [], None, 0, {}
        while True:
            q = {"limit": str(limit)}
            if cursor:
                q["cursor"] = cursor
            sep = "&" if "?" in path else "?"
            page = self.ctx.call("GET", f"{path}{sep}{urllib.parse.urlencode(q)}", tok)
            items += page["items"]
            for k, v in (page.get("included") or {}).items():
                included.setdefault(k, []).extend(v)
            cursor, pages = page.get("next"), pages + 1
            if not cursor or pages > 500:
                return items, pages, included


def run(ctx: Ctx) -> None:
    s = Suite(ctx)
    one_household = ctx.feature("one_family")
    _owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    _member_id, member_email, member = ctx.make_user("member", PW)
    if not one_household:
        inv = ctx.call("POST", "/api/v1/family/invites", owner, {"email": member_email, "role": "member"})
        ctx.call("POST", "/api/v1/family/invites/accept", member, {"code": inv["code"]})
    outsider = None
    if not one_household:
        _xid, _xe, outsider = ctx.make_family_admin("outsider", PW)

    shared, private = s.track(owner), s.track(owner, visibility="private")
    feedback(ctx, s, owner, member, outsider, shared, private)
    lyrics(ctx, owner, member, outsider, shared, private)
    file_tags(ctx, s, owner, member)
    duplicates(ctx, s, owner, member, outsider)
    intent(ctx, owner, member)
    covers(ctx, s, owner, member, shared, private)
    playlists(ctx, s, owner, member, outsider, shared)
    smart(ctx, owner, member)
    snapshot(ctx, owner, member, shared)


def feedback(ctx: Ctx, s: Suite, owner: str, member: str, outsider: str | None, shared: dict, private: dict) -> None:
    check = ctx.check
    mine = Feed(ctx, member, "feedback")
    theirs = Feed(ctx, owner, "feedback")
    tid = shared["id"]
    check("PUT /me/feedback/track/{id} (dislike) → 204",
          ctx.status_of("PUT", f"/api/v2/me/feedback/track/{tid}", member, {"value": "dislike"}) == 204)
    listed = ctx.call("GET", "/api/v2/me/feedback", member)["items"]
    check("GET /me/feedback lists it, `dislike`, with when",
          any(i["id"] == tid and i["kind"] == "track" and i["value"] == "dislike" and i.get("updated_at") for i in listed))
    rows = mine.read()
    row = _last(rows, "feedback.track", tid)
    check("…and it reaches the listener's /sync/changes as `feedback.track`",
          row is not None and row["op"] == "upsert" and row["record"]["value"] == "dislike" and row["record"]["id"] == tid)
    check("…as their state only: the change feed of the track's owner does not carry it",
          _last(theirs.read(), "feedback.track", tid) is None)
    check("…nor their /me/feedback", all(i["id"] != tid for i in ctx.call("GET", "/api/v2/me/feedback", owner)["items"]))

    ctx.call("PUT", f"/api/v2/me/feedback/track/{tid}", member, {"value": "ban"}, expect=(204,))
    row = _last(mine.read(), "feedback.track", tid)
    check("a ban replaces the dislike, in the feed too", row is not None and row["record"]["value"] == "ban")
    bans = ctx.call("GET", "/api/v2/me/feedback?value=ban", member)["items"]
    dislikes = ctx.call("GET", "/api/v2/me/feedback?value=dislike", member)["items"]
    check("GET /me/feedback?value= keeps one of the two", any(i["id"] == tid for i in bans) and all(i["id"] != tid for i in dislikes))

    check("DELETE /me/feedback/track/{id} → 204", ctx.status_of("DELETE", f"/api/v2/me/feedback/track/{tid}", member) == 204)
    check("…and it is gone from the list", all(i["id"] != tid for i in ctx.call("GET", "/api/v2/me/feedback", member)["items"]))
    row = _last(mine.read(), "feedback.track", tid)
    check("…and a tombstone in the feed", row is not None and row["op"] == "delete")
    check("taking back what was never given is 204 too", ctx.status_of("DELETE", f"/api/v2/me/feedback/track/{tid}", member) == 204)

    check("feedback on a private track of someone else's → 404",
          _problem(ctx.request("PUT", f"/api/v2/me/feedback/track/{private['id']}", member, {"value": "dislike"}), 404))
    check("feedback on a book → 400 unsupported_kind",
          _problem(ctx.request("PUT", f"/api/v2/me/feedback/book/{tid}", member, {"value": "dislike"}), 400, "unsupported_kind"))
    check("a value other than dislike or ban → 400 invalid_value",
          _problem(ctx.request("PUT", f"/api/v2/me/feedback/track/{tid}", member, {"value": "meh"}), 400, "invalid_value")
          and _problem(ctx.request("GET", "/api/v2/me/feedback?value=meh", member), 400, "invalid_value"))
    check("no token → 401", _problem(ctx.request("GET", "/api/v2/me/feedback"), 401))
    if outsider:
        check("another household: feedback on the shared track → 404",
              _problem(ctx.request("PUT", f"/api/v2/me/feedback/track/{tid}", outsider, {"value": "dislike"}), 404))
    else:
        ctx.skip("another household's feedback", "one household on this server")

    # Paging: three of them, one at a time against one page.
    tracks = [s.track(owner) for _ in range(3)]
    for t in tracks:
        ctx.call("PUT", f"/api/v2/me/feedback/track/{t['id']}", member, {"value": "dislike"}, expect=(204,))
    full, _p, _i = s.walk("/api/v2/me/feedback", member, 500)
    small, pages, _i = s.walk("/api/v2/me/feedback", member, 1)
    check("GET /me/feedback pages without loss or repeats, most recent first",
          [i["id"] for i in small] == [i["id"] for i in full] and pages >= 3 and len({i["id"] for i in small}) == len(small)
          and [i["id"] for i in full[:3]] == [t["id"] for t in reversed(tracks)])
    for t in tracks:
        ctx.call("DELETE", f"/api/v2/me/feedback/track/{t['id']}", member, expect=(204,))


def lyrics(ctx: Ctx, owner: str, member: str, outsider: str | None, shared: dict, private: dict) -> None:
    check = ctx.check
    tid = shared["id"]
    put = ctx.call("PUT", f"/api/v2/tracks/{tid}/lyrics", owner, {"text": "First line\nSecond line"})
    got = ctx.call("GET", f"/api/v2/tracks/{tid}/lyrics", member)
    check("PUT /tracks/{id}/lyrics (text) → the lyrics, and another member reads them back",
          put["text"] == "First line\nSecond line" and not put["synced"]
          and got["text"] == put["text"] and [l["text"] for l in got["lines"]] == ["First line", "Second line"])
    put = ctx.call("PUT", f"/api/v2/tracks/{tid}/lyrics", owner,
                   {"lines": [{"start_ms": 1500, "text": "One"}, {"start_ms": 63250, "text": "Two"}]})
    got = ctx.call("GET", f"/api/v2/tracks/{tid}/lyrics", owner)
    check("…as timed lines: kept as LRC, read back with their times",
          put["synced"] and got["synced"] and [l.get("start_ms") for l in got["lines"]] == [1500, 63250]
          and got["text"].startswith("[00:01.50]One"))
    cleared = ctx.call("PUT", f"/api/v2/tracks/{tid}/lyrics", owner, {"text": None})
    got = ctx.call("GET", f"/api/v2/tracks/{tid}/lyrics", owner)
    check("…and null takes them away: the file's own are read again (the tone has none)",
          cleared["text"] is None and got["text"] is None and got["lines"] == [])
    check("both `text` and `lines` → 400 invalid_body",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{tid}/lyrics", owner, {"text": "x", "lines": []}), 400, "invalid_body"))
    check("lines that go back in time → 400 invalid_body",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{tid}/lyrics", owner,
                               {"lines": [{"start_ms": 5000, "text": "a"}, {"start_ms": 1000, "text": "b"}]}), 400, "invalid_body"))
    check("another member, on a shared track → 403",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{tid}/lyrics", member, {"text": "mine"}), 403))
    check("…on a private one → 404",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{private['id']}/lyrics", member, {"text": "mine"}), 404))
    if outsider:
        check("another household → 404", _problem(ctx.request("PUT", f"/api/v2/tracks/{tid}/lyrics", outsider, {"text": "x"}), 404))


def file_tags(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    check = ctx.check
    sfx = ctx.sfx
    tagged = _tagged_wav(f"Tagged Title {sfx}", f"Tagged Artist {sfx}", f"Tagged Album {sfx}")
    made = s.track(owner, content=tagged, title=f"Typed Title {sfx}")
    tid = made["id"]
    check("POST /tracks with a title of its own keeps it over the file's", made["title"] == f"Typed Title {sfx}")
    tags = ctx.call("GET", f"/api/v2/tracks/{tid}/file-tags", owner)
    check("GET /tracks/{id}/file-tags: what the file says, not what the library says",
          tags["title"] == f"Tagged Title {sfx}" and tags["artist"] == f"Tagged Artist {sfx}"
          and tags["album"] == f"Tagged Album {sfx}" and tags["has_cover"] is False and str(tags.get("file_name") or "") != "")
    check("…the owner's: another member → 403", _problem(ctx.request("GET", f"/api/v2/tracks/{tid}/file-tags", member), 403))

    again = ctx.call("POST", f"/api/v2/tracks/{tid}/rescan", owner)
    check("POST /tracks/{id}/rescan → the track, with the file's title read back", again["title"] == f"Tagged Title {sfx}")
    check("…and GET /tracks/{id} says so", ctx.call("GET", f"/api/v2/tracks/{tid}", owner)["title"] == f"Tagged Title {sfx}")
    check("…the owner's: another member → 403", _problem(ctx.request("POST", f"/api/v2/tracks/{tid}/rescan", member), 403))
    plain = s.track(owner)
    check("a file without tags → 422 no_tags", _problem(ctx.request("POST", f"/api/v2/tracks/{plain['id']}/rescan", owner), 422, "no_tags"))
    if ctx.db_available:
        ctx.sql(f"INSERT INTO music_track_identity (track_id, source) VALUES ('{tid}', 'identify')")
        try:
            check("a track identified here → 409 identified (MusicBrainz is the better source)",
                  _problem(ctx.request("POST", f"/api/v2/tracks/{tid}/rescan", owner), 409, "identified"))
        finally:
            ctx.sql(f"DELETE FROM music_track_identity WHERE track_id = '{tid}'")
    else:
        ctx.skip("rescan of an identified track", "needs --compose-dir to mark one identified")
    s.tagged = tagged  # type: ignore[attr-defined]
    s.tagged_id = tid  # type: ignore[attr-defined]


def duplicates(ctx: Ctx, s: Suite, owner: str, member: str, outsider: str | None) -> None:
    check = ctx.check
    # Two copies of the tone and two of the tagged file: two groups to page through.
    tone_a, tone_b = s.track(owner), s.track(owner)
    tagged_b = s.track(owner, content=s.tagged)  # type: ignore[attr-defined]
    ids = [tone_a["id"], tone_b["id"], s.tagged_id, tagged_b["id"]]  # type: ignore[attr-defined]

    def read() -> bool:
        return all(ctx.call("GET", f"/api/v2/tracks/{i}", owner).get("sha256") for i in ids)

    if not ctx.wait_until(read, timeout=120, every=1.0):
        ctx.fail("duplicates", "the server did not read the suite's files' checksums in two minutes")
        return
    full, _pages, included = s.walk("/api/v2/tracks/duplicates?include=tracks", owner, 500)
    groups = {g["key"]: g for g in full}
    tone_group = next((g for g in full if tone_a["id"] in g["track_ids"]), None)
    tagged_group = next((g for g in full if tagged_b["id"] in g["track_ids"]), None)
    check("GET /tracks/duplicates finds the two copies of one file as one group",
          tone_group is not None and tone_b["id"] in tone_group["track_ids"] and tone_group["kind"] == "identical"
          and tagged_group is not None and s.tagged_id in tagged_group["track_ids"] and len(groups) == len(full))  # type: ignore[attr-defined]
    sha = ctx.call("GET", f"/api/v2/tracks/{tone_a['id']}", owner)["sha256"]
    check("…keyed by the file's SHA-256, oldest first", tone_group is not None and tone_group["key"] == sha
          and tone_group["track_ids"].index(tone_a["id"]) < tone_group["track_ids"].index(tone_b["id"]))
    embedded = {t["id"] for t in included.get("tracks", [])}
    check("…include=tracks embeds them once", set(ids) <= embedded and len(embedded) == len(included.get("tracks", [])))
    small, pages, _i = s.walk("/api/v2/tracks/duplicates", owner, 1)
    check("…and pages without loss or repeats", [g["key"] for g in small] == [g["key"] for g in full] and pages >= 2)
    mine = ctx.call("GET", "/api/v2/tracks/duplicates?limit=500", member)["items"]
    check("another member's list never holds the owner's tracks",
          all(not set(g["track_ids"]) & set(ids) for g in mine))
    if outsider:
        theirs = ctx.call("GET", "/api/v2/tracks/duplicates?limit=500", outsider)["items"]
        check("…nor another household's", all(not set(g["track_ids"]) & set(ids) for g in theirs))
    check("include= of something else → 400 invalid_include",
          _problem(ctx.request("GET", "/api/v2/tracks/duplicates?include=albums", owner), 400, "invalid_include"))
    check("a cursor it did not hand out → 400 invalid_cursor",
          _problem(ctx.request("GET", "/api/v2/tracks/duplicates?cursor=nope", owner), 400, "invalid_cursor"))


def intent(ctx: Ctx, owner: str, member: str) -> None:
    check = ctx.check
    got = ctx.call("POST", "/api/v2/smart-playlists/intent", member, {"text": "an hour for my gym session"})
    rule = got.get("rule") or {}
    check("POST /smart-playlists/intent: a sentence becomes a rule", isinstance(rule, dict) and "limit" in rule)
    tried = ctx.request("POST", "/api/v2/smart-playlists/resolve", member, {"rule": rule})
    check("…that resolves as it is", tried[0] == 200)
    check("a sentence that names no music → 422 not_understood",
          _problem(ctx.request("POST", "/api/v2/smart-playlists/intent", owner, {"text": "hello"}), 422, "not_understood"))
    check("an empty one → 400 invalid_body",
          _problem(ctx.request("POST", "/api/v2/smart-playlists/intent", owner, {"text": "  "}), 400, "invalid_body"))
    check("over 500 characters → 400 invalid_body",
          _problem(ctx.request("POST", "/api/v2/smart-playlists/intent", owner, {"text": "jazz " * 101}), 400, "invalid_body"))
    check("no token → 401", _problem(ctx.request("POST", "/api/v2/smart-playlists/intent", None, {"text": "jazz"}), 401))


def covers(ctx: Ctx, s: Suite, owner: str, member: str, shared: dict, private: dict) -> None:
    check = ctx.check
    tid = shared["id"]
    before = ctx.call("GET", f"/api/v2/tracks/{tid}", owner).get("image")
    put = ctx.call("PUT", f"/api/v2/tracks/{tid}/image", owner, {"upload_id": s.image(owner, 10)})
    check("PUT /tracks/{id}/image → the track with its new cover", put.get("image") and put["image"] != before)
    check("…a picture the cover link serves", ctx.probe_media(put["image"], expect=(200, 206)) in (200, 206))
    check("not an image → 400 not_an_image",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{tid}/image", owner, {"upload_id": s.image(owner, content_type="text/plain")}), 400, "not_an_image"))
    check("another member, on a shared track → 403",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{tid}/image", member, {"upload_id": s.image(member, 20)}), 403))
    check("…on a private one → 404",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{private['id']}/image", member, {"upload_id": s.image(member, 30)}), 404))
    check("DELETE /tracks/{id}/image → 204, and the track has none",
          ctx.status_of("DELETE", f"/api/v2/tracks/{tid}/image", owner) == 204
          and ctx.call("GET", f"/api/v2/tracks/{tid}", owner).get("image") is None)
    check("…another member → 403", _problem(ctx.request("DELETE", f"/api/v2/tracks/{tid}/image", member), 403))
    edits = ctx.call("GET", f"/api/v2/edits?item_kind=track&item_id={tid}&limit=50", owner)["items"]
    check("…both are changes in the edit history, to undo", sum("cover_object_id" in json.dumps(e) for e in edits) >= 2)


def playlists(ctx: Ctx, s: Suite, owner: str, member: str, outsider: str | None, shared: dict) -> None:
    check = ctx.check
    sfx = ctx.sfx
    feed_o = Feed(ctx, owner, "playlist")
    feed_m = Feed(ctx, member, "playlist")
    later = s.track(owner, visibility="private")
    pl = ctx.call("POST", "/api/v2/playlists", owner, {"name": f"Music List {sfx}", "visibility": "family",
                                                       "track_ids": [shared["id"], later["id"]]}, expect=(201,))
    pid = pl["id"]
    check("the record says whether it was generated and kept", pl.get("generated") is False and pl.get("kept") is False)
    row_o, row_m = _last(feed_o.read(), "playlist", pid), _last(feed_m.read(), "playlist", pid)
    check("a new playlist reaches /sync/changes (kind `playlist`), counted as each reader sees it",
          row_o is not None and row_o["op"] == "upsert" and row_o["record"]["track_count"] == 2 and "updated_at" in row_o["record"]
          and row_m is not None and row_m["op"] == "upsert" and row_m["record"]["track_count"] == 1)

    ctx.call("PATCH", f"/api/v2/playlists/{pid}", owner, {"name": f"Music List {sfx} renamed"})
    row = _last(feed_m.read(), "playlist", pid)
    check("a rename arrives as a change of the playlist", row is not None and row["record"]["name"] == f"Music List {sfx} renamed")
    ctx.call("POST", f"/api/v2/playlists/{pid}/entries", owner, {"track_ids": [shared["id"]]})
    row = _last(feed_o.read(), "playlist", pid)
    check("…so does an added entry", row is not None and row["record"]["track_count"] == 3)
    ctx.call("PUT", f"/api/v2/tracks/{later['id']}/visibility", owner, {"visibility": "family"})
    row = _last(feed_m.read(), "playlist", pid)
    check("a track it holds, shared: the playlist again, with what the reader can now see",
          row is not None and row["op"] == "upsert" and row["record"]["track_count"] == 3)

    put = ctx.call("PUT", f"/api/v2/playlists/{pid}/image", owner, {"upload_id": s.image(owner, 40)})
    row = _last(feed_m.read(), "playlist", pid)
    check("PUT /playlists/{id}/image → the playlist with its cover, in the feed too",
          put.get("image") and row is not None and row["record"]["image"] == put["image"])
    check("…another member → 403",
          _problem(ctx.request("PUT", f"/api/v2/playlists/{pid}/image", member, {"upload_id": s.image(member, 50)}), 403))
    check("DELETE /playlists/{id}/image → 204, its own cover gone",
          ctx.status_of("DELETE", f"/api/v2/playlists/{pid}/image", owner) == 204
          and ctx.call("GET", f"/api/v2/playlists/{pid}", owner).get("image") != put["image"])

    ctx.call("PATCH", f"/api/v2/playlists/{pid}", owner, {"visibility": "private"})
    row = _last(feed_m.read(), "playlist", pid)
    check("made private: a removal for the member", row is not None and row["op"] == "delete")
    ctx.call("PATCH", f"/api/v2/playlists/{pid}", owner, {"visibility": "family"})
    feed_m.read()

    ctx.call("DELETE", f"/api/v2/playlists/{pid}", owner, expect=(204,))
    row_o, row_m = _last(feed_o.read(), "playlist", pid), _last(feed_m.read(), "playlist", pid)
    check("a deleted playlist (into the trash) is a tombstone, for everyone who saw it",
          row_o is not None and row_o["op"] == "delete" and row_m is not None and row_m["op"] == "delete")
    ctx.call("POST", f"/api/v2/trash/playlist/{pid}/restore", owner, expect=(204,))
    row = _last(feed_m.read(), "playlist", pid)
    check("…and restored from the trash, back again", row is not None and row["op"] == "upsert")

    check("PATCH kept on a playlist made by hand → 409 not_generated",
          _problem(ctx.request("PATCH", f"/api/v2/playlists/{pid}", owner, {"kept": True}), 409, "not_generated"))
    made = ctx.call("POST", "/api/v1/music/playlists", owner, {"name": f"Music Mix {sfx}", "generated": True,
                                                               "track_ids": [shared["id"]]}, expect=(200, 201))
    gid = made["id"]
    kept = ctx.call("PATCH", f"/api/v2/playlists/{gid}", owner, {"kept": True})
    row = _last(feed_o.read(), "playlist", gid)
    check("PATCH /playlists/{id} {kept: true} keeps a generated one, in the feed too",
          kept["generated"] is True and kept["kept"] is True and row is not None and row["record"]["kept"] is True)
    check("…and v1 says it is kept", bool(ctx.call("GET", f"/api/v1/music/playlists/{gid}", owner).get("kept_at")))
    let_go = ctx.call("PATCH", f"/api/v2/playlists/{gid}", owner, {"kept": False})
    check("{kept: false} lets it go again", let_go["kept"] is False)
    check("…the owner's: another member → 404 (it is private)",
          _problem(ctx.request("PATCH", f"/api/v2/playlists/{gid}", member, {"kept": True}), 404))
    if outsider:
        check("another household's admin: the playlist's routes → 404",
              _problem(ctx.request("PUT", f"/api/v2/playlists/{pid}/image", outsider, {"upload_id": "x"}), 404))
    ctx.call("DELETE", f"/api/v2/playlists/{gid}", owner, expect=(204,))


def smart(ctx: Ctx, owner: str, member: str) -> None:
    check = ctx.check
    sfx = ctx.sfx
    feed_o = Feed(ctx, owner, "smart_playlist")
    feed_m = Feed(ctx, member, "smart_playlist")
    rule = {"filters": {"genres": {"include": [f"musicgenre{sfx}"]}}, "limit": {"kind": "count", "n": 10},
            "sequence": {"max_per_artist": 10, "no_consecutive_artist": False}}
    sp = ctx.call("POST", "/api/v2/smart-playlists", owner, {"name": f"Music Smart {sfx}", "rule": rule}, expect=(201,))
    row = _last(feed_o.read(), "smart_playlist", sp["id"])
    check("a smart playlist reaches /sync/changes (kind `smart_playlist`)",
          row is not None and row["op"] == "upsert" and row["record"]["name"] == f"Music Smart {sfx}")
    row = _last(feed_m.read(), "smart_playlist", sp["id"])
    check("…a private one not the member's", row is None or row["op"] == "delete")
    ctx.call("PATCH", f"/api/v2/smart-playlists/{sp['id']}", owner, {"visibility": "family"})
    row = _last(feed_m.read(), "smart_playlist", sp["id"])
    check("shared: the member gets it", row is not None and row["op"] == "upsert" and row["record"]["visibility"] == "family")
    ctx.call("DELETE", f"/api/v2/smart-playlists/{sp['id']}", owner, expect=(204,))
    row = _last(feed_m.read(), "smart_playlist", sp["id"])
    check("deleted: a tombstone", row is not None and row["op"] == "delete")


def snapshot(ctx: Ctx, owner: str, member: str, shared: dict) -> None:
    check = ctx.check
    sfx = ctx.sfx
    pl = ctx.call("POST", "/api/v2/playlists", owner, {"name": f"Snap List {sfx}", "track_ids": [shared["id"]]}, expect=(201,))
    rule = {"filters": {"genres": {"include": [f"snapgenre{sfx}"]}}, "limit": {"kind": "count", "n": 5}}
    sp = ctx.call("POST", "/api/v2/smart-playlists", owner, {"name": f"Snap Smart {sfx}", "rule": rule}, expect=(201,))
    ctx.call("PUT", f"/api/v2/me/feedback/track/{shared['id']}", owner, {"value": "ban"}, expect=(204,))

    def lines(tok: str, query: str) -> list[dict]:
        raw = ctx.call("GET", f"/api/v2/sync/snapshot{query}", tok, raw=True)
        return [json.loads(x) for x in raw.decode().splitlines() if x.strip()]

    picked = lines(owner, "?kinds=playlist,smart_playlist,feedback")
    recs = {(x["kind"], x["id"]): x["record"] for x in picked if x["type"] == "record"}
    check("GET /sync/snapshot?kinds=playlist,smart_playlist,feedback: the caller's, as their records",
          recs.get(("playlist", pl["id"]), {}).get("track_count") == 1 and "updated_at" in recs.get(("playlist", pl["id"]), {})
          and ("smart_playlist", sp["id"]) in recs
          and recs.get(("feedback.track", shared["id"]), {}).get("value") == "ban"
          and {k for k, _ in recs} <= {"playlist", "smart_playlist", "feedback.track"})
    ids = {(x["kind"], x["id"]) for x in lines(owner, "?kinds=playlist,smart_playlist,feedback&fields=ids") if x["type"] == "id"}
    check("…ids only: the same items", ids == set(recs))
    full = {(x.get("kind"), x.get("id")) for x in lines(owner, "") if x["type"] == "record"}
    check("…and in a snapshot of every kind", ("playlist", pl["id"]) in full and ("smart_playlist", sp["id"]) in full
          and ("feedback.track", shared["id"]) in full)
    theirs = {(x.get("kind"), x.get("id")) for x in lines(member, "?kinds=playlist,smart_playlist,feedback") if x["type"] == "record"}
    check("another member's snapshot holds neither the private playlists nor the owner's feedback",
          ("playlist", pl["id"]) not in theirs and ("smart_playlist", sp["id"]) not in theirs and ("feedback.track", shared["id"]) not in theirs)
    ctx.call("DELETE", f"/api/v2/me/feedback/track/{shared['id']}", owner, expect=(204,))
