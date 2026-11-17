# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 listening state (completion plan phase 8): the cross-device queue, player settings, the notification inbox, push devices, and track feedback in `POST /sync/push`.

Two devices of one listener: one writes the queue, the other gets it from `/sync/changes` and a
`library` event on `/events`; a write made against an older queue (`If-Match`) is `412
revision_mismatch` with the current revision; v1's queue and v2's are one queue (a v1 write moves
the revision on). Settings round-trip in milliseconds, are refused out of range without changing
anything, and reach the other device's change feed; v1 reads what v2 wrote. Notifications (a
household admin moving a member's songs to the trash) page newest first, are acknowledged by id
or `up_to`, and reach the change feed when they arrive and when they are read. A push device is
seen and removed only by its owner and moves with its token. Problem shapes throughout.
"""
from __future__ import annotations

import json
import time
import urllib.parse
import urllib.request

from core import Ctx
from suites.v2_books import Feed, _last, _problem
from suites.v2_sync import _upload

NAME = "v2_listening"
PW = "V2ListeningTest123!"


def _device(ctx: Ctx, email: str, name: str, kind: str) -> str:
    data = urllib.parse.urlencode({"grant_type": "password", "username": email, "password": PW,
                                   "device_name": name, "device_kind": kind}).encode()
    st, _h, raw = ctx.request("POST", "/api/v2/auth/token", body=data, headers={"Content-Type": "application/x-www-form-urlencoded"})
    assert st == 200, f"sign-in as {name}: {st} {raw[:200]!r}"
    return json.loads(raw)["access_token"]


def _library_event_after(ctx: Ctx, token: str, action) -> bool:
    """Opens the caller's `/events`, does `action`, and says whether a `library` event came."""
    req = urllib.request.Request(ctx.url("/api/v2/events"), headers={"Authorization": f"Bearer {token}", "Accept": "text/event-stream"})
    try:
        with urllib.request.urlopen(req, timeout=20) as r:
            action()
            deadline = time.time() + 20
            for raw in r:
                if raw.decode().strip() == "event: library":
                    return True
                if time.time() > deadline:
                    break
    except Exception:  # noqa: BLE001 — a timeout means no event came
        pass
    return False


def _put_queue(ctx: Ctx, token: str, body: dict, revision: int | str | None = None):
    headers = {"Content-Type": "application/json"}
    if revision is not None:
        headers["If-Match"] = f'"{revision}"' if isinstance(revision, int) else revision
    return ctx.request("PUT", "/api/v2/me/queue", token, json.dumps(body).encode(), headers=headers)


def run(ctx: Ctx) -> None:
    one_household = ctx.feature("one_family")
    admin_id, _ae, admin = ctx.make_family_admin("admin", PW)
    member_id, member_email, member = ctx.make_user("member", PW)
    other_id, other_email, other = ctx.make_user("other", PW)
    if not one_household:
        for email, tok in ((member_email, member), (other_email, other)):
            inv = ctx.call("POST", "/api/v2/family/invites", admin, {"kind": "email", "email": email, "role": "member"}, expect=(201,))
            ctx.call("POST", "/api/v2/family/join", tok, {"code": inv["code"]})
    phone = _device(ctx, member_email, "Phone", "ios")
    speaker = _device(ctx, member_email, "Speaker", "tvos")

    queue(ctx, member_id, phone, speaker, other)
    settings(ctx, member_id, phone, speaker)
    notifications(ctx, admin, member, member_id, other)
    push_devices(ctx, member, other)
    offline_feedback(ctx, member)
    snapshot(ctx, member_id, phone)


def queue(ctx: Ctx, member_id: str, phone: str, speaker: str, other: str) -> None:
    check, sfx = ctx.check, ctx.sfx
    tracks = [ctx.upload_track(phone, f"Queue Song {sfx} {i}", artist=f"Queue Band {sfx}", album=f"Queue Album {sfx}",
                               content=ctx.tone(), content_type="audio/wav") for i in range(2)]
    book = ctx.call("POST", "/api/v2/books", phone, {"title": f"Queue Book {sfx}", "author": f"Queue Writer {sfx}", "files": [
        {"upload_id": _upload(ctx, phone, "book_file", f"q{i}.mp3"), "relative_path": f"0{i + 1}.mp3"} for i in range(2)]}, expect=(201,))
    book = ctx.call("GET", f"/api/v2/books/{book['id']}", phone)
    show = ctx.call("POST", "/api/v2/podcasts", phone, {"title": f"Queue Show {sfx}"}, expect=(200, 201))
    episode = ctx.call("POST", f"/api/v2/podcasts/{show['id']}/episodes", phone, {
        "upload_id": _upload(ctx, phone, "episode", "qe.mp3"), "guid": f"queue-{sfx}", "title": "Queue Episode"}, expect=(200, 201))
    other_track = ctx.upload_track(other, f"Not Yours {sfx}", content=ctx.tone(), content_type="audio/wav")

    st, h, raw = ctx.request("GET", "/api/v2/me/queue", phone)
    empty = json.loads(raw) if st == 200 else {}
    check("GET /me/queue before the first write: empty, revision 0, ETag \"0\"",
          st == 200 and empty.get("items") == [] and empty.get("revision") == 0 and h.get("etag") == '"0"', f"{st} {raw[:200]!r}")

    feed = Feed(ctx, speaker, "queue")
    items = [{"kind": "track", "id": tracks[0]["id"]}, {"kind": "book", "id": book["id"], "file_id": book["files"][1]["id"]},
             {"kind": "episode", "id": episode["id"]}, {"kind": "track", "id": tracks[1]["id"]}]
    st, h, raw = _put_queue(ctx, phone, {"items": items, "index": 2, "position_ms": 61_000}, 0)
    put = json.loads(raw) if st == 200 else {}
    check("PUT /me/queue with If-Match \"0\" on no queue → 200, revision 1, as sent",
          st == 200 and put.get("revision") == 1 and put.get("items") == items and put.get("index") == 2
          and put.get("position_ms") == 61_000 and h.get("etag") == '"1"', f"{st} {raw[:300]!r}")
    check("…naming the device that wrote it", put.get("device_name") == "Phone" and put.get("device_kind") == "ios", str(put)[:200])
    row = _last(feed.read(), "queue", member_id)
    check("the other device gets it from /sync/changes as `queue`",
          row is not None and row["op"] == "upsert" and row["record"]["revision"] == 1 and row["record"]["items"] == items, str(row)[:300])

    came = _library_event_after(ctx, speaker, lambda: _put_queue(ctx, phone, {"items": items[:2], "index": 1, "position_ms": 5000}, 1))
    check("…and a `library` event on its /events when the queue changes", came)
    now = ctx.call("GET", "/api/v2/me/queue", speaker)
    check("the second write moved the revision to 2", now["revision"] == 2 and now["items"] == items[:2], str(now)[:200])

    st, h, raw = _put_queue(ctx, speaker, {"items": items, "index": 0}, 1)
    check("a write made against an older queue → 412 revision_mismatch", _problem((st, h, raw), 412, "revision_mismatch"), f"{st} {raw[:200]!r}")
    check("…which names the current revision, and changes nothing",
          st == 412 and json.loads(raw).get("revision") == 2 and ctx.call("GET", "/api/v2/me/queue", phone)["revision"] == 2)
    check("If-Match \"0\" when there is a queue → 412", _problem(_put_queue(ctx, speaker, {"items": []}, 0), 412, "revision_mismatch"))
    check("an If-Match that is no revision → 412", _problem(_put_queue(ctx, speaker, {"items": []}, '"abc"'), 412, "revision_mismatch"))
    st, h, raw = _put_queue(ctx, speaker, {"items": items, "index": 9, "position_ms": -5})
    last = json.loads(raw) if st == 200 else {}
    check("without If-Match the last write wins (as v1); the index is kept in the list, the position from 0",
          st == 200 and last["revision"] == 3 and last["index"] == 3 and last["position_ms"] == 0 and last["device_name"] == "Speaker",
          f"{st} {raw[:200]!r}")

    v1 = ctx.call("GET", "/api/v1/playback/queue", phone)
    kinds = [(i["media_kind"], i["item_id"], i.get("part_id")) for i in v1["items"]]
    check("v1 reads the same queue: an episode as its show and itself, a book with its file",
          kinds == [("music", tracks[0]["id"], None), ("audiobook", book["id"], book["files"][1]["id"]),
                    ("podcast", show["id"], episode["id"]), ("music", tracks[1]["id"], None)], str(kinds)[:300])
    ctx.call("PUT", "/api/v1/playback/queue", phone, {"items": v1["items"][:1], "current_index": 0, "position_secs": 2.5})
    after_v1 = ctx.call("GET", "/api/v2/me/queue", phone)
    check("a v1 write moves the revision on, so a v2 write made before it is refused",
          after_v1["revision"] == 4 and after_v1["position_ms"] == 2500
          and _problem(_put_queue(ctx, speaker, {"items": items}, 3), 412, "revision_mismatch"), str(after_v1)[:200])

    check("an item of a kind that is not played → 400 invalid_kind",
          _problem(_put_queue(ctx, phone, {"items": [{"kind": "album", "id": tracks[0]["id"]}]}), 400, "invalid_kind"))
    check("someone else's private song → 400 invalid_item",
          _problem(_put_queue(ctx, phone, {"items": [{"kind": "track", "id": other_track["id"]}]}), 400, "invalid_item"))
    check("a file that is not the book's → 400 invalid_item",
          _problem(_put_queue(ctx, phone, {"items": [{"kind": "book", "id": book["id"], "file_id": tracks[0]["id"]}]}), 400, "invalid_item"))
    check("a file on a song → 400 invalid_body",
          _problem(_put_queue(ctx, phone, {"items": [{"kind": "track", "id": tracks[0]["id"], "file_id": book["files"][0]["id"]}]}), 400, "invalid_body"))
    check("more than 1000 items → 400 too_many",
          _problem(_put_queue(ctx, phone, {"items": [{"kind": "track", "id": tracks[0]["id"]}] * 1001}), 400, "too_many"))
    check("…and none of those refusals changed it", ctx.call("GET", "/api/v2/me/queue", phone)["revision"] == 4)
    check("another listener's queue is their own", ctx.call("GET", "/api/v2/me/queue", other)["revision"] == 0)


def settings(ctx: Ctx, member_id: str, phone: str, speaker: str) -> None:
    check = ctx.check
    first = ctx.call("GET", "/api/v2/me/settings", phone)
    check("GET /me/settings: v1's defaults in milliseconds",
          first["audiobook"]["skip_back_ms"] == 15_000 and first["audiobook"]["skip_forward_ms"] == 30_000
          and first["playback"]["speed"] == 1.0 and first["eq"] == [], str(first)[:300])
    feed = Feed(ctx, speaker, "settings")
    patched = ctx.call("PATCH", "/api/v2/me/settings", phone, {
        "audiobook": {"skip_back_ms": 10_000, "skip_forward_ms": 45_000, "speed": 1.25},
        "playback": {"speed": 1.1, "skip_intro_ms": 5000}})
    want = {"skip_back_ms": 10_000, "skip_forward_ms": 45_000, "speed": 1.25}
    check("PATCH /me/settings round-trips in milliseconds",
          patched["audiobook"] == want and patched["playback"]["speed"] == 1.1 and patched["playback"]["skip_intro_ms"] == 5000
          and patched["playback"]["skip_outro_ms"] == 0, str(patched)[:300])
    check("…and reads back the same", ctx.call("GET", "/api/v2/me/settings", speaker)["audiobook"] == want)
    v1 = ctx.call("GET", "/api/v1/playback/settings", phone)
    check("v1 reads what v2 wrote, in seconds", v1["ab_skip_backward_secs"] == 10 and v1["ab_skip_forward_secs"] == 45
          and v1["ab_playback_speed"] == 1.25, str(v1)[:200])
    row = _last(feed.read(), "settings", member_id)
    check("the other device gets them from /sync/changes as `settings`",
          row is not None and row["op"] == "upsert" and row["record"]["audiobook"] == want, str(row)[:300])

    bands = [1.5, 0, -3, 0, 2, 12]
    eq = ctx.call("PATCH", "/api/v2/me/settings", phone, {"eq": {"device_kind": "tvos", "enabled": True, "bands_db": bands,
                                                                 "preamp_db": -2, "preset_name": "Night"}})["eq"]
    tv = next((c for c in eq if c["device_kind"] == "tvos"), {})
    check("an equaliser curve for a named device kind", tv.get("enabled") is True and tv.get("bands_db") == bands
          and tv.get("preamp_db") == -2 and tv.get("preset_name") == "Night", str(eq)[:300])
    v1eq = ctx.call("GET", "/api/v1/playback/settings/eq?device_kind=tvos", phone)
    check("…the one v1 reads for that device", v1eq["band6_db"] == 12 and v1eq["enabled"] is True and v1eq["preset_name"] == "Night")
    own = ctx.call("PATCH", "/api/v2/me/settings", phone, {"eq": {"enabled": True}})["eq"]
    check("without a device kind, the calling device's own (a phone signed in as `ios`)",
          any(c["device_kind"] == "ios" and c["enabled"] for c in own) and len(own) == 2, str(own)[:300])
    renamed = ctx.call("PATCH", "/api/v2/me/settings", phone, {"eq": {"device_kind": "tvos", "preset_name": None}})["eq"]
    tv2 = next((c for c in renamed if c["device_kind"] == "tvos"), {})
    check("a patch keeps what it leaves out; `null` takes a name away",
          tv2.get("preset_name") is None and all(tv2.get(k) == tv.get(k) for k in ("enabled", "bands_db", "preamp_db")), str(tv2))

    before = ctx.call("GET", "/api/v2/me/settings", phone)
    for label, body in [("a skip step under a second", {"audiobook": {"skip_back_ms": 500}}),
                        ("a speed over 3", {"playback": {"speed": 4}}),
                        ("five bands", {"eq": {"bands_db": [0, 0, 0, 0, 0]}}),
                        ("a gain over 12 dB", {"eq": {"bands_db": [0, 0, 0, 0, 0, 13]}}),
                        ("an unknown device kind", {"eq": {"device_kind": "toaster"}}),
                        ("a good value beside a bad one", {"audiobook": {"skip_back_ms": 20_000}, "eq": {"preamp_db": 40}})]:
        check(f"PATCH /me/settings with {label} → 400 invalid_body",
              _problem(ctx.request("PATCH", "/api/v2/me/settings", phone, body), 400, "invalid_body"))
    check("…and none of them changed anything", ctx.call("GET", "/api/v2/me/settings", phone) == before)
    ctx.call("PUT", "/api/v1/playback/settings/audiobook-defaults", phone,
             {"ab_skip_forward_secs": 20, "ab_skip_backward_secs": 5, "ab_playback_speed": 1.0})
    row = _last(feed.read(), "settings", member_id)
    check("a v1 write reaches the change feed too", row is not None and row["record"]["audiobook"]["skip_back_ms"] == 5000, str(row)[:200])


def notifications(ctx: Ctx, admin: str, member: str, member_id: str, other: str) -> None:
    check, sfx = ctx.check, ctx.sfx
    feed = Feed(ctx, member, "notification")
    songs = [ctx.upload_track(member, f"Inbox Song {sfx} {i}", visibility="family", content=ctx.tone(), content_type="audio/wav")
             for i in range(4)]
    for s in songs:
        ctx.call("DELETE", f"/api/v2/tracks/{s['id']}", admin, expect=(204,))
        time.sleep(0.05)  # distinct arrival times, so newest-first is checkable
    page = ctx.call("GET", "/api/v2/me/notifications?limit=500", member)
    mine = [n for n in page["items"] if n.get("item", {}).get("id") in {s["id"] for s in songs}]
    check("GET /me/notifications: the four songs a household admin moved to the trash, newest first, unread",
          [n["item"]["id"] for n in mine] == [s["id"] for s in reversed(songs)]
          and all(n["kind"] == "item_trashed" and n["item"]["kind"] == "track" and "read_at" not in n for n in mine),
          json.dumps(page["items"][:2])[:400])
    rows = [r for r in feed.read() if r["kind"] == "notification"]
    check("each reached /sync/changes as `notification`", {r["id"] for r in rows} >= {n["id"] for n in mine}
          and all(r["op"] == "upsert" and r["record"]["title"] for r in rows), str(rows)[:300])
    walked, cursor = [], None
    for _ in range(20):
        q = "limit=1" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else "")
        p = ctx.call("GET", f"/api/v2/me/notifications?{q}", member)
        walked += [n["id"] for n in p["items"]]
        cursor = p.get("next")
        if not cursor:
            break
    check("paging one at a time gives the same list, none twice", walked == [n["id"] for n in page["items"]])
    check("another member's inbox does not hold them",
          not {n["id"] for n in mine} & {n["id"] for n in ctx.call("GET", "/api/v2/me/notifications?limit=500", other)["items"]})

    n0, n1, n2, n3 = (n["id"] for n in mine)
    one = ctx.call("POST", "/api/v2/me/notifications/ack", member, {"ids": [n0]})
    again = ctx.call("POST", "/api/v2/me/notifications/ack", member, {"ids": [n0]})
    check("ack by id: one acknowledged, and nothing the second time", one == {"acknowledged": 1} and again == {"acknowledged": 0})
    check("another member cannot acknowledge them", ctx.call("POST", "/api/v2/me/notifications/ack", other, {"ids": [n1]}) == {"acknowledged": 0})
    upto = ctx.call("POST", "/api/v2/me/notifications/ack", member, {"up_to": n2})
    unread = [n["id"] for n in ctx.call("GET", "/api/v2/me/notifications?unread_only=true&limit=500", member)["items"]]
    check("ack up_to: that one and every older one", upto["acknowledged"] >= 2 and n1 in unread
          and not {n0, n2, n3} & set(unread), f"{upto} {unread}")
    read = {n["id"]: n for n in ctx.call("GET", "/api/v2/me/notifications?limit=500", member)["items"]}
    check("read ones say when", all(read[i].get("read_at") for i in (n0, n2, n3)) and "read_at" not in read[n1])
    row = _last(feed.read(), "notification", n3)
    check("being read reaches the change feed", row is not None and row["record"].get("read_at"), str(row)[:200])
    check("up_to someone else's → 404", _problem(ctx.request("POST", "/api/v2/me/notifications/ack", other, {"up_to": n1}), 404))
    check("neither ids nor up_to → 400 invalid_body", _problem(ctx.request("POST", "/api/v2/me/notifications/ack", member, {}), 400, "invalid_body"))
    check("both → 400 invalid_body", _problem(ctx.request("POST", "/api/v2/me/notifications/ack", member, {"ids": [n1], "up_to": n1}), 400, "invalid_body"))
    check("a bad cursor → 400 invalid_cursor", _problem(ctx.request("GET", "/api/v2/me/notifications?cursor=%25%25", member), 400, "invalid_cursor"))


def push_devices(ctx: Ctx, member: str, other: str) -> None:
    check, sfx = ctx.check, ctx.sfx
    token = f"push-{sfx}-0123456789abcdef"
    st, _h, raw = ctx.request("POST", "/api/v2/me/push-tokens", member, {"platform": "apns", "token": token, "device_name": "Phone"},
                              headers={"Idempotency-Key": f"push-{sfx}"})
    made = json.loads(raw) if st == 201 else {}
    check("POST /me/push-tokens → 201, only the token's tail shown",
          st == 201 and made.get("platform") == "apns" and made.get("token_suffix") == token[-6:] and token not in raw.decode(), f"{st} {raw[:200]!r}")
    st2, _h, raw2 = ctx.request("POST", "/api/v2/me/push-tokens", member, {"platform": "apns", "token": token, "device_name": "Phone"},
                                headers={"Idempotency-Key": f"push-{sfx}"})
    check("the same Idempotency-Key answers the first result", st2 == 201 and json.loads(raw2)["id"] == made.get("id"))
    again = ctx.call("POST", "/api/v2/me/push-tokens", member, {"platform": "apns", "token": token}, expect=(200,))
    check("registering it again refreshes it: 200, the same device", again["id"] == made.get("id"))
    listed = ctx.call("GET", "/api/v2/me/push-tokens", member)
    check("GET /me/push-tokens lists it", [d["id"] for d in listed].count(made.get("id")) == 1)
    check("another member does not see it", made.get("id") not in [d["id"] for d in ctx.call("GET", "/api/v2/me/push-tokens", other)])
    check("…nor remove it (404)", _problem(ctx.request("DELETE", f"/api/v2/me/push-tokens/{made.get('id')}", other), 404))
    check("an unknown platform → 400 invalid_platform",
          _problem(ctx.request("POST", "/api/v2/me/push-tokens", member, {"platform": "sms", "token": "x"}), 400, "invalid_platform"))
    check("an empty token → 400 invalid_body",
          _problem(ctx.request("POST", "/api/v2/me/push-tokens", member, {"platform": "fcm", "token": " "}), 400, "invalid_body"))
    moved = ctx.call("POST", "/api/v2/me/push-tokens", other, {"platform": "apns", "token": token}, expect=(201,))
    check("a token registered by another account moves to it",
          moved["id"] == made.get("id") and made.get("id") not in [d["id"] for d in ctx.call("GET", "/api/v2/me/push-tokens", member)])
    check("DELETE /me/push-tokens/{id} → 204, then 404",
          ctx.status_of("DELETE", f"/api/v2/me/push-tokens/{moved['id']}", other) == 204
          and _problem(ctx.request("DELETE", f"/api/v2/me/push-tokens/{moved['id']}", other), 404))
    modules = ctx.call("GET", "/.well-known/own-audio")["modules"]
    check("discovery says whether the server sends push: modules.push is false here", modules.get("push") is False, str(modules.get("push")))


def offline_feedback(ctx: Ctx, member: str) -> None:
    check, sfx = ctx.check, ctx.sfx
    song = ctx.upload_track(member, f"Offline Dislike {sfx}", content=ctx.tone(), content_type="audio/wav")
    res = ctx.call("POST", "/api/v2/sync/push", member, {"changes": [
        {"client_id": "f1", "op": "feedback", "kind": "track", "id": song["id"], "value": "ban"},
        {"client_id": "f2", "op": "feedback", "kind": "track", "id": song["id"]},
        {"client_id": "f3", "op": "feedback", "kind": "book", "id": song["id"], "value": "ban"}]})
    check("POST /sync/push takes track feedback made offline; a missing value or another kind is refused per change",
          [r["status"] for r in res] == ["ok", "invalid_body", "unsupported_kind"], str(res))
    fb = ctx.call("GET", "/api/v2/me/feedback?value=ban", member)["items"]
    check("…and stores it as `PUT /me/feedback` does", any(f["id"] == song["id"] and f["value"] == "ban" for f in fb))
    res = ctx.call("POST", "/api/v2/sync/push", member, {"changes": [{"client_id": "u1", "op": "unfeedback", "kind": "track", "id": song["id"]}]})
    check("`unfeedback` takes it back", res[0]["status"] == "ok"
          and not any(f["id"] == song["id"] for f in ctx.call("GET", "/api/v2/me/feedback", member)["items"]))


def snapshot(ctx: Ctx, member_id: str, token: str) -> None:
    check = ctx.check

    def lines(query: str) -> list[dict]:
        raw = ctx.call("GET", f"/api/v2/sync/snapshot{query}", token, raw=True)
        return [json.loads(x) for x in raw.decode().splitlines() if x.strip()]

    got = lines("?kinds=queue,settings")
    recs = {(x["kind"], x["id"]): x["record"] for x in got if x["type"] == "record"}
    check("the snapshot carries the queue and the settings, named by the listener",
          set(recs) == {("queue", member_id), ("settings", member_id)} and recs[("queue", member_id)]["revision"] >= 1
          and got[-1]["type"] == "end", str(got)[:300])
    ids = [x for x in lines("?kinds=queue,settings&fields=ids") if x["type"] == "id"]
    check("…and in the ids-only snapshot", sorted((x["kind"], x["id"]) for x in ids) == [("queue", member_id), ("settings", member_id)])
    every = {x["kind"] for x in lines("") if x["type"] == "record"}
    check("…and in the default one", {"queue", "settings"} <= every, str(sorted(every)))
    check("notifications are not a snapshot kind (400 invalid_kind)",
          _problem(ctx.request("GET", "/api/v2/sync/snapshot?kinds=notification", token), 400, "invalid_kind"))
