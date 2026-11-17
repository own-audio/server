# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 statistics (completion plan phase 9): a listener's totals over a range grouped by day, week or month, the plays history, who sees one's figures, and the household's figures for its admins.

What the v1 `stats` suite proves, on v2: plays reported in a batch, a retried play counted once,
totals per kind and the most listened item, day buckets in the listener's own time zone (a play
just before UTC midnight lands on the next day at UTC+2), the streak; the history paged without
loss, newest first, with each device; privacy — a member who does not share is absent from
`GET /family/stats`, sharing in `/me/settings` makes them present and reaches their devices'
change feed, an admin may turn it on only for an account they manage (`403
member_not_restricted`), a plain member cannot read the household's figures. Problem shapes for
a bad range, group, kind and time zone.
"""
from __future__ import annotations

import json
import urllib.parse
from datetime import datetime, timedelta, timezone

from core import Ctx
from suites.v2_books import Feed, _last, _problem
from suites.v2_sync import _upload

NAME = "v2_stats"
PW = "V2StatsTest123!"


def _iso(t: datetime) -> str:
    return t.astimezone(timezone.utc).isoformat().replace("+00:00", "Z")


def _play(cid: str, kind: str, item: str, end: datetime, seconds: int, device: str = "android", file_id: str | None = None) -> dict:
    p = {"client_id": cid, "kind": kind, "id": item, "started_at": _iso(end - timedelta(seconds=seconds)), "ended_at": _iso(end),
         "listened_ms": seconds * 1000, "device_kind": device}
    if file_id:
        p["file_id"] = file_id
    return p


def _q(**kw) -> str:
    return "?" + urllib.parse.urlencode({k: v for k, v in kw.items() if v is not None})


def run(ctx: Ctx) -> None:
    check, sfx = ctx.check, ctx.sfx
    one_household = ctx.feature("one_family")
    parent_id, _pe, parent = ctx.make_family_admin("parent", PW)
    kid_id, kid_email, kid = ctx.make_user("kid", PW)
    adult_id, adult_email, adult = ctx.make_user("adult", PW)
    if not one_household:
        for email, tok in ((kid_email, kid), (adult_email, adult)):
            inv = ctx.call("POST", "/api/v2/family/invites", parent, {"kind": "email", "email": email, "role": "member"}, expect=(201,))
            ctx.call("POST", "/api/v2/family/join", tok, {"code": inv["code"]})

    book = ctx.call("POST", "/api/v2/books", parent, {"title": f"Stats Book {sfx}", "author": f"Stats Writer {sfx}", "visibility": "family",
                                                     "files": [{"upload_id": _upload(ctx, parent, "book_file", "s.mp3"), "relative_path": "01.mp3"}]},
                    expect=(201,))
    song = ctx.upload_track(parent, f"Stats Song {sfx}", visibility="family", content=ctx.tone(), content_type="audio/wav")
    now = datetime.now(timezone.utc)

    # ── Reporting, retries, totals ─────────────────────────────────────────
    batch = [_play(f"{sfx}-a1", "book", book["id"], now - timedelta(minutes=60), 600),
             _play(f"{sfx}-a2", "book", book["id"], now - timedelta(minutes=40), 900),
             _play(f"{sfx}-a3", "book", book["id"], now - timedelta(minutes=20), 300, device="ios"),
             _play(f"{sfx}-m1", "track", song["id"], now - timedelta(minutes=10), 120)]
    first = ctx.call("POST", "/api/v2/me/plays", parent, {"plays": batch})
    again = ctx.call("POST", "/api/v2/me/plays", parent, {"plays": batch})
    check("plays reported in a batch, and the same batch again counts nothing",
          [r["status"] for r in first] == ["ok"] * 4 and [r["status"] for r in again] == ["duplicate"] * 4, f"{first} {again}")
    stats = ctx.call("GET", "/api/v2/me/stats" + _q(from_=None, kind=None), parent)
    check("GET /me/stats: a retried play counted once (1920 s), four plays", stats["listened_ms"] == 1_920_000 and stats["plays"] == 4,
          json.dumps(stats)[:300])
    kinds = {k["kind"]: k for k in stats["by_kind"]}
    check("…per kind, in v2's words", kinds.get("book", {}).get("listened_ms") == 1_800_000 and kinds["book"]["plays"] == 3
          and kinds.get("track", {}).get("listened_ms") == 120_000, str(stats["by_kind"]))
    check("…the most listened first", stats["top"][0] == {"kind": "book", "id": book["id"], "listened_ms": 1_800_000, "plays": 3},
          str(stats["top"][:2]))
    check("…a streak that counts today, and a 30-day range ending now by default",
          stats["streak_days"] >= 1 and stats["group"] == "day"
          and abs((datetime.fromisoformat(stats["to"].replace("Z", "+00:00")) - datetime.fromisoformat(stats["from"].replace("Z", "+00:00"))).days - 30) <= 1)
    only_songs = ctx.call("GET", "/api/v2/me/stats" + _q(kind="track"), parent)
    check("kind=track counts songs only", only_songs["listened_ms"] == 120_000 and [k["kind"] for k in only_songs["by_kind"]] == ["track"])
    check("buckets add up to the totals", sum(b["listened_ms"] for b in stats["buckets"]) == stats["listened_ms"]
          and sum(b["plays"] for b in stats["buckets"]) == 4)

    # ── Day bucketing in the listener's time ──────────────────────────────
    day = (now - timedelta(days=3)).replace(hour=23, minute=0, second=0, microsecond=0)
    ctx.call("POST", "/api/v2/me/plays", adult, {"plays": [_play(f"{sfx}-tz", "track", song["id"], day + timedelta(minutes=30), 600)]})
    rng = {"from": _iso(day - timedelta(days=1)), "to": _iso(day + timedelta(days=2))}
    utc = ctx.call("GET", "/api/v2/me/stats" + _q(**rng), adult)
    east = ctx.call("GET", "/api/v2/me/stats" + _q(**rng, tz_offset_minutes=120), adult)
    check("a play at 23:00 UTC falls on its UTC day…", [b["start"] for b in utc["buckets"]] == [day.date().isoformat()], str(utc["buckets"]))
    check("…and on the next day at UTC+2, the totals unchanged",
          [b["start"] for b in east["buckets"]] == [(day + timedelta(days=1)).date().isoformat()] and east["listened_ms"] == utc["listened_ms"] == 600_000,
          str(east["buckets"]))
    monthly = ctx.call("GET", "/api/v2/me/stats" + _q(**rng, group="month"), adult)
    check("group=month starts its buckets on the 1st", all(b["start"].endswith("-01") for b in monthly["buckets"]), str(monthly["buckets"]))
    weekly = ctx.call("GET", "/api/v2/me/stats" + _q(**rng, group="week"), adult)
    check("group=week starts its buckets on a Monday", all(datetime.fromisoformat(b["start"]).weekday() == 0 for b in weekly["buckets"]))

    for label, query, code in [("from after to", _q(**{"from": _iso(now), "to": _iso(now - timedelta(days=1))}), "invalid_range"),
                               ("more than ten years", _q(**{"from": "2000-01-01T00:00:00Z"}), "invalid_range"),
                               ("group=year", _q(group="year"), "invalid_group"),
                               ("kind=album", _q(kind="album"), "unsupported_kind"),
                               ("tz_offset_minutes=999", _q(tz_offset_minutes=999), "invalid_tz")]:
        check(f"GET /me/stats with {label} → 400 {code}", _problem(ctx.request("GET", "/api/v2/me/stats" + query, parent), 400, code))

    # ── History ──────────────────────────────────────────────────────────────
    hist = ctx.call("GET", "/api/v2/me/plays?limit=500", parent)["items"]
    mine = [h for h in hist if (h.get("client_id") or "").startswith(sfx)]
    check("GET /me/plays: every play once, newest first", [h["client_id"] for h in mine] == [f"{sfx}-m1", f"{sfx}-a3", f"{sfx}-a2", f"{sfx}-a1"],
          str([h.get("client_id") for h in mine]))
    check("…with its device, its length in ms and that a device reported it",
          {h["device_kind"] for h in mine} == {"android", "ios"} and mine[0]["listened_ms"] == 120_000
          and all(h["source"] == "reported" for h in mine) and mine[1]["kind"] == "book" and mine[1]["item_id"] == book["id"])
    walked, cursor = [], None
    for _ in range(50):
        p = ctx.call("GET", "/api/v2/me/plays" + _q(limit=1, cursor=cursor), parent)
        walked += [h["id"] for h in p["items"]]
        cursor = p.get("next")
        if not cursor:
            break
    check("the history one at a time: the same plays, none twice, none lost", walked == [h["id"] for h in hist] and len(set(walked)) == len(walked))
    check("another member's plays are not in it", not {h["id"] for h in hist} & {h["id"] for h in ctx.call("GET", "/api/v2/me/plays?limit=500", adult)["items"]})
    check("a bad cursor → 400 invalid_cursor", _problem(ctx.request("GET", "/api/v2/me/plays?cursor=%25%25", parent), 400, "invalid_cursor"))

    # ── Privacy ──────────────────────────────────────────────────────────────
    ctx.call("POST", "/api/v2/me/plays", kid, {"plays": [_play(f"{sfx}-k1", "track", song["id"], now - timedelta(minutes=5), 60)]})
    fam = {m["user_id"]: m for m in ctx.call("GET", "/api/v2/family/stats", parent)}
    check("GET /family/stats: the admin's own figures", fam.get(parent_id, {}).get("listened_ms") == 1_920_000, str(fam.get(parent_id)))
    check("…and a member who does not share is absent", kid_id not in fam and adult_id not in fam)
    check("a plain member cannot read the household's figures (403)", _problem(ctx.request("GET", "/api/v2/family/stats", kid), 403))
    check("GET /me/settings says the adult does not share", ctx.call("GET", "/api/v2/me/settings", adult)["stats_visible_to_admin"] is False)

    check("an admin cannot expose an adult who has no restriction → 403 member_not_restricted",
          _problem(ctx.request("PATCH", f"/api/v2/family/members/{adult_id}", parent, {"stats_visible_to_admin": True}), 403, "member_not_restricted"))
    feed = Feed(ctx, adult, "settings")
    shared = ctx.call("PATCH", "/api/v2/me/settings", adult, {"stats_visible_to_admin": True})
    check("the adult shares in /me/settings", shared["stats_visible_to_admin"] is True)
    row = _last(feed.read(), "settings", adult_id)
    check("…which reaches their devices' change feed", row is not None and row["record"]["stats_visible_to_admin"] is True, str(row)[:200])
    fam = {m["user_id"]: m for m in ctx.call("GET", "/api/v2/family/stats", parent)}
    check("…and their figures are in /family/stats now", fam.get(adult_id, {}).get("listened_ms") == 600_000
          and fam[adult_id]["by_kind"] == [{"kind": "track", "listened_ms": 600_000, "plays": 1}], str(fam.get(adult_id)))
    ctx.call("PATCH", "/api/v2/me/settings", adult, {"stats_visible_to_admin": False})
    check("withdrawing takes them out again", adult_id not in {m["user_id"] for m in ctx.call("GET", "/api/v2/family/stats", parent)})

    # A managed account (v1's restriction) may be supervised without its consent.
    ctx.call("PUT", f"/api/v1/family/members/{kid_id}/policy", parent, {"media_kind": "audiobook", "policy": "deny_all"})
    kid_feed = Feed(ctx, kid, "settings")
    check("an admin turns it on for an account they manage (PATCH /family/members/{id})",
          ctx.status_of("PATCH", f"/api/v2/family/members/{kid_id}", parent, {"stats_visible_to_admin": True}) == 204)
    fam = {m["user_id"]: m for m in ctx.call("GET", "/api/v2/family/stats", parent)}
    check("…and its figures are there", fam.get(kid_id, {}).get("listened_ms") == 60_000, str(fam.get(kid_id)))
    row = _last(kid_feed.read(), "settings", kid_id)
    check("…which the managed account's devices learn from the change feed", row is not None and row["record"]["stats_visible_to_admin"] is True)
    check("the range and kind apply to the household's figures too",
          {m["user_id"]: m for m in ctx.call("GET", "/api/v2/family/stats?kind=book", parent)}[parent_id]["listened_ms"] == 1_800_000)
    check("…and are checked the same way", _problem(ctx.request("GET", "/api/v2/family/stats?kind=album", parent), 400, "unsupported_kind"))
