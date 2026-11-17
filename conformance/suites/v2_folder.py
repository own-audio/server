# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 file sync remainder (completion plan phase 11): companion files, holdings, shortcuts, organise, an uploaded show in the trash.

What the v1 `filesync` suite proves, on v2. Companion files: made from an upload by path or next
to an item (`attach_to`), the cover and lyrics rules, refused types and places, `Idempotency-Key`;
kind `file` in the snapshot (only when asked) and the change feed — made, shared, moved, trashed
and restored; downloaded through `POST /playback-info` (always the original); an image made a
track's cover with `PUT /tracks/{id}/image {file_id}`; deleted into the trash. A device's
holdings (the whole set, added and removed, which devices hold an item, paging). Shortcuts by
`album_id`, a book or a member's whole kind, in the listener's change feed, and still what v1's
sync clients read. Organise: a dry run lists exactly the moves the same request then makes, and
the files beside a track go along. An uploaded show deleted into the trash with its episodes,
restored, a new show of its guid meanwhile, and purged with its stored objects (SQL); a followed
show (a feed served from this machine) is still unfollowed outright.
"""
from __future__ import annotations

import json
import urllib.parse
import urllib.request
import uuid

from core import Ctx
from suites.v2_books import Feed, _last, _problem
from suites.v2_sync import _upload

NAME = "v2_folder"
PW = "V2FolderTest123!"


def _companion(ctx: Ctx, token: str, name: str, content: bytes, ctype: str, **body) -> tuple[int, dict]:
    made = ctx.call("POST", "/api/v2/uploads", token, {"kind": "companion", "filename": name, "content_type": ctype,
                                                         "size_bytes": len(content)}, expect=(201,))
    ctx.put_bytes(made["url"], content, made["headers"]["Content-Type"])
    status, _h, raw = ctx.request("POST", "/api/v2/files", token, {"upload_id": made["upload_id"], **body})
    return status, (json.loads(raw) if raw else {})


def _snapshot(ctx: Ctx, token: str, kinds: str | None) -> list[dict]:
    q = f"?kinds={kinds}" if kinds else ""
    status, _h, raw = ctx.request("GET", "/api/v2/sync/snapshot" + q, token)
    assert status == 200, status
    return [json.loads(line) for line in raw.decode().splitlines() if line.strip()]


def _get_bytes(url: str) -> bytes:
    with urllib.request.urlopen(urllib.request.Request(url, headers={"User-Agent": "own-audio-conformance"}), timeout=30) as r:
        return r.read()


def run(ctx: Ctx) -> None:
    check, sfx = ctx.check, ctx.sfx
    one_household = ctx.feature("one_family")
    owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    member_id, member_email, member = ctx.make_user("member", PW)
    if not one_household:
        inv = ctx.call("POST", "/api/v2/family/invites", owner, {"kind": "email", "email": member_email, "role": "member"}, expect=(201,))
        ctx.call("POST", "/api/v2/family/join", member, {"code": inv["code"]})

    album_dir = f"Music/Companion {sfx}"
    track = ctx.call("POST", "/api/v2/tracks", owner, {"upload_id": _upload(ctx, owner, "track", "01 Song.mp3"), "filename": "01 Song.mp3",
                                                       "path": f"{album_dir}/01 Song.mp3", "title": f"Song {sfx}",
                                                       "album": f"Companion {sfx}", "artist": f"Folder Band {sfx}", "visibility": "family"},
                     expect=(201,))
    files_feed = Feed(ctx, member, kinds="file")
    owner_feed = Feed(ctx, owner, kinds="file,track")
    files(ctx, owner, member, track, album_dir, files_feed, owner_feed)
    holdings(ctx, owner, member, track)
    shortcuts(ctx, owner, owner_id, member, track)
    uploaded_show(ctx, owner, member)
    followed_show(ctx, owner)


def files(ctx: Ctx, owner: str, member: str, track: dict, album_dir: str, files_feed: Feed, owner_feed: Feed) -> None:
    check = ctx.check
    status, cover = _companion(ctx, owner, "Cover.JPG", b"\xff\xd8cover", "image/jpeg", path=f"{album_dir}/Cover.JPG")
    check("POST /files: 201, a file at its path, private, that became the album's cover",
          status == 201 and cover.get("path") == f"{album_dir}/Cover.JPG" and cover.get("name") == "Cover.JPG"
          and cover.get("visibility") == "private" and cover.get("used_as") == "cover", json.dumps(cover)[:300])
    now = ctx.call("GET", f"/api/v2/tracks/{track['id']}", owner)
    check("…on the track that had none", now.get("image") is not None and track.get("image") is None)
    status, back = _companion(ctx, owner, "back.jpg", b"\xff\xd8back", "image/jpeg",
                              attach_to={"kind": "track", "id": track["id"]}, relative_path="back.jpg")
    check("next to a track (attach_to): in its folder, kept, not used", status == 201 and back.get("path") == f"{album_dir}/back.jpg"
          and "used_as" not in back, json.dumps(back)[:300])
    status, lrc = _companion(ctx, owner, "01 Song.lrc", b"[00:01.00]Hello\n", "text/plain", path=f"{album_dir}/01 Song.lrc")
    check("a .lrc named like a track becomes its lyrics", status == 201 and lrc.get("used_as") == "lyrics", json.dumps(lrc)[:200])
    lyrics = ctx.call("GET", f"/api/v2/tracks/{track['id']}/lyrics", owner)
    check("…read back as the track's lyrics", "Hello" in (lyrics.get("text") or ""), str(lyrics)[:200])
    status, booklet = _companion(ctx, owner, "booklet.pdf", b"%PDF-1.4 booklet", "application/pdf",
                                 attach_to={"kind": "album", "id": track["album_id"]}, relative_path="booklet.pdf", visibility="family")
    check("next to an album, shared with the household", status == 201 and booklet.get("path") == f"{album_dir}/booklet.pdf"
          and booklet.get("visibility") == "family", json.dumps(booklet)[:300])
    for label, name, body, code in (
        ("a .zip: 400 unsupported_type", "x.zip", {"path": f"{album_dir}/x.zip"}, "unsupported_type"),
        ("under Podcasts/: 400 invalid_path", "p.jpg", {"path": f"Podcasts/Show {ctx.sfx}/p.jpg"}, "invalid_path"),
        ("a '..' in the path: 400 invalid_path", "q.jpg", {"path": f"{album_dir}/../q.jpg"}, "invalid_path"),
        ("path and attach_to both: 400 invalid_body", "r.jpg", {"path": f"{album_dir}/r.jpg", "attach_to": {"kind": "track", "id": track["id"]},
                                                              "relative_path": "r.jpg"}, "invalid_body"),
        ("attach_to without relative_path: 400 invalid_body", "s.jpg", {"attach_to": {"kind": "track", "id": track["id"]}}, "invalid_body"),
    ):
        made = ctx.call("POST", "/api/v2/uploads", owner, {"kind": "companion", "filename": name, "content_type": "image/jpeg", "size_bytes": 3}, expect=(201,))
        ctx.put_bytes(made["url"], b"abc", made["headers"]["Content-Type"])
        check(f"POST /files {label}", _problem(ctx.request("POST", "/api/v2/files", owner, {"upload_id": made["upload_id"], **body}), 400, code))
    status, _ = _companion(ctx, member, "m.jpg", b"\xff\xd8m", "image/jpeg", attach_to={"kind": "track", "id": track["id"]}, relative_path="m.jpg")
    check("attach_to someone else's track: 404", status == 404)
    made = ctx.call("POST", "/api/v2/uploads", owner, {"kind": "companion", "filename": "idem.txt", "content_type": "text/plain", "size_bytes": 4}, expect=(201,))
    ctx.put_bytes(made["url"], b"note", made["headers"]["Content-Type"])
    key = str(uuid.uuid4())
    body = {"upload_id": made["upload_id"], "path": f"{album_dir}/idem.txt"}
    first = ctx.request("POST", "/api/v2/files", owner, body, headers={"Idempotency-Key": key})
    again = ctx.request("POST", "/api/v2/files", owner, body, headers={"Idempotency-Key": key})
    check("Idempotency-Key: a retry answers the first answer, one file",
          first[0] == 201 and again[0] == 201 and json.loads(first[2])["id"] == json.loads(again[2])["id"])
    idem = json.loads(first[2])

    # ── Sync ────────────────────────────────────────────────────────────────
    lines = _snapshot(ctx, owner, "file")
    ids = {l["id"] for l in lines if l.get("type") == "record" and l.get("kind") == "file"}
    check("snapshot kinds=file: the owner's files", {cover["id"], back["id"], lrc["id"], booklet["id"], idem["id"]} <= ids, str(len(ids)))
    plain = _snapshot(ctx, owner, None)
    check("…files only when asked for: not in the default snapshot", not any(l.get("kind") == "file" for l in plain))
    seen = {l["id"] for l in _snapshot(ctx, member, "file") if l.get("type") == "record"}
    check("the member's snapshot: the shared booklet, not the private files", booklet["id"] in seen and cover["id"] not in seen, str(seen))
    rows = files_feed.read()
    check("the member's change feed: the shared booklet, not the private files",
          (_last(rows, "file", booklet["id"]) or {}).get("op") == "upsert"
          and (_last(rows, "file", cover["id"]) or {}).get("op") != "upsert",
          str([(r["kind"], r["op"]) for r in rows])[:300])
    check("GET /files/{id}: another member's private file is 404", ctx.status_of("GET", f"/api/v2/files/{cover['id']}", member) == 404)
    check("PUT /files/{id}/visibility by another member: 404",
          _problem(ctx.request("PUT", f"/api/v2/files/{cover['id']}/visibility", member, {"visibility": "family"}), 404))
    check("PUT /files/{id}/visibility visibility=public: 400",
          _problem(ctx.request("PUT", f"/api/v2/files/{cover['id']}/visibility", owner, {"visibility": "public"}), 400, "invalid_body"))
    shared = ctx.call("PUT", f"/api/v2/files/{cover['id']}/visibility", owner, {"visibility": "family"})
    rows = files_feed.read()
    check("sharing an image: 200, and it reaches the member's change feed", shared == {"visibility": "family"}
          and (_last(rows, "file", cover["id"]) or {}).get("record", {}).get("visibility") == "family", str(rows)[:300])
    check("…and GET /files/{id} as the member", ctx.call("GET", f"/api/v2/files/{cover['id']}", member)["id"] == cover["id"])

    # ── Download ────────────────────────────────────────────────────────────
    info = ctx.call("POST", "/api/v2/playback-info", member, {"kind": "file", "id": booklet["id"], "capabilities": {"direct": ["audio/mpeg"]}})
    check("POST /playback-info kind=file: the original, whatever the player plays", info["decision"] == "direct"
          and "companion_file" in info["reasons"] and info.get("content_type") == "application/pdf", json.dumps(info)[:300])
    check("…its bytes, unchanged", _get_bytes(info["url"]) == b"%PDF-1.4 booklet")
    check("kind=file of a private file of someone else's: 404",
          _problem(ctx.request("POST", "/api/v2/playback-info", member, {"kind": "file", "id": back["id"]}), 404))

    # ── Use as cover ────────────────────────────────────────────────────────
    before = ctx.call("GET", f"/api/v2/tracks/{track['id']}", owner)["image"]
    after = ctx.call("PUT", f"/api/v2/tracks/{track['id']}/image", owner, {"file_id": back["id"]})
    check("PUT /tracks/{id}/image {file_id}: a file of one's own becomes the cover", after.get("image") and after["image"] != before)
    check("…a file that is not an image: 400 not_an_image",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{track['id']}/image", owner, {"file_id": booklet["id"]}), 400, "not_an_image"))
    check("…upload_id and file_id both: 400 invalid_body",
          _problem(ctx.request("PUT", f"/api/v2/tracks/{track['id']}/image", owner, {"file_id": back["id"], "upload_id": "x"}), 400, "invalid_body"))
    book = ctx.call("POST", "/api/v2/books", member, {"title": f"Folder Book {ctx.sfx}", "author": "Folder Writer",
                                                      "files": [{"upload_id": _upload(ctx, member, "book_file", "b.mp3"), "relative_path": "01.mp3"}]},
                    expect=(201,))
    check("…someone else's file: 404",
          _problem(ctx.request("PUT", f"/api/v2/books/{book['id']}/image", member, {"file_id": cover["id"]}), 404))

    # ── Delete, restore ─────────────────────────────────────────────────────
    check("DELETE /files/{id} by another member: 403 (visible, not theirs)",
          _problem(ctx.request("DELETE", f"/api/v2/files/{booklet['id']}", member), 403))
    ctx.call("DELETE", f"/api/v2/files/{booklet['id']}", owner, expect=(204,))
    rows = files_feed.read()
    check("a deleted file: its removal in the change feed", (_last(rows, "file", booklet["id"]) or {}).get("op") == "delete")
    trash = ctx.call("GET", "/api/v2/trash", owner)["items"]
    row = next((t for t in trash if t["id"] == booklet["id"]), None)
    check("…in the trash as kind file, under its name", row is not None and row["kind"] == "file" and row["title"] == "booklet.pdf", str(row))
    ctx.call("POST", f"/api/v2/trash/file/{booklet['id']}/restore", owner, expect=(204,))
    rows = files_feed.read()
    back_again = (_last(rows, "file", booklet["id"]) or {}).get("record") or {}
    check("…restored: back in the change feed, where it was", back_again.get("path") == f"{album_dir}/booklet.pdf")

    # ── Organise ────────────────────────────────────────────────────────────
    check("organise without dry_run: 422 invalid_body, nothing moved",
          _problem(ctx.request("POST", "/api/v2/library/organise", owner, {"kind": "track"}), 422, "invalid_body"))
    check("organise kind=films: 400 invalid_kind",
          _problem(ctx.request("POST", "/api/v2/library/organise", owner, {"kind": "films", "dry_run": True}), 400, "invalid_kind"))
    owner_feed.read()
    dry = ctx.call("POST", "/api/v2/library/organise", owner, {"kind": "track", "dry_run": True, "ids": [track["id"]]})
    mv = {m["id"]: m for m in dry["moves"]}
    check("a dry run lists the track's move with the files beside it", dry["dry_run"] is True and track["id"] in mv
          and mv[track["id"]]["from"] == f"{album_dir}/01 Song.mp3" and mv[track["id"]]["to"] != mv[track["id"]]["from"]
          and mv[track["id"]]["files"] >= 4, json.dumps(dry)[:300])
    target = mv.get(track["id"], {}).get("to", "").rsplit("/", 1)[0]
    rows = owner_feed.read()
    check("…and changes nothing", ctx.call("GET", f"/api/v2/tracks/{track['id']}", owner)["path"] == f"{album_dir}/01 Song.mp3"
          and not any((r.get("record") or {}).get("path", "").startswith(target + "/") for r in rows))
    done = ctx.call("POST", "/api/v2/library/organise", owner, {"kind": "track", "dry_run": False, "ids": [track["id"]]})
    check("the moves made are the dry run's", done["dry_run"] is False and done["moves"] == dry["moves"], json.dumps(done)[:300])
    new_dir = mv[track["id"]]["to"].rsplit("/", 1)[0]
    rows = owner_feed.read()
    moved = (_last(rows, "track", track["id"]) or {}).get("record", {}).get("path")
    moved_files = [r for r in rows if r["kind"] == "file" and (r.get("record") or {}).get("path", "").startswith(new_dir + "/")]
    check("…the track's new path in the change feed, same id", moved == mv[track["id"]]["to"], str(moved))
    check("…and its files', in the new folder", len(moved_files) >= 4, str([(r["kind"], (r.get("record") or {}).get("path")) for r in rows])[:400])
    again = ctx.call("POST", "/api/v2/library/organise", owner, {"kind": "track", "dry_run": True})
    check("an organised track drops out of the next dry run", track["id"] not in {m["id"] for m in again["moves"]})
    other = ctx.call("POST", "/api/v2/library/organise", member, {"kind": "track", "dry_run": True})
    check("nobody organises someone else's items", track["id"] not in {m["id"] for m in other["moves"]})


def holdings(ctx: Ctx, owner: str, member: str, track: dict) -> None:
    check = ctx.check
    refs = [{"kind": "track", "id": track["id"]}] + [{"kind": "episode", "id": str(uuid.uuid4())} for _ in range(3)]
    ctx.call("PUT", "/api/v2/me/holdings", owner, {"items": refs}, expect=(204,))
    held = ctx.call("GET", "/api/v2/me/holdings?limit=500", owner)["items"]
    check("PUT /me/holdings {items}: the device's set, in v2's words", {(h["kind"], h["id"]) for h in held} == {(r["kind"], r["id"]) for r in refs},
          str(held)[:300])
    small, cursor, pages = [], None, 0
    while True:
        page = ctx.call("GET", "/api/v2/me/holdings?limit=1" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else ""), owner)
        small += page["items"]
        cursor, pages = page.get("next"), pages + 1
        if not cursor or pages > 20:
            break
    check("…paged one at a time without loss", [h["id"] for h in small] == [h["id"] for h in held] and pages == len(held))
    ctx.call("PUT", "/api/v2/me/holdings", owner, {"added": [{"kind": "book", "id": str(uuid.uuid4())}], "removed": refs[1:]}, expect=(204,))
    held = ctx.call("GET", "/api/v2/me/holdings", owner)["items"]
    check("added and removed change the set", sorted(h["kind"] for h in held) == ["book", "track"], str(held))
    devices = ctx.call("GET", f"/api/v2/me/holdings/track/{track['id']}", owner)
    check("GET /me/holdings/{kind}/{id}: this device, as the current one", len(devices) == 1 and devices[0]["current"] is True, str(devices))
    check("…another account sees none of it", ctx.call("GET", f"/api/v2/me/holdings/track/{track['id']}", member) == []
          and ctx.call("GET", "/api/v2/me/holdings", member)["items"] == [])
    check("an unknown kind: 400 invalid_kind", _problem(ctx.request("PUT", "/api/v2/me/holdings", owner, {"added": [{"kind": "music_track", "id": track["id"]}]}), 400, "invalid_kind")
          and _problem(ctx.request("GET", f"/api/v2/me/holdings/films/{track['id']}", owner), 400, "invalid_kind"))


def shortcuts(ctx: Ctx, owner: str, owner_id: str, member: str, track: dict) -> None:
    check = ctx.check
    feed = Feed(ctx, member, kinds="shortcut")
    made = ctx.request("POST", "/api/v2/sync/shortcuts", member, {"album_id": track["album_id"]})
    s = json.loads(made[2]) if made[2] else {}
    check("POST /sync/shortcuts {album_id}: 201, the album's owner found, by its id", made[0] == 201 and s.get("album_id") == track["album_id"]
          and s.get("member_id") == owner_id and s.get("media") == "music", json.dumps(s)[:300])
    again = ctx.request("POST", "/api/v2/sync/shortcuts", member, {"album_id": track["album_id"], "media": "music"})
    check("…again: 200, the same one", again[0] == 200 and json.loads(again[2])["id"] == s.get("id"))
    v1 = ctx.call("GET", "/api/v1/sync/shortcuts", member)
    old = next((x for x in v1 if x["id"] == s.get("id")), {})
    check("…what v1's sync clients read: an album shortcut they match by name or release group",
          old.get("container_kind") == "album" and (old.get("release_group") or old.get("album")), str(old))
    whole = ctx.request("POST", "/api/v2/sync/shortcuts", member, {"member_id": owner_id, "media": "books"})
    check("a member's whole kind: media=books", whole[0] == 201 and json.loads(whole[2])["media"] == "books" and json.loads(whole[2])["book_id"] is None)
    for label, body, code in (("nothing named: 400 invalid_body", {}, "invalid_body"),
                              ("two things named: 400 invalid_body", {"album_id": track["album_id"], "book_id": str(uuid.uuid4())}, "invalid_body"),
                              ("media=films: 400 invalid_media", {"media": "films", "member_id": owner_id}, "invalid_media"),
                              ("media against the album: 400 invalid_media", {"media": "books", "album_id": track["album_id"]}, "invalid_media")):
        check(f"POST /sync/shortcuts {label}", _problem(ctx.request("POST", "/api/v2/sync/shortcuts", member, body), 400, code))
    check("POST /sync/shortcuts to oneself: 400", _problem(ctx.request("POST", "/api/v2/sync/shortcuts", owner, {"member_id": owner_id, "media": "music"}), 400))
    check("an album nobody else has: 404", _problem(ctx.request("POST", "/api/v2/sync/shortcuts", member, {"album_id": str(uuid.uuid4())}), 404))
    listed = ctx.call("GET", "/api/v2/sync/shortcuts", member)
    check("GET /sync/shortcuts: the member's", {x["id"] for x in listed} >= {s.get("id"), json.loads(whole[2])["id"]})
    check("…the owner's list is separate", ctx.call("GET", "/api/v2/sync/shortcuts", owner) == [])
    rows = feed.read()
    check("a new shortcut reaches the listener's change feed", (_last(rows, "shortcut", s.get("id")) or {}).get("record", {}).get("album_id") == track["album_id"])
    check("DELETE someone else's shortcut: 404", _problem(ctx.request("DELETE", f"/api/v2/sync/shortcuts/{s.get('id')}", owner), 404))
    ctx.call("DELETE", f"/api/v2/sync/shortcuts/{s.get('id')}", member, expect=(204,))
    check("…removed: 204, then 404", ctx.status_of("DELETE", f"/api/v2/sync/shortcuts/{s.get('id')}", member) == 404)
    rows = feed.read()
    check("…its removal in the change feed", (_last(rows, "shortcut", s.get("id")) or {}).get("op") == "delete")
    lines = _snapshot(ctx, member, "shortcut")
    check("snapshot kinds=shortcut: the listener's", {l["id"] for l in lines if l.get("kind") == "shortcut"} == {json.loads(whole[2])["id"]})


def uploaded_show(ctx: Ctx, owner: str, member: str) -> None:
    check, sfx = ctx.check, ctx.sfx
    feed = Feed(ctx, member, kinds="podcast,episode")
    guid = f"folder-show-{sfx}"
    show = ctx.call("POST", "/api/v2/podcasts", owner, {"title": f"Folder Show {sfx}", "visibility": "family", "guid": guid}, expect=(201,))
    eps = [ctx.call("POST", f"/api/v2/podcasts/{show['id']}/episodes", owner, {"upload_id": _upload(ctx, owner, "episode", f"e{i}.mp3"),
                                                                             "guid": f"{guid}-{i}", "title": f"Folder Episode {i}"}, expect=(200, 201))
           for i in range(2)]
    feed.read()
    check("DELETE /podcasts/{id} of another member's shared show: 403", _problem(ctx.request("DELETE", f"/api/v2/podcasts/{show['id']}", member), 403))
    ctx.call("DELETE", f"/api/v2/podcasts/{show['id']}", owner, expect=(204,))
    check("an uploaded show deleted: gone from the library", ctx.status_of("GET", f"/api/v2/podcasts/{show['id']}", owner) == 404
          and ctx.status_of("GET", f"/api/v2/episodes/{eps[0]['id']}", member) == 404)
    rows = feed.read()
    check("…its removal and its episodes' in the change feed", (_last(rows, "podcast", show["id"]) or {}).get("op") == "delete"
          and all((_last(rows, "episode", e["id"]) or {}).get("op") == "delete" for e in eps), str([(r["kind"], r["op"]) for r in rows]))
    trash = ctx.call("GET", "/api/v2/trash", owner)["items"]
    row = next((t for t in trash if t["id"] == show["id"]), None)
    check("…in the trash as kind podcast, with its episodes' size", row is not None and row["kind"] == "podcast"
          and row["title"] == f"Folder Show {sfx}" and row["size_bytes"] > 0, str(row))
    check("…its episodes are in it with the show, not listed apart", not any(t["id"] in {e["id"] for e in eps} for t in trash))
    v1 = ctx.call("GET", "/api/v1/trash", owner)
    check("v1's trash list does not show it (v1 knows no shows in the trash)", not any(t["id"] == show["id"] for t in v1))
    remade = ctx.call("POST", "/api/v2/podcasts", owner, {"title": f"Folder Show {sfx} again", "guid": guid}, expect=(201,))
    check("a new show of the same guid while the old one is in the trash", remade["id"] != show["id"])
    ctx.call("POST", f"/api/v2/trash/podcast/{show['id']}/restore", owner, expect=(204,))
    back = ctx.call("GET", f"/api/v2/podcasts/{show['id']}", owner)
    listed = ctx.call("GET", f"/api/v2/podcasts/{show['id']}/episodes", owner)["items"]
    check("restored: the show with its episodes, beside the new one", back["id"] == show["id"] and len(listed) == 2
          and ctx.call("GET", f"/api/v2/podcasts/{remade['id']}", owner)["id"] == remade["id"])
    rows = feed.read()
    check("…back in the member's change feed, episodes too", (_last(rows, "podcast", show["id"]) or {}).get("op") == "upsert"
          and all((_last(rows, "episode", e["id"]) or {}).get("op") == "upsert" for e in eps))
    batch = ctx.call("POST", "/api/v2/trash", owner, {"items": [{"kind": "podcast", "id": show["id"]}, {"kind": "podcast", "id": remade["id"]}]})
    check("POST /trash with kind podcast", [i["status"] for i in batch["items"]] == ["ok", "ok"], str(batch))
    if not ctx.db_available:
        ctx.skip("purging a show's objects", "needs --compose-dir")
        ctx.call("DELETE", f"/api/v2/trash/podcast/{show['id']}", owner, expect=(204,))
        return
    keys = ctx.sql(f"SELECT string_agg(m.object_key, ',') FROM podcast_episodes e JOIN media_objects m ON m.id = e.audio_object_id "
                   f"WHERE e.feed_id = '{show['id']}'")
    ctx.call("DELETE", f"/api/v2/trash/podcast/{show['id']}", owner, expect=(204,))
    left = ctx.sql(f"SELECT (SELECT count(*) FROM podcast_feeds_all WHERE id = '{show['id']}') || ' ' "
                   f"|| (SELECT count(*) FROM podcast_episodes WHERE feed_id = '{show['id']}') || ' ' "
                   f"|| (SELECT count(*) FROM media_objects WHERE object_key = ANY(string_to_array('{keys}', ',')))")
    check("purged: the show, its episodes and their stored objects are gone", left == "0 0 0", left)
    check("…and it is out of the trash", ctx.status_of("POST", f"/api/v2/trash/podcast/{show['id']}/restore", owner) == 404)


def followed_show(ctx: Ctx, owner: str) -> None:
    from suites.v2_podcasts import Feeds
    if not ctx.is_local:
        ctx.skip("a followed show", "needs a loopback --base-url: the server fetches the feed from this machine")
        return
    feeds = Feeds(ctx)
    ctx.on_cleanup("feed server", feeds.stop)
    url = feeds.add_show(f"folder{ctx.sfx}", 2)
    status, _h, raw = ctx.request("POST", "/api/v2/podcasts", owner, {"feed_url": url})
    if status not in (200, 201):
        ctx.skip("a followed show", f"the server could not follow a feed from this machine ({status})")
        return
    show = json.loads(raw)
    ctx.call("DELETE", f"/api/v2/podcasts/{show['id']}", owner, expect=(204,))
    trash = ctx.call("GET", "/api/v2/trash", owner)["items"]
    ctx.check("a followed show is unfollowed outright, not trashed", ctx.status_of("GET", f"/api/v2/podcasts/{show['id']}", owner) == 404
              and not any(t["id"] == show["id"] for t in trash))
