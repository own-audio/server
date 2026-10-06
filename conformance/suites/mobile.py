# SPDX-License-Identifier: AGPL-3.0-or-later
"""Mobile-client surface: play queue, delta sync, grouped browsing, push, inbox, bulk.

Covers the pieces a phone app depends on and a web client does not: the
cross-device play queue, incremental sync with delete tombstones, grouped
music browsing, bulk operations, and push/notification registration. All of
it is platform-neutral — the same calls serve iOS and Android.

Ported from audio2/scripts/mobile_sync_test.py; the checks and their labels
are kept in the original order.
"""
from __future__ import annotations

import uuid

NAME = "mobile"
PW = "MobileTest123!"


def run(ctx) -> None:
    sfx = ctx.sfx
    uid, email, _ = ctx.make_user("mobile", PW)
    # Log in again with device attribution — the original test does this on
    # purpose, so the queue's `updated_by_device` has a real device behind it.
    tok = ctx.call("POST", "/api/v1/auth/login",
                   body={"email": email, "password": PW, "device_kind": "android",
                         "device_name": "Pixel 9"})["token"]

    def upload_track(title: str, artist: str = "Test Artist", album: str = "Test Album",
                     visibility: str = "private"):
        return ctx.upload_track(tok, title, artist, album, visibility, genre="Testing")

    try:
        # ── Play queue ───────────────────────────────────────────────────────
        ctx.log("\n[play queue]")
        empty = ctx.call("GET", "/api/v1/playback/queue", tok)
        ctx.check("empty queue returns a body, not 404", empty["items"] == [])

        t1 = upload_track(f"Queue One {sfx}")
        t2 = upload_track(f"Queue Two {sfx}")
        saved = ctx.call("PUT", "/api/v1/playback/queue", tok, {
            "items": [{"media_kind": "music", "item_id": t1["id"]},
                      {"media_kind": "music", "item_id": t2["id"]}],
            "current_index": 1, "position_secs": 42.5, "device_kind": "android"})
        ctx.check("queue saved with both items", len(saved["items"]) == 2)
        ctx.check("current index preserved", saved["current_index"] == 1)
        ctx.check("position preserved", abs(saved["position_secs"] - 42.5) < 0.01)
        ctx.check("writing device recorded", saved["updated_by_device"] == "android",
                  f"updated_by_device={saved.get('updated_by_device')!r}")

        fetched = ctx.call("GET", "/api/v1/playback/queue", tok)
        ctx.check("queue survives a round trip", fetched["items"][1]["item_id"] == t2["id"])

        # A stale index from another device must not strand the client.
        clamped = ctx.call("PUT", "/api/v1/playback/queue", tok, {
            "items": [{"media_kind": "music", "item_id": t1["id"]}],
            "current_index": 99, "device_kind": "ios"})
        ctx.check("out-of-range index is clamped, not rejected", clamped["current_index"] == 0,
                  f"current_index={clamped.get('current_index')!r}")
        ctx.call("PUT", "/api/v1/playback/queue", tok,
                 {"items": [{"media_kind": "cooking", "item_id": t1["id"]}]}, expect=(400,))
        ctx.check("unknown media kind rejected", True)

        # ── Delta sync ───────────────────────────────────────────────────────
        ctx.log("\n[delta sync]")
        full = ctx.call("GET", "/api/v1/library/changes", tok)
        ctx.check("first call is a full sync", full["full_sync"] is True)
        ctx.check("full sync carries the tracks", len({t["id"] for t in full["tracks"]}) >= 2)
        ctx.check("full sync returns no tombstones", full["deleted"] == [])
        cursor = full["now"]

        nothing = ctx.call("GET", f"/api/v1/library/changes?since={cursor}", tok)
        ctx.check("nothing changed since the cursor", nothing["tracks"] == [])
        ctx.check("incremental sync is flagged as such", nothing["full_sync"] is False)

        t3 = upload_track(f"Queue Three {sfx}")
        delta = ctx.call("GET", f"/api/v1/library/changes?since={cursor}", tok)
        ctx.check("only the new track comes back", [t["id"] for t in delta["tracks"]] == [t3["id"]],
                  f"tracks={[t['id'] for t in delta['tracks']]!r}")
        cursor2 = delta["now"]

        # A deletion must be reported, or the client keeps a ghost forever.
        ctx.call("DELETE", f"/api/v1/music/tracks/{t1['id']}", tok)
        after_delete = ctx.call("GET", f"/api/v1/library/changes?since={cursor2}", tok)
        tombstones = [d for d in after_delete["deleted"] if d["item_id"] == t1["id"]]
        ctx.check("deletion produces a tombstone", len(tombstones) == 1,
                  f"deleted={after_delete['deleted']!r}")
        ctx.check("tombstone names the media kind",
                  bool(tombstones) and tombstones[0]["media_kind"] == "music")

        ctx.call("GET", "/api/v1/library/changes?since=not-a-date", tok, expect=(400,))
        ctx.check("malformed cursor rejected", True)

        # ── Grouped browsing ─────────────────────────────────────────────────
        ctx.log("\n[grouped browsing]")
        artists = ctx.call("GET", "/api/v1/music/artists", tok)
        mine = next((a for a in artists if a["artist"] == "Test Artist"), None)
        ctx.check("artist appears with counts", mine is not None and mine["track_count"] >= 2,
                  f"artist={mine!r}")
        albums = ctx.call("GET", "/api/v1/music/albums?artist=Test%20Artist", tok)
        ctx.check("albums filter by artist", all(a["artist"] == "Test Artist" for a in albums))
        ctx.check("album reports its track count", bool(albums) and albums[0]["track_count"] >= 2,
                  f"albums={albums!r}")
        genres = ctx.call("GET", "/api/v1/music/genres", tok)
        ctx.check("genre appears", any(g["genre"] == "Testing" for g in genres))

        # ── Push registration ────────────────────────────────────────────────
        ctx.log("\n[push registration]")
        fcm = "fcm-token-" + uuid.uuid4().hex
        ctx.call("POST", "/api/v1/devices/push-token", tok,
                 {"platform": "fcm", "token": fcm, "device_name": "Pixel 9"})
        tokens = ctx.call("GET", "/api/v1/devices/push-tokens", tok)
        ctx.check("android token registered", any(t["platform"] == "fcm" for t in tokens))
        ctx.check("full token is never echoed back", all("token" not in t for t in tokens))

        apns = "apns-token-" + uuid.uuid4().hex
        ctx.call("POST", "/api/v1/devices/push-token", tok,
                 {"platform": "apns", "token": apns, "device_name": "iPhone"})
        ctx.check("both platforms coexist",
                  {t["platform"] for t in ctx.call("GET", "/api/v1/devices/push-tokens", tok)}
                  == {"fcm", "apns"})

        # Re-registering is idempotent — clients call this on every launch.
        ctx.call("POST", "/api/v1/devices/push-token", tok,
                 {"platform": "fcm", "token": fcm, "device_name": "Pixel 9"})
        ctx.check("re-registering does not duplicate",
                  len(ctx.call("GET", "/api/v1/devices/push-tokens", tok)) == 2)

        ctx.call("POST", "/api/v1/devices/push-token", tok,
                 {"platform": "windows-phone", "token": "x"}, expect=(400,))
        ctx.check("unknown platform rejected", True)

        ctx.call("DELETE", "/api/v1/devices/push-token", tok, {"token": fcm})
        ctx.check("token removed on sign-out",
                  len(ctx.call("GET", "/api/v1/devices/push-tokens", tok)) == 1)

        # ── Notification inbox (the polling fallback) ────────────────────────
        ctx.log("\n[notification inbox]")
        inbox = ctx.call("GET", "/api/v1/devices/notifications", tok)
        ctx.check("inbox starts empty", inbox == [], f"inbox={inbox!r}")
        acked = ctx.call("POST", "/api/v1/devices/notifications/ack", tok, {"ids": []})
        ctx.check("acking nothing is a no-op", acked["acknowledged"] == 0)

        # ── Bulk operations ──────────────────────────────────────────────────
        ctx.log("\n[bulk operations]")
        bulk = ctx.call("POST", "/api/v1/playback/episodes/progress/bulk", tok,
                        {"episode_ids": [], "completed": True})
        ctx.check("empty bulk request is a no-op", bulk["updated"] == 0)
        bulk = ctx.call("POST", "/api/v1/playback/episodes/progress/bulk", tok,
                        {"episode_ids": [str(uuid.uuid4())], "completed": True})
        ctx.check("unknown episode ids update nothing", bulk["updated"] == 0)

    finally:
        # make_user already registered the user for cleanup; deleting here as
        # well keeps the original's "clean up before anything else" ordering
        # and is harmless (the runner's cleanup tolerates 404).
        try:
            ctx.delete_user(uid)
        except Exception as e:  # noqa: BLE001 — cleanup is best effort
            ctx.log(f"  warn cleanup: {e}")
        ctx.log("\ncleanup done")
