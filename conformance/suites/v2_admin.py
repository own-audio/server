# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 running the server (completion plan phase 12): setup once, accounts, households, the server's figures, background jobs, library folders, the disc-number back-fill job.

The permission matrix first: every `/admin` operation is `401` without a token and `403` to a
member and to a household admin who does not run the server; the library folders are read by any
member and scanned by a household admin. Then: `POST /setup` refused once the server has its
administrator (`409 setup_complete`; its success is checked by `run.py` on a fresh stack);
accounts made (`Idempotency-Key`, `email_taken`, `weak_password`, `invalid_role`), found by `q`,
paged, renamed, made and unmade administrators, blocked (refused at sign-in, signed out) and let
in again, signed out everywhere, deleted; the administrator keeps their own rights
(`409 own_account`) and household (`409 own_family`); households listed by state, read with their
people, storage and trash, a household with people not deleted (`409 family_not_empty`); with
`--compose-dir`, a second household made through SQL is locked (its people cannot sign in) and let
in again, then deleted once empty, and households page without loss. The server's figures; jobs
filtered by kind and state and paged without loss while new ones arrive; with `--compose-dir`, a
track marked through SQL as never read for its disc number is read by the `disc_backfill` job,
which reports `{checked, left}` in the jobs list.
"""
from __future__ import annotations

import json
import struct
import urllib.parse
import uuid

from core import Ctx
from suites.v2_books import _problem

NAME = "v2_admin"
PW = "V2AdminTest12345!"


def _token(ctx: Ctx, email: str, password: str):
    data = urllib.parse.urlencode({"grant_type": "password", "username": email, "password": password,
                                   "device_name": "Conformance"}).encode()
    return ctx.request("POST", "/api/v2/auth/token", body=data, headers={"Content-Type": "application/x-www-form-urlencoded"})


def _signs_in(ctx: Ctx, email: str, password: str) -> str | None:
    status, _h, raw = _token(ctx, email, password)
    return json.loads(raw)["access_token"] if status == 200 else None


def _tagged_wav(disc: int, total: int) -> bytes:
    """A second of silence as WAV with an ID3v2.3 tag saying which disc it is on."""
    text = b"\x00" + f"{disc}/{total}".encode()
    frame = b"TPOS" + struct.pack(">I", len(text)) + b"\x00\x00" + text
    body = frame + b"\x00" * 16
    size = len(body)
    synchsafe = bytes([(size >> 21) & 0x7F, (size >> 14) & 0x7F, (size >> 7) & 0x7F, size & 0x7F])
    id3 = b"ID3\x03\x00\x00" + synchsafe + body
    rate, frames = 8000, 8000
    pcm = b"".join(struct.pack("<h", (i % 50) * 100) for i in range(frames))
    fmt = b"fmt " + struct.pack("<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16)
    data = b"data" + struct.pack("<I", len(pcm)) + pcm
    chunk = b"id3 " + struct.pack("<I", len(id3)) + id3 + (b"\x00" if len(id3) % 2 else b"")
    riff = b"WAVE" + fmt + data + chunk
    return b"RIFF" + struct.pack("<I", len(riff)) + riff


def run(ctx: Ctx) -> None:
    me = ctx.call("GET", "/api/v2/me", ctx.admin_token)
    matrix(ctx)
    setup(ctx)
    accounts(ctx, me)
    households(ctx, me)
    figures(ctx)
    jobs(ctx)
    folders(ctx)
    quarantine(ctx)
    if ctx.db_available:
        second_household(ctx)
        disc_backfill(ctx)
    else:
        ctx.skip("a second household locked and let in again", "needs --compose-dir (a household made through SQL)")
        ctx.skip("the disc-number back-fill job", "needs --compose-dir (a track marked through SQL)")


ADMIN_OPS = [
    ("GET", "/api/v2/admin/users"), ("POST", "/api/v2/admin/users"),
    ("GET", "/api/v2/admin/users/{id}"), ("PATCH", "/api/v2/admin/users/{id}"), ("DELETE", "/api/v2/admin/users/{id}"),
    ("DELETE", "/api/v2/admin/users/{id}/sessions"),
    ("GET", "/api/v2/admin/families"), ("GET", "/api/v2/admin/families/{id}"),
    ("PATCH", "/api/v2/admin/families/{id}"), ("DELETE", "/api/v2/admin/families/{id}"),
    ("GET", "/api/v2/admin/stats"), ("GET", "/api/v2/admin/jobs"), ("GET", "/api/v2/admin/jobs/{id}"),
]


def matrix(ctx: Ctx) -> None:
    _aid, _ae, family_admin = ctx.make_family_admin("hadmin", PW)
    _mid, _me, member = ctx.make_member("member", PW, family_admin)
    fam = ctx.call("GET", "/api/v2/me", family_admin)
    ctx.check("the household admin of this check runs a household, not the server",
              fam["family"]["role"] == "admin" and fam["instance_role"] == "user")
    body = {"email": f"nobody-{ctx.sfx}@example.com", "password": PW, "display_name": "Nobody", "locked": True}
    wrong = []
    for method, path in ADMIN_OPS:
        # The server's own administrator's id, so a refusal is about the caller, not the id.
        filled = path.replace("{id}", ctx.call("GET", "/api/v2/me", ctx.admin_token)["id"]) if "users/{id}" in path \
            else path.replace("{id}", str(uuid.uuid4()))
        for who, token, want in (("anonymous", None, 401), ("member", member, 403), ("household admin", family_admin, 403)):
            answer = ctx.request(method, filled, token, body if method in ("POST", "PATCH") else None)
            if not _problem(answer, want, "unauthorized" if want == 401 else "forbidden"):
                wrong.append(f"{who} {method} {path} → {answer[0]}")
    ctx.check(f"every /admin operation: 401 anonymous, 403 forbidden to a member and to a household admin ({len(ADMIN_OPS)} operations)",
              not wrong, "; ".join(wrong[:8]))
    ctx.check("a member reads the library folders → 200", ctx.status_of("GET", "/api/v2/library/folders", member) == 200)
    ctx.check("a member may not start a scan → 403",
              _problem(ctx.request("POST", "/api/v2/library/folders/scan", member), 403, "forbidden"))
    ctx.check("a household admin starts a scan → 202", ctx.status_of("POST", "/api/v2/library/folders/scan", family_admin) == 202)


def setup(ctx: Ctx) -> None:
    body = {"email": f"late-{ctx.sfx}@example.com", "password": PW, "display_name": "Too late"}
    ctx.check("POST /setup once the server has its administrator → 409 setup_complete",
              _problem(ctx.request("POST", "/api/v2/setup", None, body), 409, "setup_complete"))
    ctx.check("…whatever the body says → 409 setup_complete",
              _problem(ctx.request("POST", "/api/v2/setup", None, {"email": "x", "password": "y", "display_name": ""}), 409, "setup_complete"))
    ctx.check("…and no account was made",
              not ctx.call("GET", f"/api/v2/admin/users?q=late-{ctx.sfx}", ctx.admin_token)["items"])
    doc = ctx.call("GET", "/.well-known/own-audio")
    ctx.check("discovery: setup.complete is true", (doc.get("setup") or {}).get("complete") is True)


def accounts(ctx: Ctx, me: dict) -> None:
    admin = ctx.admin_token
    sfx = ctx.sfx
    email = f"v2admin-made-{sfx}@example.com"
    body = {"email": email, "password": PW, "display_name": f"Made {sfx}"}
    key = {"Idempotency-Key": f"admin-user-{sfx}"}
    first = ctx.request("POST", "/api/v2/admin/users", admin, body, headers=key)
    made = json.loads(first[2]) if first[0] == 201 else {}
    if made.get("id"):
        ctx.track_user(made["id"], email)
    # The server's one household, or one of their own on a server with several.
    family = (made.get("family") or {}).get("id")
    ctx.check("POST /admin/users → 201 AdminUser, a user in a household, address confirmed",
              first[0] == 201 and made.get("email") == email and made.get("instance_role") == "user"
              and made.get("blocked") is False and made.get("email_verified") is True and made.get("pending") is False
              and (family == me["family"]["id"] if ctx.feature("one_family") else bool(family) and family != me["family"]["id"]))
    again = ctx.request("POST", "/api/v2/admin/users", admin, body, headers=key)
    ctx.check("the same Idempotency-Key answers the first account and makes nothing more",
              again[0] == 201 and json.loads(again[2]).get("id") == made.get("id")
              and len(ctx.call("GET", f"/api/v2/admin/users?q={urllib.parse.quote(email)}", admin)["items"]) == 1)
    ctx.check("…the same key with another body → 422 idempotency_key_reused",
              _problem(ctx.request("POST", "/api/v2/admin/users", admin, {**body, "display_name": "Other"}, headers=key),
                       422, "idempotency_key_reused"))
    ctx.check("an address in use → 409 email_taken",
              _problem(ctx.request("POST", "/api/v2/admin/users", admin, {**body, "email": email.upper()}), 409, "email_taken"))
    ctx.check("a short password → 400 weak_password",
              _problem(ctx.request("POST", "/api/v2/admin/users", admin, {**body, "email": f"w-{email}", "password": "short"}), 400, "weak_password"))
    ctx.check("no address → 400 invalid_email",
              _problem(ctx.request("POST", "/api/v2/admin/users", admin, {**body, "email": "nobody"}), 400, "invalid_email"))
    ctx.check("an unknown role → 400 invalid_role",
              _problem(ctx.request("POST", "/api/v2/admin/users", admin, {**body, "email": f"r-{email}", "instance_role": "root"}), 400, "invalid_role"))
    uid = made["id"]
    token = _signs_in(ctx, email, PW)
    ctx.check("the account signs in with its password", token is not None)

    for i in range(3):
        ctx.call("POST", "/api/v2/admin/users", admin, {"email": f"v2admin-page{i}-{sfx}@example.com", "password": PW,
                                                         "display_name": f"Pager {i} {sfx}"}, expect=(201,))
        ctx.track_user(ctx.call("GET", f"/api/v2/admin/users?q=v2admin-page{i}-{sfx}", admin)["items"][0]["id"])
    found = ctx.call("GET", f"/api/v2/admin/users?q={urllib.parse.quote(f'PAGER 1 {sfx}')}", admin)["items"]
    ctx.check("q= finds an account by a part of its name, any case", [u["display_name"] for u in found] == [f"Pager 1 {sfx}"])
    full = ctx.call("GET", "/api/v2/admin/users?limit=500", admin)
    small, cursor, pages = [], None, 0
    while True:
        page = ctx.call("GET", "/api/v2/admin/users?limit=2" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else ""), admin)
        small += page["items"]
        pages += 1
        cursor = page.get("next")
        if not cursor or pages > 500:
            break
    ids = [u["id"] for u in full["items"]]
    ctx.check("GET /admin/users pages without loss or repeats",
              [u["id"] for u in small] == ids and len(set(ids)) == len(ids) and pages > 1 and me["id"] in ids,
              f"{len(small)} in {pages} pages against {len(ids)}")
    ctx.check("a bad cursor → 400 invalid_cursor",
              _problem(ctx.request("GET", "/api/v2/admin/users?cursor=%25%25", admin), 400, "invalid_cursor"))

    got = ctx.call("GET", f"/api/v2/admin/users/{uid}", admin)
    ctx.check("GET /admin/users/{id} → the account", got["id"] == uid and got["email"] == email)
    ctx.check("an unknown account → 404", _problem(ctx.request("GET", f"/api/v2/admin/users/{uuid.uuid4()}", admin), 404, "not_found"))
    renamed = ctx.call("PATCH", f"/api/v2/admin/users/{uid}", admin, {"display_name": f"Renamed {sfx}"})
    ctx.check("PATCH {display_name} → the account renamed", renamed["display_name"] == f"Renamed {sfx}")
    ctx.check("a blank name → 400 invalid_display_name",
              _problem(ctx.request("PATCH", f"/api/v2/admin/users/{uid}", admin, {"display_name": " "}), 400, "invalid_display_name"))
    promoted = ctx.call("PATCH", f"/api/v2/admin/users/{uid}", admin, {"instance_role": "admin"})
    old = token
    token = _signs_in(ctx, email, PW)
    ctx.check("PATCH {instance_role: admin} → signed out, and signed in again they run the server",
              promoted["instance_role"] == "admin" and ctx.status_of("GET", "/api/v2/me", old) == 401
              and token is not None and ctx.status_of("GET", "/api/v2/admin/stats", token) == 200)
    demoted = ctx.call("PATCH", f"/api/v2/admin/users/{uid}", admin, {"instance_role": "user"})
    ctx.check("PATCH {instance_role: user} → their administrator's token stops at once",
              demoted["instance_role"] == "user" and ctx.status_of("GET", "/api/v2/admin/stats", token) == 401)
    token = _signs_in(ctx, email, PW)
    ctx.check("…and signed in again they no longer run the server",
              token is not None and ctx.status_of("GET", "/api/v2/admin/stats", token) == 403)
    ctx.check("the same role again signs nobody out",
              ctx.call("PATCH", f"/api/v2/admin/users/{uid}", admin, {"instance_role": "user"})["instance_role"] == "user"
              and ctx.status_of("GET", "/api/v2/me", token) == 200)
    blocked = ctx.call("PATCH", f"/api/v2/admin/users/{uid}", admin, {"blocked": True})
    ctx.check("PATCH {blocked: true} → blocked, signed out, and refused at sign-in",
              blocked["blocked"] is True and ctx.status_of("GET", "/api/v2/me", token) == 401 and _signs_in(ctx, email, PW) is None)
    ctx.call("PATCH", f"/api/v2/admin/users/{uid}", admin, {"blocked": False})
    token = _signs_in(ctx, email, PW)
    ctx.check("PATCH {blocked: false} → they sign in again", token is not None and ctx.status_of("GET", "/api/v2/me", token) == 200)

    for label, patch in (("demote", {"instance_role": "user"}), ("block", {"blocked": True})):
        ctx.check(f"an administrator does not {label} themselves → 409 own_account",
                  _problem(ctx.request("PATCH", f"/api/v2/admin/users/{me['id']}", admin, patch), 409, "own_account"))
    ctx.check("…nor delete themselves here → 409 own_account",
              _problem(ctx.request("DELETE", f"/api/v2/admin/users/{me['id']}", admin), 409, "own_account"))
    ctx.check("…and still runs the server", ctx.call("GET", "/api/v2/me", admin)["instance_role"] == "admin")

    ctx.check("DELETE /admin/users/{id}/sessions → 204, and their token stops working",
              ctx.status_of("DELETE", f"/api/v2/admin/users/{uid}/sessions", admin) == 204 and ctx.status_of("GET", "/api/v2/me", token) == 401)
    ctx.check("…of an unknown account → 404",
              _problem(ctx.request("DELETE", f"/api/v2/admin/users/{uuid.uuid4()}/sessions", admin), 404, "not_found"))
    ctx.check("DELETE /admin/users/{id} → 204, then 404",
              ctx.status_of("DELETE", f"/api/v2/admin/users/{uid}", admin) == 204
              and _problem(ctx.request("GET", f"/api/v2/admin/users/{uid}", admin), 404, "not_found")
              and _problem(ctx.request("DELETE", f"/api/v2/admin/users/{uid}", admin), 404, "not_found"))
    ctx.check("…and the account cannot sign in", _signs_in(ctx, email, PW) is None)


def households(ctx: Ctx, me: dict) -> None:
    admin = ctx.admin_token
    own = me["family"]["id"]
    _uid, _e, owner = ctx.make_member("trashowner", PW, admin)
    track = ctx.upload_track(owner, f"Admin Trash {ctx.sfx}", content=ctx.tone(), content_type="audio/wav")
    ctx.call("DELETE", f"/api/v2/tracks/{track['id']}", owner, expect=(200, 202, 204))

    listed = ctx.call("GET", "/api/v2/admin/families?limit=500", admin)["items"]
    mine = next((f for f in listed if f["id"] == own), None)
    ctx.check("GET /admin/families lists the administrator's household, with its people and trash",
              mine is not None and mine["member_count"] >= 2 and mine["locked"] is False and mine["trash"]["items"] >= 1)
    in_state = lambda state: {f["id"] for f in ctx.call("GET", f"/api/v2/admin/families?state={state}&limit=500", admin)["items"]}  # noqa: E731
    ctx.check("state=active has it, state=locked does not, state=trashed has it (something in its trash)",
              own in in_state("active") and own not in in_state("locked") and own in in_state("trashed"))
    ctx.check("an unknown state → 400 invalid_state",
              _problem(ctx.request("GET", "/api/v2/admin/families?state=deleted", admin), 400, "invalid_state"))
    detail = ctx.call("GET", f"/api/v2/admin/families/{own}", admin)
    members = {m["user_id"]: m for m in detail["members"]}
    ctx.check("GET /admin/families/{id} → its people, storage and trash",
              me["id"] in members and members[me["id"]]["blocked"] is False
              and all(isinstance(detail["storage"].get(k), int) for k in ("total_bytes", "music_bytes", "audiobooks_bytes"))
              and detail["trash"]["bytes"] >= 0 and isinstance(detail["trash"]["flagged"], bool))
    ctx.check("an unknown household → 404", _problem(ctx.request("GET", f"/api/v2/admin/families/{uuid.uuid4()}", admin), 404, "not_found"))
    ctx.check("locking the household one is in → 409 own_family",
              _problem(ctx.request("PATCH", f"/api/v2/admin/families/{own}", admin, {"locked": True}), 409, "own_family"))
    ctx.check("…and nobody in it was locked out",
              ctx.status_of("GET", "/api/v2/me", admin) == 200 and ctx.status_of("GET", "/api/v2/me", owner) == 200
              and not ctx.call("GET", f"/api/v2/admin/families/{own}", admin)["locked"])
    ctx.check("deleting a household with people → 409 family_not_empty",
              _problem(ctx.request("DELETE", f"/api/v2/admin/families/{own}", admin), 409, "family_not_empty"))
    ctx.check("…an unknown one → 404", _problem(ctx.request("DELETE", f"/api/v2/admin/families/{uuid.uuid4()}", admin), 404, "not_found"))
    ctx.check("PATCH an unknown one → 404",
              _problem(ctx.request("PATCH", f"/api/v2/admin/families/{uuid.uuid4()}", admin, {"locked": False}), 404, "not_found"))


def quarantine(ctx: Ctx) -> None:
    admin = ctx.admin_token
    _mid, _me, member = ctx.make_member("quarantine-member", PW, admin)
    # Random bytes do not decode: the fingerprint job sets the song aside. Through SQL when the
    # job is off or slow.
    track = ctx.upload_track(admin, f"Broken {ctx.sfx}")
    listed = lambda: [q for q in ctx.call("GET", "/api/v2/library/quarantine", admin)["items"] if q["track_id"] == track["id"]]  # noqa: E731
    if not ctx.wait_until(lambda: bool(listed()), timeout=30) and ctx.db_available:
        ctx.sql(f"UPDATE music_tracks_all SET quarantined_at = now(), quarantine_reason = 'not_decoding' WHERE id = '{track['id']}'")
    held = listed()
    if not held:
        ctx.skip("GET /library/quarantine, and letting a track out", "the fingerprint job did not set the song aside and there is no --compose-dir")
        ctx.call("DELETE", f"/api/v2/tracks/{track['id']}", admin, expect=(200, 202, 204))
        return
    ctx.check("GET /library/quarantine → the household's set-aside songs, with file and reason",
              held[0]["title"] == f"Broken {ctx.sfx}" and held[0]["reason"] and held[0]["file"] and held[0]["since"], str(held[0]))
    ctx.check("…a member may not see it → 403", _problem(ctx.request("GET", "/api/v2/library/quarantine", member), 403, "forbidden"))
    ctx.check("…nor let a song out → 403",
              _problem(ctx.request("DELETE", f"/api/v2/library/quarantine/{track['id']}", member), 403, "forbidden"))
    ctx.check("DELETE /library/quarantine/{track_id} → 204, the song is back",
              ctx.status_of("DELETE", f"/api/v2/library/quarantine/{track['id']}", admin) == 204 and not listed()
              and ctx.status_of("GET", f"/api/v2/tracks/{track['id']}", admin) == 200)
    ctx.check("…one not in quarantine → 404",
              _problem(ctx.request("DELETE", f"/api/v2/library/quarantine/{track['id']}", admin), 404, "not_found"))
    ctx.call("DELETE", f"/api/v2/tracks/{track['id']}", admin, expect=(200, 202, 204))


def figures(ctx: Ctx) -> None:
    s = ctx.call("GET", "/api/v2/admin/stats", ctx.admin_token)
    windows = lambda w: all(isinstance(w.get(k), int) for k in ("d1", "d7", "d30"))  # noqa: E731
    ctx.check("GET /admin/stats → accounts, households, storage, trash, sign-ins, listening in ms",
              s["users"] >= 1 and s["families"] >= 1 and windows(s["registrations"]) and windows(s["sign_ins"])
              and s["storage"]["total_bytes"] == s["storage"]["audio_bytes"] + s["storage"]["other_bytes"]
              and isinstance(s["trash_bytes"], int)
              and all(isinstance(s["listening"][w].get("listened_ms"), int) for w in ("d1", "d7", "d30")))


def jobs(ctx: Ctx) -> None:
    admin = ctx.admin_token
    first = ctx.call("GET", "/api/v2/admin/jobs?limit=500", admin)["items"]
    ctx.check("GET /admin/jobs → jobs newest first, with kind and state",
              bool(first) and all(j.get("kind") and j.get("state") in ("pending", "running", "completed", "failed") for j in first)
              and [j["created_at"] for j in first] == sorted((j["created_at"] for j in first), reverse=True))
    if first:
        one = ctx.call("GET", f"/api/v2/admin/jobs/{first[-1]['id']}", admin)
        ctx.check("GET /admin/jobs/{id} → that job", one["id"] == first[-1]["id"] and one["kind"] == first[-1]["kind"])
    ctx.check("an unknown job → 404", _problem(ctx.request("GET", f"/api/v2/admin/jobs/{uuid.uuid4()}", admin), 404, "not_found"))
    ctx.check("an unknown state → 400 invalid_state", _problem(ctx.request("GET", "/api/v2/admin/jobs?state=done", admin), 400, "invalid_state"))
    ctx.check("a kind that is not one → 400 invalid_kind", _problem(ctx.request("GET", "/api/v2/admin/jobs?kind=Feed%20Refresh", admin), 400, "invalid_kind"))
    kinds: dict[str, int] = {}
    for j in first:
        kinds[j["kind"]] = kinds.get(j["kind"], 0) + 1
    kind = max(kinds, key=kinds.get) if kinds else None
    if kind:
        only = ctx.call("GET", f"/api/v2/admin/jobs?kind={kind}&limit=500", admin)["items"]
        ctx.check(f"kind={kind} → only those", bool(only) and all(j["kind"] == kind for j in only))
        state = only[-1]["state"]
        same = ctx.call("GET", f"/api/v2/admin/jobs?kind={kind}&state={state}&limit=500", admin)["items"]
        ctx.check(f"…and state={state} → only those", bool(same) and all(j["kind"] == kind and j["state"] == state for j in same))
    # Jobs are made all the time; new ones come first, so a walk made after the long page holds
    # it whole after whatever arrived meanwhile.
    full = [j["id"] for j in ctx.call("GET", "/api/v2/admin/jobs?limit=500", admin)["items"]]
    if len(full) < 3:
        ctx.skip("GET /admin/jobs pages without loss", f"{len(full)} jobs: too few to page")
        return
    small, cursor, pages = [], None, 0
    while len(small) < len(full) + 200:
        page = ctx.call("GET", "/api/v2/admin/jobs?limit=2" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else ""), admin)
        small += [j["id"] for j in page["items"]]
        pages += 1
        cursor = page.get("next")
        if not cursor:
            break
    start = small.index(full[0]) if full[0] in small else -1
    ctx.check("GET /admin/jobs pages without loss or repeats while new jobs arrive",
              start >= 0 and small[start:start + len(full)] == full and len(set(small)) == len(small) and pages > 1,
              f"{len(small)} in {pages} pages against {len(full)}")


def folders(ctx: Ctx) -> None:
    got = ctx.call("GET", "/api/v2/library/folders", ctx.admin_token)
    ok = isinstance(got.get("scanning"), bool) and isinstance(got.get("files_scanned"), int) and isinstance(got.get("folders"), list)
    for f in got.get("folders", []):
        ok = ok and f.get("media") in ("music", "books") and f.get("visibility") in ("family", "private") and isinstance(f.get("files_missing"), int)
    ctx.check(f"GET /library/folders → the folders and their last scan ({len(got.get('folders', []))} configured)", ok)
    if got.get("folders"):
        scanned = [f for f in got["folders"] if f.get("last_scan")]
        ctx.check("…each scanned folder says when and what it found",
                  all(isinstance(f["last_scan"].get("files_seen"), int) and f["last_scan"].get("started_at") for f in scanned))
    ctx.check("POST /library/folders/scan → 202", ctx.status_of("POST", "/api/v2/library/folders/scan", ctx.admin_token) == 202)


def second_household(ctx: Ctx) -> None:
    """A household the administrator is not in. On a one-household server only SQL makes one; on
    one with several, a new account has its own."""
    admin = ctx.admin_token
    sfx = ctx.sfx
    email = f"v2admin-locked-{sfx}@example.com"
    made = ctx.call("POST", "/api/v2/admin/users", admin, {"email": email, "password": PW, "display_name": f"Locked {sfx}"}, expect=(201,))
    uid = made["id"]
    ctx.track_user(uid, email)
    fid = made["family"]["id"]
    if ctx.feature("one_family"):
        fid = ctx.sql(f"INSERT INTO families (name, created_by) VALUES ('Admin test {sfx}', '{uid}') RETURNING id").splitlines()[0]
        ctx.sql(f"UPDATE family_members SET family_id = '{fid}', role = 'family_admin' WHERE user_id = '{uid}'")
        ctx.on_cleanup("the test household", lambda: ctx.sql(f"DELETE FROM families WHERE id = '{fid}' AND NOT EXISTS "
                                                             f"(SELECT 1 FROM family_members WHERE family_id = '{fid}')"))
    # Whatever happens below, nobody stays locked out.
    ctx.on_cleanup("unlock the test household",
                   lambda: ctx.call("PATCH", f"/api/v2/admin/families/{fid}", admin, {"locked": False}, expect=(200, 404)))
    token = _signs_in(ctx, email, PW)
    locked = ctx.call("PATCH", f"/api/v2/admin/families/{fid}", admin, {"locked": True})
    ctx.check("PATCH /admin/families/{id} {locked: true} → locked, every member blocked",
              locked["locked"] is True and all(m["blocked"] for m in locked["members"]))
    ctx.check("…its people are signed out and cannot sign in",
              token is not None and ctx.status_of("GET", "/api/v2/me", token) == 401 and _signs_in(ctx, email, PW) is None)
    ctx.check("…state=locked lists it, state=active does not",
              fid in {f["id"] for f in ctx.call("GET", "/api/v2/admin/families?state=locked&limit=500", admin)["items"]}
              and fid not in {f["id"] for f in ctx.call("GET", "/api/v2/admin/families?state=active&limit=500", admin)["items"]})
    open_again = ctx.call("PATCH", f"/api/v2/admin/families/{fid}", admin, {"locked": False})
    ctx.check("{locked: false} → open again, and its people sign in",
              open_again["locked"] is False and _signs_in(ctx, email, PW) is not None)

    every = [f["id"] for f in ctx.call("GET", "/api/v2/admin/families?limit=500", admin)["items"]]
    walked, cursor, pages = [], None, 0
    while True:
        page = ctx.call("GET", "/api/v2/admin/families?limit=1" + (f"&cursor={urllib.parse.quote(cursor)}" if cursor else ""), admin)
        walked += [f["id"] for f in page["items"]]
        pages += 1
        cursor = page.get("next")
        if not cursor or pages > 500:
            break
    ctx.check("GET /admin/families pages without loss or repeats", walked == every and len(every) >= 2 and pages >= 2,
              f"{len(walked)} in {pages} pages against {len(every)}")

    ctx.check("a household with people is not deleted → 409 family_not_empty",
              _problem(ctx.request("DELETE", f"/api/v2/admin/families/{fid}", admin), 409, "family_not_empty"))
    ctx.call("DELETE", f"/api/v2/admin/users/{uid}", admin, expect=(204,))
    ctx.check("deleting its last account takes the empty household with it",
              _problem(ctx.request("GET", f"/api/v2/admin/families/{fid}", admin), 404, "not_found"))
    if ctx.feature("one_family"):
        empty = ctx.sql(f"INSERT INTO families (name, created_by) VALUES ('Empty {sfx}', NULL) RETURNING id").splitlines()[0]
        ctx.on_cleanup("the empty household", lambda: ctx.sql(f"DELETE FROM families WHERE id = '{empty}'"))
        ctx.check("an empty household → DELETE 204, then 404",
                  ctx.status_of("DELETE", f"/api/v2/admin/families/{empty}", admin) == 204
                  and _problem(ctx.request("GET", f"/api/v2/admin/families/{empty}", admin), 404, "not_found"))


def disc_backfill(ctx: Ctx) -> None:
    _uid, _e, owner = ctx.make_user("discs", PW)
    track = ctx.upload_track(owner, f"Disc Two {ctx.sfx}", content=_tagged_wav(2, 3), content_type="audio/wav")
    tid = track["id"]
    read_at_upload = ctx.sql(f"SELECT disc_number FROM music_tracks_all WHERE id = '{tid}'")
    ctx.sql(f"UPDATE music_tracks_all SET disc_number = NULL, disc_total = NULL, disc_checked = false WHERE id = '{tid}'")
    done = ctx.wait_until(lambda: ctx.sql(f"SELECT disc_checked FROM music_tracks_all WHERE id = '{tid}'") == "t", timeout=120, every=2)
    ctx.check("a track marked as never read for its disc number is read by the worker on its own", done)
    after = ctx.sql(f"SELECT disc_number FROM music_tracks_all WHERE id = '{tid}'")
    ctx.check("…and gets the disc number its file carries", after == read_at_upload,
              f"after the job {after!r}, at upload {read_at_upload!r}")
    if read_at_upload != "2":
        ctx.skip("the file's own disc number", f"the upload read {read_at_upload!r} from the test file, not 2")
    runs = ctx.call("GET", "/api/v2/admin/jobs?kind=disc_backfill&limit=50", ctx.admin_token)["items"]
    reported = [j for j in runs if j["state"] == "completed" and isinstance((j.get("result") or {}).get("checked"), int)]
    ctx.check("GET /admin/jobs?kind=disc_backfill shows the pass with {checked, left}",
              bool(reported) and any(j["result"]["checked"] >= 1 for j in reported) and all(j["result"]["left"] >= 0 for j in reported))
