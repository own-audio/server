# SPDX-License-Identifier: AGPL-3.0-or-later
"""The 30-day trash: delete, list, restore, purge, and who may do what.

Builds a family (owner = family admin, adult = member), deletes, lists,
restores, purges and checks permissions over the real API. Backdates items
and inspects stored objects through SQL, so it needs --compose-dir.
"""
from __future__ import annotations

import datetime
import hashlib
import json
import urllib.parse
import urllib.request
import uuid

from core import USER_AGENT, Ctx

NAME = "trash"
REQUIRES = {"db"}
PW = "TrashTest123!"


def ids(items):
    return {i["id"] for i in items}


def subsonic(ctx: Ctx, user_token, endpoint, **params):
    key = ctx.call("GET", "/api/v1/users/me/subsonic-key", user_token)
    salt = uuid.uuid4().hex[:8]
    q = {"u": key["username"], "t": hashlib.md5((key["api_key"] + salt).encode()).hexdigest(),
         "s": salt, "v": "1.16.1", "c": "trash-test", "f": "json", **params}
    req = urllib.request.Request(
        ctx.url(f"/rest/{endpoint}?{urllib.parse.urlencode(q)}"), headers={"User-Agent": USER_AGENT}
    )
    with urllib.request.urlopen(req, timeout=30) as r:
        return json.loads(r.read())["subsonic-response"]


def upload(ctx: Ctx, tok, title, visibility="family"):
    return ctx.call("POST", "/api/v1/music/tracks/upload", tok, multipart={
        "title": title, "artist": "Trash Test", "visibility": visibility,
        "file": (f"{uuid.uuid4().hex}.mp3", ctx.audio())})


def run(ctx: Ctx) -> None:
    call, sql, check, sfx, admin_tok = ctx.call, ctx.sql, ctx.check, ctx.sfx, ctx.admin_token

    owner_id, owner_email, owner_tok = ctx.make_user("owner", PW)
    adult_id, adult_email, adult_tok = ctx.make_user("adult", PW)
    created = [owner_id, adult_id]

    try:
        inv = call("POST", "/api/v1/family/invites", owner_tok, {"email": adult_email, "role": "member"})
        call("POST", "/api/v1/family/invites/accept", adult_tok, {"code": inv["code"]})
        check("family has 2 members", len(call("GET", "/api/v1/family", owner_tok)["members"]) == 2)

        # ── Delete moves to the trash ────────────────────────────────────────
        ctx.log("\n[delete → trash]")
        before = (datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(seconds=2)).isoformat().replace("+00:00", "Z")
        t = upload(ctx, owner_tok, f"Shared Song {sfx}")
        tid = t["id"]
        call("DELETE", f"/api/v1/music/tracks/{tid}", adult_tok, expect=(403,))
        check("a member cannot delete someone else's shared track (403)", True)

        batch = str(uuid.uuid4())
        call("DELETE", f"/api/v1/music/tracks/{tid}", owner_tok, headers={"X-Trash-Batch": batch})
        check("owner's delete answers 204", True)
        check("gone from the owner's list", tid not in ids(call("GET", "/api/v1/music/tracks", owner_tok)))
        check("gone from the member's list", tid not in ids(call("GET", "/api/v1/music/tracks", adult_tok)))
        call("GET", f"/api/v1/music/tracks/{tid}", owner_tok, expect=(404,))
        check("direct fetch is 404", True)
        hits = subsonic(ctx, owner_tok, "search3.view", query=f"Shared Song {sfx}")
        check("gone from Subsonic search3", not hits.get("searchResult3", {}).get("song"))
        call("DELETE", f"/api/v1/music/tracks/{tid}", owner_tok, expect=(404,))
        check("deleting it again is 404", True)

        changes = call("GET", f"/api/v1/library/changes?since={urllib.parse.quote(before)}", adult_tok)
        check("member's delta sync carries a tombstone",
              any(d["item_id"] == tid and d["media_kind"] == "music" for d in changes["deleted"]))

        trash = call("GET", "/api/v1/trash", owner_tok)
        row = next((r for r in trash if r["id"] == tid), None)
        check("listed in the owner's trash", row is not None)
        if row:
            purge_in = (datetime.datetime.fromisoformat(row["purge_at"].replace("Z", "+00:00"))
                        - datetime.datetime.fromisoformat(row["trashed_at"].replace("Z", "+00:00")))
            check("purge date is 30 days on", purge_in == datetime.timedelta(days=30), str(purge_in))
            check("batch id from the header is kept", row["batch"] == batch, str(row["batch"]))
            check("kind is music_track", row["kind"] == "music_track", str(row["kind"]))
            check("size is counted", row["size_bytes"] > 0, str(row["size_bytes"]))
            check("restoring the same day costs nothing", row["restore_charge_micro"] == 0, str(row["restore_charge_micro"]))
        call("GET", "/api/v1/trash?scope=family", adult_tok, expect=(403,))
        check("a member cannot see the family trash", True)
        check("the family admin sees it in the family trash",
              tid in ids(call("GET", "/api/v1/trash?scope=family", owner_tok)))
        check("the member's own trash is empty", call("GET", "/api/v1/trash", adult_tok) == [])
        call("POST", f"/api/v1/trash/music_track/{tid}/restore", adult_tok, expect=(404,))
        check("a member cannot restore someone else's trash (404)", True)

        # ── Restore ───────────────────────────────────────────────────────────
        ctx.log("\n[restore]")
        r = call("POST", f"/api/v1/trash/music_track/{tid}/restore", owner_tok)
        check("restore reports one item, no charge", r == {"restored": 1, "charged_micro": 0}, str(r))
        check("back in the member's list", tid in ids(call("GET", "/api/v1/music/tracks", adult_tok)))
        check("trash is empty again", call("GET", "/api/v1/trash", owner_tok) == [])
        changes = call("GET", f"/api/v1/library/changes?since={urllib.parse.quote(before)}", adult_tok)
        check("delta sync lists it again", any(x["id"] == tid for x in changes["tracks"]))

        # ── A family admin deletes a member's shared item ─────────────────────
        ctx.log("\n[family admin deletes a member's item]")
        mt = upload(ctx, adult_tok, f"Member Song {sfx}")
        private = upload(ctx, adult_tok, f"Member Private {sfx}", visibility="private")
        call("DELETE", f"/api/v1/music/tracks/{private['id']}", owner_tok, expect=(404,))
        check("a family admin cannot delete a member's private item (404)", True)
        call("DELETE", f"/api/v1/music/tracks/{mt['id']}", owner_tok)
        check("a family admin can delete a member's shared item", True)
        mine = call("GET", "/api/v1/trash", adult_tok)
        row = next((r for r in mine if r["id"] == mt["id"]), None)
        check("it is in the member's own trash", row is not None)
        check("trashed_by names the admin", row is not None and row["trashed_by"]["id"] == owner_id)
        notes = call("GET", "/api/v1/devices/notifications", adult_tok)
        check("the member is notified (item_trashed)",
              any(n.get("kind") == "item_trashed" and (n.get("data") or {}).get("id") == mt["id"] for n in notes))
        r = call("POST", f"/api/v1/trash/music_track/{mt['id']}/restore", adult_tok)
        check("the member can restore it", r["restored"] == 1, str(r))

        # ── Batch restore ─────────────────────────────────────────────────────
        ctx.log("\n[batch restore]")
        b = str(uuid.uuid4())
        a1, a2 = upload(ctx, owner_tok, f"Batch A {sfx}"), upload(ctx, owner_tok, f"Batch B {sfx}")
        for x in (a1, a2):
            call("DELETE", f"/api/v1/music/tracks/{x['id']}", owner_tok, headers={"X-Trash-Batch": b})
        r = call("POST", f"/api/v1/trash/batches/{b}/restore", owner_tok)
        check("both come back in one call", r["restored"] == 2, str(r))

        # ── Restore charges the days in the trash ─────────────────────────────
        ctx.log("\n[restore charge]")
        call("DELETE", f"/api/v1/music/tracks/{a1['id']}", owner_tok)
        sql(f"UPDATE media_objects SET size_bytes = 50000000000 WHERE id = "
            f"(SELECT audio_object_id FROM music_tracks_all WHERE id = '{a1['id']}')")
        sql(f"UPDATE music_tracks_all SET trashed_at = now() - interval '5 days 1 hour' WHERE id = '{a1['id']}'")
        row = next(r for r in call("GET", "/api/v1/trash", owner_tok) if r["id"] == a1["id"])
        # Only a billing edition charges for a restore; elsewhere the quote is 0 by contract.
        if ctx.feature("billing"):
            check("the list shows what a restore would cost", row["restore_charge_micro"] > 0, str(row["restore_charge_micro"]))
        else:
            check("the list shows a restore costs nothing without billing", row["restore_charge_micro"] == 0, str(row["restore_charge_micro"]))
        r = call("POST", f"/api/v1/trash/music_track/{a1['id']}/restore", owner_tok)
        check("restore charges exactly that", r["charged_micro"] == row["restore_charge_micro"],
              f"charged {r['charged_micro']}, listed {row['restore_charge_micro']}")
        ledger = sql(f"SELECT amount_micro FROM credit_ledger WHERE entry_type = 'trash_restore_charge' "
                     f"AND user_id = '{owner_id}'")
        if ctx.feature("billing"):
            check("a trash_restore_charge ledger entry was written", ledger == str(-r["charged_micro"]), ledger)
        else:
            check("no ledger entry without billing", ledger in ("", "0") , ledger)
        days = sql(f"SELECT days_in_trash FROM trash_restores WHERE item_id = '{a1['id']}' ORDER BY created_at DESC LIMIT 1")
        check("the restore is recorded for monitoring", days == "5", days)
        stats = call("GET", "/api/v1/admin/families/trash", admin_tok)
        fam_id = call("GET", "/api/v1/family", owner_tok)["id"]
        fam = next((s for s in stats if s["family_id"] == fam_id), None)
        check("admin trash stats list the family with its restores", fam is not None and fam["restores_30d"] >= 4, str(fam))

        # ── Delete forever ────────────────────────────────────────────────────
        ctx.log("\n[delete forever]")
        obj = sql(f"SELECT audio_object_id FROM music_tracks_all WHERE id = '{a2['id']}'")
        call("DELETE", f"/api/v1/music/tracks/{a2['id']}", owner_tok)
        call("DELETE", f"/api/v1/trash/music_track/{a2['id']}", owner_tok)
        check("the row is gone", sql(f"SELECT count(*) FROM music_tracks_all WHERE id = '{a2['id']}'") == "0")
        check("its media object is gone", sql(f"SELECT count(*) FROM media_objects WHERE id = '{obj}'") == "0")
        call("POST", f"/api/v1/trash/music_track/{a2['id']}/restore", owner_tok, expect=(404,))
        check("it can no longer be restored", True)

        # ── The purge job ─────────────────────────────────────────────────────
        ctx.log("\n[purge job]")
        call("DELETE", f"/api/v1/music/tracks/{tid}", owner_tok)
        sql(f"UPDATE music_tracks_all SET trashed_at = now() - interval '31 days' WHERE id = '{tid}'")
        sql("INSERT INTO jobs (job_type, payload) VALUES ('trash_purge', '{\"run_date\": \"test-" + sfx + "\"}')")
        ctx.wait_until(lambda: sql(f"SELECT count(*) FROM music_tracks_all WHERE id = '{tid}'") == "0", timeout=40, every=1)
        check("trash_purge deleted the 31-day-old item",
              sql(f"SELECT count(*) FROM music_tracks_all WHERE id = '{tid}'") == "0")

        # ── Books and playlists ───────────────────────────────────────────────
        ctx.log("\n[audiobooks + playlists]")
        book = call("POST", "/api/v1/audiobooks", owner_tok, {"title": f"Book {sfx}", "author": "T", "visibility": "family"})
        call("DELETE", f"/api/v1/audiobooks/{book['id']}", owner_tok)
        check("book leaves the list", book["id"] not in ids(call("GET", "/api/v1/audiobooks", adult_tok)))
        check("book is in the trash", book["id"] in ids(call("GET", "/api/v1/trash", owner_tok)))
        call("POST", f"/api/v1/trash/audiobook/{book['id']}/restore", owner_tok)
        check("book comes back", book["id"] in ids(call("GET", "/api/v1/audiobooks", adult_tok)))

        pl = call("POST", "/api/v1/music/playlists", owner_tok, {"name": f"List {sfx}"})
        call("DELETE", f"/api/v1/music/playlists/{pl['id']}", owner_tok)
        check("playlist leaves the list", pl["id"] not in ids(call("GET", "/api/v1/music/playlists", owner_tok)))
        call("POST", f"/api/v1/trash/playlist/{pl['id']}/restore", owner_tok)
        check("playlist comes back", pl["id"] in ids(call("GET", "/api/v1/music/playlists", owner_tok)))
        pl2 = call("POST", "/api/v1/music/playlists", owner_tok, {"name": f"Sub {sfx}"})
        res = subsonic(ctx, owner_tok, "deletePlaylist.view", id=pl2["id"])
        check("Subsonic deletePlaylist answers ok", res["status"] == "ok", str(res))
        check("…and moves it to the trash", pl2["id"] in ids(call("GET", "/api/v1/trash", owner_tok)))

        # ── A stored podcast episode (the admin's dev library) ────────────────
        ctx.log("\n[stored episode]")
        ep = sql("SELECT e.id || '|' || e.feed_id FROM podcast_episodes e JOIN podcast_feeds f ON f.id = e.feed_id "
                 f"JOIN users u ON u.id = f.user_id WHERE u.email = '{ctx.admin_email}' "
                 "AND e.audio_object_id IS NOT NULL LIMIT 1")
        if not ep:
            ctx.skip("stored episode", "no stored episode in the admin's library")
        else:
            ep_id, feed_id = ep.split("|")
            obj = sql(f"SELECT audio_object_id FROM podcast_episodes WHERE id = '{ep_id}'")
            r = call("DELETE", f"/api/v1/podcasts/{feed_id}/episodes/{ep_id}/download", admin_tok)
            check("the episode stays, no longer stored", r["id"] == ep_id and not r.get("has_local"), str(r))
            check("its object moved to the trashed column",
                  sql(f"SELECT trashed_audio_object_id FROM podcast_episodes WHERE id = '{ep_id}'") == obj)
            check("the object row still exists", sql(f"SELECT count(*) FROM media_objects WHERE id = '{obj}'") == "1")
            call("POST", f"/api/v1/trash/podcast_episode/{ep_id}/restore", admin_tok)
            check("restore puts the same object back",
                  sql(f"SELECT audio_object_id FROM podcast_episodes WHERE id = '{ep_id}'") == obj)

        # ── Deleting an account removes its objects at once ───────────────────
        ctx.log("\n[account deletion]")
        gone_id, gone_email, gone_tok = ctx.make_user("gone", PW)
        g = upload(ctx, gone_tok, f"Gone {sfx}", visibility="private")
        gobj = sql(f"SELECT audio_object_id FROM music_tracks_all WHERE id = '{g['id']}'")
        call("DELETE", "/api/v1/users/me", gone_tok)
        check("the deleted account's object is gone", sql(f"SELECT count(*) FROM media_objects WHERE id = '{gobj}'") == "0")
    finally:
        for uid in created:
            ctx.delete_user(uid)
