# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 deletion and the trash (completion plan phase 4): delete a track or a book, many things as one batch, list the trash page by page, restore one item or a batch, purge one, empty it.

What the v1 `trash` suite proves, on v2: a deleted item leaves every list and the change feed
sends its removal (and its album's, when it was the album's only song); a restore brings it back
in both; a batch restores whole; a purge deletes the stored object (checked through SQL with
`--compose-dir`); only the owner, or a household admin for what is shared, may delete, restore or
purge; another household's ids answer 404 (on a server with several households). Also: each item
of a batch answers on its own, `Idempotency-Key` answers a retry with the first answer, the list
pages without loss, and every refusal is problem details.
"""
from __future__ import annotations

import datetime
import json
import urllib.parse
import uuid

from core import Ctx
from suites.v2_sync import _upload

NAME = "v2_trash"
PW = "V2TrashTest123!"
PROBLEM = "application/problem+json"


def _problem(answer, status: int, code: str | None = None) -> bool:
    got, headers, body = answer
    if got != status or not headers.get("content-type", "").startswith(PROBLEM):
        return False
    try:
        doc = json.loads(body)
    except ValueError:
        return False
    return (doc.get("status") == status and str(doc.get("type", "")).startswith("urn:own-audio:problem:")
            and bool(doc.get("title")) and (code is None or doc.get("code") == code))


def _time(s: str) -> datetime.datetime:
    return datetime.datetime.fromisoformat(s.replace("Z", "+00:00"))


class Feed:
    """One reader's change feed from a cursor taken now."""

    def __init__(self, ctx: Ctx, token: str) -> None:
        self.ctx, self.token = ctx, token
        st, _h, raw = ctx.request("GET", "/api/v2/sync/changes", token)
        assert st == 200, f"changes {st}"
        self.cursor = json.loads(raw)["cursor"]

    def read(self) -> dict[tuple[str, str], str]:
        """The last op per (kind, id) since the cursor; moves the cursor on."""
        got: dict[tuple[str, str], str] = {}
        for _ in range(100):
            st, _h, raw = self.ctx.request("GET", "/api/v2/sync/changes?" + urllib.parse.urlencode({"cursor": self.cursor, "limit": "500"}), self.token)
            assert st == 200, f"changes {st}"
            page = json.loads(raw)
            for c in page["changes"]:
                got[(c["kind"], c["id"])] = c["op"]
            self.cursor = page["cursor"]
            if not page["has_more"]:
                break
        return got


class Suite:
    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx
        self.n = 0

    def track(self, tok: str, visibility: str = "family", album: str | None = None) -> dict:
        self.n += 1
        sfx = self.ctx.sfx
        return self.ctx.call("POST", "/api/v2/tracks", tok, {
            "upload_id": _upload(self.ctx, tok, "track", f"t{self.n}.mp3"), "filename": f"t{self.n}.wav",
            "title": f"Trash Song {sfx} {self.n}", "artist": f"Trash Band {sfx}",
            "album": album or f"Trash Album {sfx} {self.n}", "visibility": visibility}, expect=(201,))

    def book(self, tok: str, visibility: str = "family") -> dict:
        self.n += 1
        sfx = self.ctx.sfx
        return self.ctx.call("POST", "/api/v2/books", tok, {
            "title": f"Trash Book {sfx} {self.n}", "author": f"Trash Writer {sfx}", "visibility": visibility,
            "files": [{"upload_id": _upload(self.ctx, tok, "book_file", f"b{self.n}.mp3"), "relative_path": "01.mp3"}]},
            expect=(201,))

    def listed(self, path: str, tok: str, item_id: str) -> bool:
        """Whether a list holds the item, searched by the suite's name for it."""
        q = urllib.parse.urlencode({"q": self.ctx.sfx, "limit": "500"})
        st, _h, raw = self.ctx.request("GET", f"{path}?{q}", tok)
        assert st == 200, f"{path} {st}"
        return item_id in {i["id"] for i in json.loads(raw)["items"]}

    def trash(self, tok: str, scope: str = "mine", limit: int = 500) -> list[dict]:
        items, cursor = [], None
        for _ in range(200):
            q = {"scope": scope, "limit": str(limit)}
            if cursor:
                q["cursor"] = cursor
            page = self.ctx.call("GET", "/api/v2/trash?" + urllib.parse.urlencode(q), tok)
            items += page["items"]
            cursor = page.get("next")
            if not cursor:
                break
        return items


def run(ctx: Ctx) -> None:
    s = Suite(ctx)
    one_household = ctx.feature("one_family")
    owner_id, _oe, owner = ctx.make_family_admin("owner", PW)
    member_id, member_email, member = ctx.make_user("member", PW)
    if not one_household:
        inv = ctx.call("POST", "/api/v1/family/invites", owner, {"email": member_email, "role": "member"})
        ctx.call("POST", "/api/v1/family/invites/accept", member, {"code": inv["code"]})

    problems(ctx, s, owner, member)
    delete_and_restore(ctx, s, owner, owner_id, member)
    batches(ctx, s, owner, member)
    idempotency(ctx, s, owner)
    who_may(ctx, s, owner, owner_id, member, member_id)
    other_household(ctx, s, owner, one_household)
    episodes_and_playlists(ctx, s, owner)
    purge(ctx, s, owner, member)
    paging(ctx, s, owner)
    empty(ctx, s, owner)


def problems(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    check, rid = ctx.check, str(uuid.uuid4())
    check("POST /trash without items is 400 invalid_body",
          _problem(ctx.request("POST", "/api/v2/trash", owner, {"items": []}), 400, "invalid_body"))
    many = [{"kind": "track", "id": str(uuid.uuid4())} for _ in range(501)]
    check("POST /trash with more than batch_max items is 400 too_many",
          _problem(ctx.request("POST", "/api/v2/trash", owner, {"items": many}), 400, "too_many"))
    disc = ctx.call("GET", "/.well-known/own-audio")
    check("discovery's batch_max is the trash's limit", disc["limits"]["batch_max"] == 500, str(disc["limits"]))
    check("an unknown scope is 400 invalid_scope",
          _problem(ctx.request("GET", "/api/v2/trash?scope=everyone", owner), 400, "invalid_scope"))
    check("a member asking for the household's trash is 403",
          _problem(ctx.request("GET", "/api/v2/trash?scope=family", member), 403))
    check("…and may not empty it either",
          _problem(ctx.request("DELETE", "/api/v2/trash?scope=family", member), 403))
    check("a cursor the list did not hand out is 400 invalid_cursor",
          _problem(ctx.request("GET", "/api/v2/trash?cursor=nope", owner), 400, "invalid_cursor"))
    check("a limit over 500 is 400 invalid_limit",
          _problem(ctx.request("GET", "/api/v2/trash?limit=501", owner), 400, "invalid_limit"))
    check("restoring an unknown kind is 400 invalid_kind",
          _problem(ctx.request("POST", f"/api/v2/trash/music_track/{rid}/restore", owner), 400, "invalid_kind"))
    check("purging an unknown kind is 400 invalid_kind",
          _problem(ctx.request("DELETE", f"/api/v2/trash/podcast/{rid}", owner), 400, "invalid_kind"))
    check("restoring what is not in the trash is 404",
          _problem(ctx.request("POST", f"/api/v2/trash/track/{rid}/restore", owner), 404, "not_found"))
    check("purging what is not in the trash is 404",
          _problem(ctx.request("DELETE", f"/api/v2/trash/book/{rid}", owner), 404, "not_found"))
    check("a batch with nothing in it is 404",
          _problem(ctx.request("POST", f"/api/v2/trash/batches/{rid}/restore", owner), 404, "not_found"))
    check("deleting a track that does not exist is 404",
          _problem(ctx.request("DELETE", f"/api/v2/tracks/{rid}", owner), 404, "not_found"))


def delete_and_restore(ctx: Ctx, s: Suite, owner: str, owner_id: str, member: str) -> None:
    check = ctx.check
    t = s.track(owner)
    b = s.book(owner)
    feed = Feed(ctx, member)
    before = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=5)

    check("before: the member's lists hold the track and the book",
          s.listed("/api/v2/tracks", member, t["id"]) and s.listed("/api/v2/books", member, b["id"]))
    ctx.call("DELETE", f"/api/v2/tracks/{t['id']}", owner, expect=(204,))
    ctx.call("DELETE", f"/api/v2/books/{b['id']}", owner, expect=(204,))
    check("a deleted track leaves the owner's and the member's lists",
          not s.listed("/api/v2/tracks", owner, t["id"]) and not s.listed("/api/v2/tracks", member, t["id"]))
    check("a deleted book leaves the lists", not s.listed("/api/v2/books", member, b["id"]))
    check("reading a deleted track is 404", _problem(ctx.request("GET", f"/api/v2/tracks/{t['id']}", owner), 404))
    check("reading a deleted book is 404", _problem(ctx.request("GET", f"/api/v2/books/{b['id']}", member), 404))
    check("deleting it again is 404", _problem(ctx.request("DELETE", f"/api/v2/tracks/{t['id']}", owner), 404))
    got = feed.read()
    check("the member's change feed removes the track and the book",
          got.get(("track", t["id"])) == "delete" and got.get(("book", b["id"])) == "delete", str(got)[:300])
    check("…and the album, whose only song it was", got.get(("album", t.get("album_id"))) == "delete", str(got)[:300])

    rows = {i["id"]: i for i in s.trash(owner)}
    row = rows.get(t["id"])
    check("the track is in the owner's trash, with its kind and title",
          row is not None and row["kind"] == "track" and row["title"] == t["title"], str(row))
    if row:
        check("…deleted now, purged 30 days on",
              _time(row["deleted_at"]) >= before and _time(row["purge_at"]) - _time(row["deleted_at"]) == datetime.timedelta(days=30),
              str(row))
        check("…its owner and who deleted it", row["owner_id"] == owner_id and row.get("deleted_by") == owner_id, str(row))
        check("…a batch of its own, its size, no restore charge on this server",
              bool(row.get("batch")) and row["batch"] != rows.get(b["id"], {}).get("batch") and row["size_bytes"] > 0
              and (row["restore_charge_micro"] == 0 or ctx.feature("billing")), str(row))
    check("the book is in the trash as kind book", rows.get(b["id"], {}).get("kind") == "book")
    check("the member's own trash does not hold the owner's things",
          not ({t["id"], b["id"]} & {i["id"] for i in s.trash(member)}))

    ctx.call("POST", f"/api/v2/trash/track/{t['id']}/restore", owner, expect=(204,))
    ctx.call("POST", f"/api/v2/trash/book/{b['id']}/restore", owner, expect=(204,))
    check("restored, the track is in the member's list again", s.listed("/api/v2/tracks", member, t["id"]))
    check("…and the book", s.listed("/api/v2/books", member, b["id"]))
    got = feed.read()
    check("the change feed sends the track, its album and the book again",
          got.get(("track", t["id"])) == "upsert" and got.get(("album", t.get("album_id"))) == "upsert"
          and got.get(("book", b["id"])) == "upsert", str(got)[:300])
    check("restoring twice is 404", _problem(ctx.request("POST", f"/api/v2/trash/track/{t['id']}/restore", owner), 404))
    check("the trash no longer holds them", not ({t["id"], b["id"]} & {i["id"] for i in s.trash(owner)}))


def batches(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    check = ctx.check
    album = f"Trash Batch Album {ctx.sfx}"
    t1, t2 = s.track(owner, album=album), s.track(owner, album=album)
    b = s.book(owner)
    rid = str(uuid.uuid4())
    feed = Feed(ctx, member)
    answer = ctx.call("POST", "/api/v2/trash", owner, {"items": [
        {"kind": "track", "id": t1["id"]}, {"kind": "track", "id": t2["id"]}, {"kind": "book", "id": b["id"]},
        {"kind": "track", "id": t1["id"]}, {"kind": "teleport", "id": rid}, {"kind": "track", "id": rid}]})
    statuses = [(i["client_id"], i["status"]) for i in answer["items"]]
    check("each item answers in order: ok, ok, ok, duplicate, invalid_kind, not_found",
          statuses == [(t1["id"], "ok"), (t2["id"], "ok"), (b["id"], "ok"), (t1["id"], "duplicate"), (rid, "invalid_kind"), (rid, "not_found")],
          str(statuses))
    batch = answer["batch"]
    rows = {i["id"]: i for i in s.trash(owner)}
    check("the three are in the trash as one batch", all(rows.get(x["id"], {}).get("batch") == batch for x in (t1, t2, b)), str(rows)[:300])
    got = feed.read()
    check("the change feed removes all three, and the album they made",
          all(got.get(k) == "delete" for k in (("track", t1["id"]), ("track", t2["id"]), ("book", b["id"]), ("album", t1.get("album_id")))),
          str(got)[:300])

    back = ctx.call("POST", f"/api/v2/trash/batches/{batch}/restore", owner)
    check("restoring the batch brings all three", back["batch"] == batch
          and sorted((i["client_id"], i["status"]) for i in back["items"]) == sorted((x["id"], "ok") for x in (t1, t2, b))
          and back["charged_micro"] >= 0, str(back))
    check("…into the lists", s.listed("/api/v2/tracks", member, t1["id"]) and s.listed("/api/v2/tracks", member, t2["id"])
          and s.listed("/api/v2/books", member, b["id"]))
    got = feed.read()
    check("…and into the change feed", all(got.get(k) == "upsert" for k in (("track", t1["id"]), ("track", t2["id"]), ("book", b["id"]))),
          str(got)[:300])
    check("restoring the batch again is 404", _problem(ctx.request("POST", f"/api/v2/trash/batches/{batch}/restore", owner), 404))

    # v1's X-Trash-Batch batches restore through v2 too.
    v1_batch = str(uuid.uuid4())
    ctx.call("DELETE", f"/api/v1/music/tracks/{t1['id']}", owner, headers={"X-Trash-Batch": v1_batch})
    ctx.call("DELETE", f"/api/v1/audiobooks/{b['id']}", owner, headers={"X-Trash-Batch": v1_batch})
    back = ctx.call("POST", f"/api/v2/trash/batches/{v1_batch}/restore", owner)
    check("a batch deleted through v1 restores through v2", len([i for i in back["items"] if i["status"] == "ok"]) == 2, str(back))


def idempotency(ctx: Ctx, s: Suite, owner: str) -> None:
    check = ctx.check
    t, other = s.track(owner), s.track(owner)
    key = f"trash-{uuid.uuid4()}"
    body = {"items": [{"kind": "track", "id": t["id"]}]}
    first = ctx.request("POST", "/api/v2/trash", owner, body, headers={"Idempotency-Key": key})
    again = ctx.request("POST", "/api/v2/trash", owner, body, headers={"Idempotency-Key": key})
    a, b = json.loads(first[2]), json.loads(again[2])
    check("a retry with the same Idempotency-Key answers the first answer, the same batch",
          first[0] == again[0] == 200 and a == b and a["items"][0]["status"] == "ok" and again[1].get("idempotent-replayed") == "true",
          f"{first[0]} {again[0]} {a} {b}")
    check("…and deletes nothing more", sum(i["id"] == t["id"] for i in s.trash(owner)) == 1)
    check("the same key with another request is 422 idempotency_key_reused",
          _problem(ctx.request("POST", "/api/v2/trash", owner, {"items": [{"kind": "track", "id": other["id"]}]},
                               headers={"Idempotency-Key": key}), 422, "idempotency_key_reused"))
    ctx.call("POST", f"/api/v2/trash/batches/{a['batch']}/restore", owner)


def who_may(ctx: Ctx, s: Suite, owner: str, owner_id: str, member: str, member_id: str) -> None:
    check = ctx.check
    shared, private = s.track(owner), s.track(owner, visibility="private")
    check("a member deleting the owner's shared track is 403",
          _problem(ctx.request("DELETE", f"/api/v2/tracks/{shared['id']}", member), 403, "forbidden"))
    check("…the owner's private track is 404",
          _problem(ctx.request("DELETE", f"/api/v2/tracks/{private['id']}", member), 404, "not_found"))
    answer = ctx.call("POST", "/api/v2/trash", member, {"items": [{"kind": "track", "id": shared["id"]}, {"kind": "track", "id": private["id"]}]})
    check("in a batch, the same refusals per item: forbidden, not_found",
          [i["status"] for i in answer["items"]] == ["forbidden", "not_found"], str(answer))

    ctx.call("DELETE", f"/api/v2/tracks/{shared['id']}", owner, expect=(204,))
    check("a member cannot restore the owner's deleted track (404)",
          _problem(ctx.request("POST", f"/api/v2/trash/track/{shared['id']}/restore", member), 404))
    check("…nor purge it (404)", _problem(ctx.request("DELETE", f"/api/v2/trash/track/{shared['id']}", member), 404))
    st, _h, _b = ctx.request("DELETE", "/api/v2/trash", member)
    check("…and emptying their own trash leaves it", st == 200 and shared["id"] in {i["id"] for i in s.trash(owner)})
    ctx.call("POST", f"/api/v2/trash/track/{shared['id']}/restore", owner, expect=(204,))

    # A household admin deletes a member's shared song: it is the member's, in the member's trash.
    theirs, their_private = s.track(member), s.track(member, visibility="private")
    check("a household admin cannot delete a member's private track (404)",
          _problem(ctx.request("DELETE", f"/api/v2/tracks/{their_private['id']}", owner), 404))
    ctx.call("DELETE", f"/api/v2/tracks/{theirs['id']}", owner, expect=(204,))
    row = next((i for i in s.trash(member) if i["id"] == theirs["id"]), None)
    check("a household admin may delete a member's shared track; it is in the member's trash, deleted by the admin",
          row is not None and row["owner_id"] == member_id and row.get("deleted_by") == owner_id, str(row))
    check("the admin sees it in the household's trash", theirs["id"] in {i["id"] for i in s.trash(owner, scope="family")})
    check("…not in their own", theirs["id"] not in {i["id"] for i in s.trash(owner)})
    ctx.call("POST", f"/api/v2/trash/track/{theirs['id']}/restore", member, expect=(204,))
    check("the member restores it", s.listed("/api/v2/tracks", member, theirs["id"]))


def other_household(ctx: Ctx, s: Suite, owner: str, one_household: bool) -> None:
    if one_household:
        ctx.skip("another household's ids are 404", "the server has one household: there is no other to try from")
        return
    check = ctx.check
    _oid, _oe, outsider = ctx.make_family_admin("outsider", PW)
    t = s.track(owner)
    trashed = s.track(owner)
    ctx.call("DELETE", f"/api/v2/tracks/{trashed['id']}", owner, expect=(204,))
    answer = ctx.call("POST", "/api/v2/trash", outsider, {"items": [{"kind": "track", "id": t["id"]}]})
    check("another household's admin: deleting the owner's shared track is 404",
          _problem(ctx.request("DELETE", f"/api/v2/tracks/{t['id']}", outsider), 404)
          and answer["items"][0]["status"] == "not_found", str(answer))
    check("…restoring or purging what the owner deleted is 404",
          _problem(ctx.request("POST", f"/api/v2/trash/track/{trashed['id']}/restore", outsider), 404)
          and _problem(ctx.request("DELETE", f"/api/v2/trash/track/{trashed['id']}", outsider), 404))
    check("…and it is not in their household's trash", trashed["id"] not in {i["id"] for i in s.trash(outsider, scope="family")})
    ctx.call("POST", f"/api/v2/trash/track/{trashed['id']}/restore", owner, expect=(204,))


def episodes_and_playlists(ctx: Ctx, s: Suite, owner: str) -> None:
    check, sfx = ctx.check, ctx.sfx
    show = ctx.call("POST", "/api/v2/podcasts", owner, {"title": f"Trash Show {sfx}", "visibility": "private"}, expect=(200, 201))
    ep = ctx.call("POST", f"/api/v2/podcasts/{show['id']}/episodes", owner, {
        "upload_id": _upload(ctx, owner, "episode", "e.mp3"), "guid": f"trash-{sfx}", "title": f"Trash Episode {sfx}"}, expect=(200, 201))
    pl = ctx.call("POST", "/api/v2/playlists", owner, {"name": f"Trash List {sfx}"}, expect=(201,))
    answer = ctx.call("POST", "/api/v2/trash", owner, {"items": [{"kind": "episode", "id": ep["id"]}, {"kind": "playlist", "id": pl["id"]}]})
    check("an episode's stored copy and a playlist go to the trash in one batch",
          [i["status"] for i in answer["items"]] == ["ok", "ok"], str(answer))
    got = ctx.call("GET", f"/api/v2/episodes/{ep['id']}", owner)
    check("the episode stays listed, no longer stored", got["stored"] is False, str(got.get("stored")))
    check("the playlist is gone", _problem(ctx.request("GET", f"/api/v2/playlists/{pl['id']}", owner), 404))
    kinds = {i["id"]: i["kind"] for i in s.trash(owner)}
    check("they are in the trash as kinds episode and playlist", kinds.get(ep["id"]) == "episode" and kinds.get(pl["id"]) == "playlist")
    ctx.call("POST", f"/api/v2/trash/batches/{answer['batch']}/restore", owner)
    check("restored, the episode is stored again and the playlist back",
          ctx.call("GET", f"/api/v2/episodes/{ep['id']}", owner)["stored"] is True
          and ctx.call("GET", f"/api/v2/playlists/{pl['id']}", owner)["id"] == pl["id"])
    check("an uploaded show itself is not deleted yet: 409 uploaded_podcast",
          _problem(ctx.request("DELETE", f"/api/v2/podcasts/{show['id']}", owner), 409, "uploaded_podcast"))


def purge(ctx: Ctx, s: Suite, owner: str, member: str) -> None:
    check = ctx.check
    t = s.track(owner)
    obj = ctx.sql(f"SELECT audio_object_id FROM music_tracks_all WHERE id = '{t['id']}'") if ctx.db_available else None
    feed = Feed(ctx, member)
    ctx.call("DELETE", f"/api/v2/tracks/{t['id']}", owner, expect=(204,))
    ctx.call("DELETE", f"/api/v2/trash/track/{t['id']}", owner, expect=(204,))
    check("purged, it is no longer in the trash", t["id"] not in {i["id"] for i in s.trash(owner)})
    check("…and cannot be restored", _problem(ctx.request("POST", f"/api/v2/trash/track/{t['id']}/restore", owner), 404))
    got = feed.read()
    check("the change feed ends with its removal", got.get(("track", t["id"])) == "delete", str(got)[:300])
    if obj:
        check("the row is gone", ctx.sql(f"SELECT count(*) FROM music_tracks_all WHERE id = '{t['id']}'") == "0")
        check("its stored object is gone", ctx.sql(f"SELECT count(*) FROM media_objects WHERE id = '{obj}'") == "0")
    else:
        ctx.skip("a purge deletes the row and the stored object", "needs --compose-dir for SQL")


def paging(ctx: Ctx, s: Suite, owner: str) -> None:
    check = ctx.check
    made = [s.track(owner) for _ in range(3)] + [s.book(owner) for _ in range(2)]
    # Two single deletes and one batch, so some share a batch and others do not.
    ctx.call("DELETE", f"/api/v2/tracks/{made[0]['id']}", owner, expect=(204,))
    ctx.call("DELETE", f"/api/v2/books/{made[3]['id']}", owner, expect=(204,))
    ctx.call("POST", "/api/v2/trash", owner, {"items": [{"kind": "track", "id": made[1]["id"]}, {"kind": "track", "id": made[2]["id"]},
                                                        {"kind": "book", "id": made[4]["id"]}]})
    full, small = s.trash(owner, limit=500), s.trash(owner, limit=2)
    keys = [(i["kind"], i["id"]) for i in full]
    check("the trash pages two at a time without loss or repeats",
          keys == [(i["kind"], i["id"]) for i in small] and len(set(keys)) == len(keys) and len(keys) >= 5,
          f"{len(small)} against {len(full)}")
    times = [_time(i["deleted_at"]) for i in full]
    check("newest first", times == sorted(times, reverse=True))
    check("everything deleted is listed", {x["id"] for x in made} <= {i["id"] for i in full})


def empty(ctx: Ctx, s: Suite, owner: str) -> None:
    check = ctx.check
    n = len(s.trash(owner))
    out = ctx.call("DELETE", "/api/v2/trash", owner)
    check("emptying the trash deletes everything in it for good", out["purged"] == n and n > 0, f"{out} of {n}")
    check("the trash is empty", s.trash(owner) == [])
    check("emptying an empty trash deletes nothing", ctx.call("DELETE", "/api/v2/trash", owner)["purged"] == 0)
