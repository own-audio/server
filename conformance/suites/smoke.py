# SPDX-License-Identifier: AGPL-3.0-or-later
"""Core smoke test: auth, users, read endpoints, playlists, media streams, refresh tokens, families, Subsonic, smart playlists.

Port of audio2's `scripts/backend_smoke_test.py`. The checks and their order
are the original's; each labelled step aborts the rest of the suite on its
first failed assertion, as the script did, and the temporary users it creates
are removed afterwards whatever happens.
"""
from __future__ import annotations

import hashlib
import json
import urllib.error
import urllib.parse
import urllib.request
import uuid
from contextlib import contextmanager
from typing import Any, Iterator

from core import USER_AGENT, ApiError, Ctx, Skip

NAME = "smoke"


class _StepFailed(AssertionError):
    """An assertion inside a labelled step did not hold."""


def need(cond: bool, detail: str) -> None:
    if not cond:
        raise _StepFailed(detail)


@contextmanager
def step(ctx: Ctx, label: str) -> Iterator[None]:
    """Record `label` as ok when the block completes, as FAIL (with the reason)
    when it does not — and then stop the suite, like the original script."""
    try:
        yield
    except Skip:
        raise
    except Exception as e:  # noqa: BLE001 — any failure inside a step fails that step
        ctx.fail(label, str(e))
        raise Skip(f"aborted after failed step '{label}'") from e
    ctx.ok(label)


# ── media discovery ─────────────────────────────────────────────────────────


def choose_streamable_audiobook(ctx: Ctx, token: str) -> tuple[str, str, str] | None:
    books = ctx.call("GET", "/api/v1/audiobooks", token)
    for book in books[:20]:
        book_id = book["id"]
        files = ctx.call("GET", f"/api/v1/audiobooks/{book_id}/files", token)
        for file_info in files:
            stream = ctx.call("GET", f"/api/v1/audiobooks/{book_id}/files/{file_info['id']}/stream", token)
            try:
                ctx.probe_media(stream["url"])
                return book_id, file_info["id"], stream["url"]
            except ApiError:
                continue
    return None


def choose_streamable_track(ctx: Ctx, token: str) -> tuple[str, str] | None:
    tracks = ctx.call("GET", "/api/v1/music/tracks", token)
    for track in tracks[:30]:
        track_id = track["id"]
        stream = ctx.call("GET", f"/api/v1/music/tracks/{track_id}/stream", token)
        try:
            ctx.probe_media(stream["url"])
            return track_id, stream["url"]
        except ApiError:
            continue
    return None


def choose_streamable_episode(ctx: Ctx, token: str) -> tuple[str, str, str] | None:
    feeds = ctx.call("GET", "/api/v1/podcasts", token)
    for feed in feeds[:15]:
        feed_id = feed["id"]
        episodes = ctx.call("GET", f"/api/v1/podcasts/{feed_id}/episodes?limit=25", token)
        for episode in episodes:
            if not episode.get("has_local"):
                continue
            episode_id = episode["id"]
            stream = ctx.call("GET", f"/api/v1/podcasts/{feed_id}/episodes/{episode_id}/stream", token)
            try:
                ctx.probe_media(stream["url"])
                return feed_id, episode_id, stream["url"]
            except ApiError:
                continue
    return None


def media_step(ctx: Ctx, label: str, what: str, found: Any) -> None:
    if found:
        ctx.ok(label, found[-1])
    elif ctx.require_media:
        ctx.fail(label, f"no streamable {what} found")
        raise Skip(f"aborted after failed step '{label}'")
    else:
        ctx.skip(label, f"no streamable {what} found")


# ── Subsonic ────────────────────────────────────────────────────────────────


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):  # noqa: D102
        return None


def check_subsonic_api(ctx: Ctx, username: str, api_key: str, track_id: str | None) -> None:
    """Exercise the OpenSubsonic-compatible /rest surface: token auth, ID3
    browsing, and (if a track is available) a stream redirect."""
    salt = uuid.uuid4().hex[:8]
    token = hashlib.md5((api_key + salt).encode("utf-8")).hexdigest()
    params = {"u": username, "t": token, "s": salt, "v": "1.16.1", "c": "smoke-test", "f": "json"}
    query = urllib.parse.urlencode(params)

    def get_json(endpoint: str) -> Any:
        payload = ctx.call("GET", f"/rest/{endpoint}?{query}")
        root = payload["subsonic-response"]
        need(root.get("status") == "ok", f"{endpoint} must return status ok: {json.dumps(payload)[:300]}")
        return root

    get_json("ping.view")
    get_json("getArtists.view")

    if track_id:
        # Ctx.call follows redirects, and the redirect target is the thing under test.
        stream_url = ctx.url(f"/rest/stream.view?{query}&id={track_id}")
        opener = urllib.request.build_opener(_NoRedirect())
        req = urllib.request.Request(stream_url, method="GET", headers={"User-Agent": USER_AGENT})
        try:
            resp = opener.open(req, timeout=ctx.timeout)
            status = resp.status
        except urllib.error.HTTPError as exc:
            status = exc.code
        need(status in (302, 303, 307), f"stream.view must redirect, got {status}")


# ── smart playlists ─────────────────────────────────────────────────────────


def check_smart_playlists(ctx: Ctx, token: str) -> None:
    """Resolve every built-in preset and a natural-language request.

    This exists because of a specific failure. The day-based filters
    (not_played_days, added_days) bound their parameter as a double, and
    `make_interval(days => ...)` takes an integer — so four of the five presets
    answered 500 against a real Postgres while every unit test stayed green,
    because none of them touch a database. Listing the presets was not enough:
    the bug was in resolving them.

    So: resolve each one. A preset that matches nothing is fine and expected on
    a small library — an empty list is a fact about the library. A non-200 is
    not.
    """
    presets = ctx.call("GET", "/api/v1/music/smart-playlists/presets", token)
    need(isinstance(presets, list) and len(presets) > 0, "presets must be listed")

    for preset in presets:
        slug = preset.get("slug", "?")
        resolved = ctx.call("POST", "/api/v1/music/smart-playlists/resolve", token, {"rule": preset["rule"]})
        need("tracks" in resolved, f"preset {slug} must resolve to a track list")
        need(isinstance(resolved.get("candidates"), int), f"preset {slug} must report how many tracks matched")

    # Text in, rule out — never tracks. The rule must then actually run.
    intent = ctx.call("POST", "/api/v1/music/intent", token, {"text": "pilates for two hours"})
    need("rule" in intent, "intent must return a rule")
    need("tracks" not in intent, "intent must return a rule, not tracks")
    resolved = ctx.call("POST", "/api/v1/music/smart-playlists/resolve", token, {"rule": intent["rule"]})
    need("tracks" in resolved, "an intent-produced rule must resolve")


# ── the suite ───────────────────────────────────────────────────────────────


def run(ctx: Ctx) -> None:
    with step(ctx, "health"):
        health = ctx.call("GET", "/health")
        need(health.get("status") == "ok", "health status must be ok")

    with step(ctx, "admin login"):
        admin_login = ctx.call("POST", "/api/v1/auth/login", body={"email": ctx.admin_email, "password": ctx.admin_password})
        admin_token = admin_login["token"]
        need(bool(admin_token), "admin login must return token")
        admin_me = ctx.call("GET", "/api/v1/auth/me", admin_token)
        need(admin_me["role"] == "admin", "seeded admin must have admin role")

    with step(ctx, "admin users"):
        users = ctx.call("GET", "/api/v1/users", admin_token)
        need(isinstance(users, list) and len(users) >= 1, "users list must contain at least one user")

    suffix = ctx.sfx
    test_email = f"smoke-{suffix}@example.com"
    test_password = "SmokePass123!"
    new_password = "SmokePass456!"

    with step(ctx, "admin create user"):
        created_user = ctx.call(
            "POST", "/api/v1/auth/admin-create-user", admin_token,
            {"email": test_email, "password": test_password, "display_name": f"Smoke User {suffix}", "role": "user"},
            expect=(200, 201),
        )
        created_user_id = created_user["id"]
        ctx.on_cleanup(f"user {test_email}", lambda: ctx.delete_user(created_user_id))

    with step(ctx, "user profile"):
        user_login = ctx.call("POST", "/api/v1/auth/login", body={"email": test_email, "password": test_password})
        user_token = user_login["token"]
        user_me = ctx.call("GET", "/api/v1/users/me", user_token)
        need(user_me["email"] == test_email, "users/me must match temp user")

        updated_name = f"Smoke User Updated {suffix}"
        ctx.call("PATCH", "/api/v1/users/me", user_token, {"display_name": updated_name}, expect=(200, 204))
        user_me_updated = ctx.call("GET", "/api/v1/users/me", user_token)
        need(user_me_updated["display_name"] == updated_name, "display name update must persist")

    with step(ctx, "read endpoints"):
        ctx.call("GET", "/api/v1/library/continue", user_token)
        ctx.call("GET", "/api/v1/library/search?q=smoke&limit=5", user_token)
        ctx.call("GET", "/api/v1/podcasts", user_token)
        ctx.call("GET", "/api/v1/audiobooks", user_token)
        ctx.call("GET", "/api/v1/music/tracks", user_token)
        ctx.call("GET", "/api/v1/music/playlists", user_token)
        ctx.call("GET", "/api/v1/playback/settings", user_token)

    admin_tracks = ctx.call("GET", "/api/v1/music/tracks", admin_token)

    # A sharing change has to reach /library/changes. It is keyed on updated_at, and the
    # sharing update used to leave that column alone — so every client kept showing the old
    # sharing state indefinitely, with nothing to notice it by.
    if admin_tracks:
        with step(ctx, "sharing reaches the sync feed"):
            share_track = admin_tracks[0]
            was = share_track.get("visibility", "private")
            flipped = "private" if was == "family" else "family"
            cursor = ctx.call("GET", "/api/v1/library/changes", admin_token)["now"]
            try:
                ctx.call(
                    "PUT", f"/api/v1/music/tracks/{share_track['id']}/visibility", admin_token,
                    {"visibility": flipped}, expect=(200, 204),
                )
                since = urllib.parse.quote(cursor, safe="")
                changes = ctx.call("GET", f"/api/v1/library/changes?since={since}", admin_token)
                reported = [t for t in changes.get("tracks", []) if t["id"] == share_track["id"]]
                need(bool(reported), "a sharing change must appear in /library/changes")
                need(reported[0]["visibility"] == flipped, "the change feed must carry the new sharing state")
            finally:
                ctx.call(
                    "PUT", f"/api/v1/music/tracks/{share_track['id']}/visibility", admin_token,
                    {"visibility": was}, expect=(200, 204),
                )
    else:
        ctx.skip("sharing reaches the sync feed", "no tracks available")

    if admin_tracks:
        with step(ctx, "track progress"):
            sample_track_id = admin_tracks[0]["id"]
            ctx.call(
                "PUT", f"/api/v1/music/tracks/{sample_track_id}/progress", admin_token,
                {"position_secs": 12.5, "completed": False},
            )
            progress = ctx.call("GET", f"/api/v1/music/tracks/{sample_track_id}/progress", admin_token)
            need(abs(float(progress["position_secs"]) - 12.5) < 0.001, "track progress must persist")
    else:
        ctx.skip("track progress", "no tracks available")

    with step(ctx, "playlist lifecycle"):
        playlist = ctx.call(
            "POST", "/api/v1/music/playlists", admin_token,
            {"name": f"Smoke Playlist {suffix}", "description": "created by smoke test"}, expect=(200, 201),
        )
        playlist_id = playlist["id"]
        try:
            ctx.call(
                "PUT", f"/api/v1/music/playlists/{playlist_id}", admin_token,
                {"name": f"Smoke Playlist Updated {suffix}", "description": "updated by smoke test"},
            )
            if admin_tracks:
                ctx.call(
                    "POST", f"/api/v1/music/playlists/{playlist_id}/tracks", admin_token,
                    {"track_id": admin_tracks[0]["id"]}, expect=(200, 201),
                )
                entries = ctx.call("GET", f"/api/v1/music/playlists/{playlist_id}/tracks", admin_token)
                need(len(entries) >= 1, "playlist should contain at least one entry after add")
        finally:
            ctx.call("DELETE", f"/api/v1/music/playlists/{playlist_id}", admin_token, expect=(200, 204))

    media_step(ctx, "audiobook stream", "audiobook file", choose_streamable_audiobook(ctx, admin_token))
    media_step(ctx, "music stream", "music track", choose_streamable_track(ctx, admin_token))
    media_step(ctx, "podcast stream", "podcast episode", choose_streamable_episode(ctx, admin_token))

    with step(ctx, "password change"):
        ctx.call(
            "POST", "/api/v1/auth/password", user_token,
            {"current_password": test_password, "new_password": new_password}, expect=(200, 204),
        )
        relogin = ctx.call("POST", "/api/v1/auth/login", body={"email": test_email, "password": new_password})
        need(bool(relogin["token"]), "relogin with new password must succeed")

    with step(ctx, "refresh token flow"):
        device_login = ctx.call(
            "POST", "/api/v1/auth/login",
            body={"email": test_email, "password": new_password, "device_name": "smoke-device", "device_kind": "other"},
        )
        first_refresh_token = device_login.get("refresh_token")
        need(bool(first_refresh_token), "login must return refresh_token")

        refreshed = ctx.call("POST", "/api/v1/auth/refresh", body={"refresh_token": first_refresh_token})
        need(bool(refreshed["token"]), "refresh must return a new access token")
        second_refresh_token = refreshed["refresh_token"]
        need(second_refresh_token != first_refresh_token, "refresh token must rotate")

        # Reusing the rotated (old) token must fail and revoke the chain…
        ctx.call("POST", "/api/v1/auth/refresh", body={"refresh_token": first_refresh_token}, expect=(401,))
        # …which also kills the newest token of that chain.
        ctx.call("POST", "/api/v1/auth/refresh", body={"refresh_token": second_refresh_token}, expect=(401,))

        sessions = ctx.call("GET", "/api/v1/auth/sessions", user_token)
        need(isinstance(sessions, list), "sessions must be a list")

        # Sign out a named device chain and confirm its refresh token dies.
        device_login2 = ctx.call(
            "POST", "/api/v1/auth/login",
            body={"email": test_email, "password": new_password, "device_name": "smoke-device-2", "device_kind": "other"},
        )
        sessions_after = ctx.call("GET", "/api/v1/auth/sessions", user_token)
        target = next((s for s in sessions_after if s.get("device_name") == "smoke-device-2"), None)
        need(target is not None, "smoke-device-2 must appear in sessions list")
        ctx.call("DELETE", f"/api/v1/auth/sessions/{target['chain_id']}", user_token, expect=(200, 204))
        ctx.call("POST", "/api/v1/auth/refresh", body={"refresh_token": device_login2["refresh_token"]}, expect=(401,))
        # The revoked device's access token must be rejected by the middleware.
        ctx.call("GET", "/api/v1/users/me", device_login2["token"], expect=(401,))

    with step(ctx, "jobs endpoint"):
        ctx.call("GET", "/api/v1/jobs", admin_token)

    with step(ctx, "family lifecycle"):
        # Every account is backfilled into a personal family of one.
        family = ctx.call("GET", "/api/v1/family", user_token)
        need(family["my_role"] == "family_admin", "own personal family must grant family_admin")
        need(len(family["members"]) == 1, "personal family must have exactly one member")

        ctx.call("PUT", "/api/v1/family", user_token, {"name": f"Smoke Family {suffix}"}, expect=(200, 204))
        renamed = ctx.call("GET", "/api/v1/family", user_token)
        need(renamed["name"] == f"Smoke Family {suffix}", "family rename must persist")

        # Invite a second account into the family.
        member_email = f"smoke-member-{suffix}@example.com"
        member_password = "SmokeMember123!"
        invite = ctx.call(
            "POST", "/api/v1/family/invites", user_token, {"email": member_email, "role": "member"}, expect=(200, 201),
        )
        need(bool(invite.get("code")), "invite must return a code")

        pending = ctx.call("GET", "/api/v1/family/invites", user_token)
        need(any(i["id"] == invite["id"] for i in pending), "invite must be listed as pending")

        # Registering with the code joins that family even on closed instances.
        member_reg = ctx.call(
            "POST", "/api/v1/auth/register",
            body={
                "email": member_email, "password": member_password,
                "display_name": f"Smoke Member {suffix}", "invite_code": invite["code"],
            },
            expect=(200, 201),
        )
        member_token = member_reg["token"]
        created_member_id = member_reg["user"]["id"]
        ctx.on_cleanup(f"user {member_email}", lambda: ctx.delete_user(created_member_id))

        member_family = ctx.call("GET", "/api/v1/family", member_token)
        need(member_family["id"] == family["id"], "invited member must land in the inviting family")
        need(member_family["my_role"] == "member", "invited member must have the invited role")
        need(len(member_family["members"]) == 2, "family must now have two members")

        # A used code cannot be redeemed twice.
        ctx.call("POST", "/api/v1/family/invites/accept", member_token, {"code": invite["code"]}, expect=(400,))

        # Members cannot perform family-admin actions.
        ctx.call("PUT", "/api/v1/family", member_token, {"name": "hijacked"}, expect=(403,))
        ctx.call("POST", "/api/v1/family/invites", member_token, {"email": "nope@example.com"}, expect=(403,))

        # The last family admin cannot be demoted.
        ctx.call(
            "PUT", f"/api/v1/family/members/{family['members'][0]['user_id']}", user_token,
            {"role": "member"}, expect=(400,),
        )

        # Labels and role promotion.
        ctx.call(
            "PUT", f"/api/v1/family/members/{created_member_id}", user_token,
            {"display_label": "Kid"}, expect=(200, 204),
        )
        labeled = ctx.call("GET", "/api/v1/family/members", user_token)
        kid = next(m for m in labeled if m["user_id"] == created_member_id)
        need(kid["display_label"] == "Kid", "display label must persist")

        # Removing a member drops them into a fresh personal family.
        ctx.call("DELETE", f"/api/v1/family/members/{created_member_id}", user_token, expect=(200, 204))
        after_removal = ctx.call("GET", "/api/v1/family", member_token)
        need(after_removal["id"] != family["id"], "removed member must leave the family")
        need(after_removal["my_role"] == "family_admin", "removed member owns their new family")
        need(len(after_removal["members"]) == 1, "removed member is alone again")

    if ctx.feature("subsonic"):
        with step(ctx, "subsonic api"):
            subsonic_key = ctx.call("GET", "/api/v1/users/me/subsonic-key", user_token)
            need(bool(subsonic_key.get("api_key")), "subsonic key must be created")
            # The temp user owns no tracks; ping/browse with their key only.
            check_subsonic_api(ctx, subsonic_key["username"], subsonic_key["api_key"], None)
            # The stream-redirect check needs a key whose user owns the track.
            admin_subsonic_key = ctx.call("GET", "/api/v1/users/me/subsonic-key", admin_token)
            check_subsonic_api(
                ctx, admin_subsonic_key["username"], admin_subsonic_key["api_key"],
                admin_tracks[0]["id"] if admin_tracks else None,
            )
    else:
        ctx.skip("subsonic api", "server reports features.subsonic = false")

    with step(ctx, "smart playlists"):
        check_smart_playlists(ctx, admin_token)
