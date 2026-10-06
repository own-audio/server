# SPDX-License-Identifier: AGPL-3.0-or-later
"""Listening history and the statistics board.

Covers the parts mobile clients depend on: batched reporting, idempotent
retries (Android WorkManager / iOS background tasks both retry), per-device
attribution, local-timezone day bucketing, and the privacy rules that decide
whether a family admin may see a member's figures.
"""
from __future__ import annotations

from datetime import datetime, timedelta, timezone

NAME = "stats"
PW = "StatsTest123!"


def span(minutes_ago, seconds, item, kind="music", device="android", sid=None, part=None):
    end = datetime.now(timezone.utc) - timedelta(minutes=minutes_ago)
    start = end - timedelta(seconds=seconds)
    s = {
        "media_kind": kind,
        "item_id": item,
        "started_at": start.isoformat(),
        "ended_at": end.isoformat(),
        "seconds_listened": seconds,
        "device_kind": device,
    }
    if sid:
        s["client_session_id"] = sid
    if part:
        s["part_id"] = part
    return s


def run(ctx):
    sfx = ctx.sfx
    call, check = ctx.call, ctx.check

    parent_id, parent_email, parent_tok = ctx.make_user("parent", PW)
    kid_id, kid_email, kid_tok = ctx.make_user("kid", PW)
    adult_id, adult_email, adult_tok = ctx.make_user("adult", PW)

    try:
        ctx.log("\n[setup] family of three")
        for email, tok in ((kid_email, kid_tok), (adult_email, adult_tok)):
            inv = call("POST", "/api/v1/family/invites", parent_tok, {"email": email, "role": "member"})
            call("POST", "/api/v1/family/invites/accept", tok, {"code": inv["code"]})
        book = call("POST", "/api/v1/audiobooks", parent_tok,
                    {"title": f"Stats Book {sfx}", "visibility": "family"})
        bid = book["id"]
        check("shared book created", book["visibility"] == "family")

        # ── Batched reporting ────────────────────────────────────────────────
        ctx.log("\n[batched reporting]")
        batch = [
            span(60, 600, bid, "audiobook", "android", sid=f"{sfx}-a1"),
            span(40, 900, bid, "audiobook", "android", sid=f"{sfx}-a2"),
            span(20, 300, bid, "audiobook", "ios", sid=f"{sfx}-a3"),
        ]
        res = call("POST", "/api/v1/playback/sessions", parent_tok, {"sessions": batch})
        check("batch of 3 accepted", res["recorded"] == 3 and res["received"] == 3)

        # ── Idempotency: the retry case that matters on mobile ───────────────
        ctx.log("\n[idempotent retry]")
        again = call("POST", "/api/v1/playback/sessions", parent_tok, {"sessions": batch})
        check("replayed batch records nothing new", again["recorded"] == 0)
        stats = call("GET", "/api/v1/stats/me?range=7d", parent_tok)
        check("total is 1800s, not doubled", stats["total_seconds"] == 1800)

        # ── Aggregates ───────────────────────────────────────────────────────
        ctx.log("\n[statistics board]")
        check("one media kind present", len(stats["by_kind"]) == 1)
        check("kind is audiobook", stats["by_kind"][0]["media_kind"] == "audiobook")
        check("session count is 3", stats["by_kind"][0]["sessions"] == 3)
        check("top item is the book", stats["top_items"][0]["item_id"] == bid)
        check("top item resolves its title", stats["top_items"][0]["title"] == f"Stats Book {sfx}")
        check("streak counts today", stats["streak_days"] >= 1)
        check("by_day has an entry", len(stats["by_day"]) >= 1)

        hist = call("GET", "/api/v1/stats/me/history?limit=10", parent_tok)
        check("history returns 3 entries", len(hist) == 3)
        check("device attribution survives", {h["device_kind"] for h in hist} == {"android", "ios"})
        check("entries are marked reported", all(h["source"] == "reported" for h in hist))

        # ── Timezone bucketing ───────────────────────────────────────────────
        ctx.log("\n[timezone]")
        utc = call("GET", "/api/v1/stats/me?range=7d&tz_offset_minutes=0", parent_tok)
        plus13 = call("GET", "/api/v1/stats/me?range=7d&tz_offset_minutes=780", parent_tok)
        check("totals are timezone-independent", utc["total_seconds"] == plus13["total_seconds"])
        call("GET", "/api/v1/stats/me?range=7d&tz_offset_minutes=999", parent_tok, expect=(400,))
        check("absurd offsets are rejected", True)
        call("GET", "/api/v1/stats/me?range=bogus", parent_tok, expect=(400,))
        check("bad range is rejected", True)

        # ── Validation ───────────────────────────────────────────────────────
        ctx.log("\n[validation]")
        bad = span(10, 60, bid, "audiobook")
        bad["ended_at"], bad["started_at"] = bad["started_at"], bad["ended_at"]
        call("POST", "/api/v1/playback/sessions", parent_tok, {"sessions": [bad]}, expect=(400,))
        check("reversed time span rejected", True)
        call("POST", "/api/v1/playback/sessions", parent_tok,
             {"sessions": [span(10, 60, bid, "cooking")]}, expect=(400,))
        check("unknown media kind rejected", True)
        empty = call("POST", "/api/v1/playback/sessions", parent_tok, {"sessions": []})
        check("empty batch is a no-op", empty["recorded"] == 0)

        # ── Derived sessions for clients that only save progress ─────────────
        # The adult never posts explicit sessions, so their progress saves must
        # still produce statistics — this is how today's web UI and Subsonic
        # apps get a board without any client change.
        ctx.log("\n[derived from progress]")
        track = ctx.upload_track(adult_tok, f"Derived Track {sfx}")
        call("PUT", f"/api/v1/music/tracks/{track['id']}/progress", adult_tok,
             {"position_secs": 90.0})
        adult_stats = call("GET", "/api/v1/stats/me?range=7d", adult_tok)
        check("progress save produced a derived session", adult_stats["total_seconds"] == 90)
        adult_hist = call("GET", "/api/v1/stats/me/history", adult_tok)
        check("derived entries are labelled", all(h["source"] == "derived" for h in adult_hist))

        # A further save only counts the delta, not the absolute position.
        call("PUT", f"/api/v1/music/tracks/{track['id']}/progress", adult_tok,
             {"position_secs": 150.0})
        adult_stats = call("GET", "/api/v1/stats/me?range=7d", adult_tok)
        check("second save adds only the delta", adult_stats["total_seconds"] == 150)

        # Seeking backwards is not listening.
        call("PUT", f"/api/v1/music/tracks/{track['id']}/progress", adult_tok,
             {"position_secs": 10.0})
        adult_stats = call("GET", "/api/v1/stats/me?range=7d", adult_tok)
        check("rewinding adds nothing", adult_stats["total_seconds"] == 150)

        # Once a client reports explicitly, derivation stops so the two sources
        # cannot both count the same playback.
        call("POST", "/api/v1/playback/sessions", adult_tok,
             {"sessions": [span(5, 60, track["id"], "music", "android", sid=f"{sfx}-d1")]})
        call("PUT", f"/api/v1/music/tracks/{track['id']}/progress", adult_tok,
             {"position_secs": 600.0})
        adult_stats = call("GET", "/api/v1/stats/me?range=7d", adult_tok)
        check("derivation stops once the client reports sessions",
              adult_stats["total_seconds"] == 210)

        # ── Privacy (decision D2) ────────────────────────────────────────────
        ctx.log("\n[stats privacy]")
        fam = call("GET", "/api/v1/stats/family", parent_tok)
        others = {e["user_id"]: e for e in fam if e["user_id"] != parent_id}
        check("members are listed", len(others) == 2)
        check("members' figures are hidden by default", all(e["hidden"] for e in others.values()))
        check("hidden members expose no totals",
              all(e.get("total_seconds") is None for e in others.values()))
        check("admin still sees their own figures",
              next(e for e in fam if e["user_id"] == parent_id)["total_seconds"] == 1800)

        call("GET", "/api/v1/stats/family", kid_tok, expect=(401, 403))
        check("plain member cannot read the family roll-up", True)

        # An adult keeps control: the parent cannot expose them unilaterally.
        call("PUT", f"/api/v1/stats/family/members/{adult_id}/visibility", parent_tok,
             {"stats_visibility": "family_admin"}, expect=(403,))
        check("admin cannot expose an unrestricted adult", True)

        # The adult may opt in themselves.
        call("PUT", "/api/v1/stats/me/visibility", adult_tok, {"stats_visibility": "family_admin"})
        fam = call("GET", "/api/v1/stats/family", parent_tok)
        check("adult becomes visible after opting in",
              not next(e for e in fam if e["user_id"] == adult_id)["hidden"])

        # A restricted (managed) account may be supervised without consent.
        call("PUT", f"/api/v1/family/members/{kid_id}/policy", parent_tok,
             {"media_kind": "audiobook", "policy": "deny_all"})
        call("PUT", f"/api/v1/stats/family/members/{kid_id}/visibility", parent_tok,
             {"stats_visibility": "family_admin"})
        fam = call("GET", "/api/v1/stats/family", parent_tok)
        check("restricted member can be supervised",
              not next(e for e in fam if e["user_id"] == kid_id)["hidden"])

        # …and can withdraw again only if the restriction is lifted.
        call("PUT", "/api/v1/stats/me/visibility", adult_tok, {"stats_visibility": "private"})
        fam = call("GET", "/api/v1/stats/family", parent_tok)
        check("adult can withdraw consent",
              next(e for e in fam if e["user_id"] == adult_id)["hidden"])

    finally:
        # Users are deleted by ctx.make_user's cleanup; the family-visibility
        # changes above live on those users and go with them.
        ctx.log("\ncleanup registered")
