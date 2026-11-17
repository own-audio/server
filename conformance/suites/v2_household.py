# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 household remainder (completion plan phase 7): the household's photo and members' pictures, blocking, leaving, accounts made by an admin and claimed with a code, the join preview, regenerated codes, the household's settings, content reports.

What the v1 `join` and `families` suites prove, on v2: a join or claim code previewed without
signing in, an account claimed then signed in to, a code used twice refused (`410 invite_used`),
an expired one `410 invite_expired` (with `--compose-dir`), a regenerated code replacing the old;
a blocked member refused at sign-in and in `/sync/changes`, and let in again; leaving the
household (on a server with several), the last admin kept (`409 last_admin`), nobody leaving the
one household of a single-household server (`409 one_household`); settings read by a member and
changed by an admin only; reports made by a member and seen and closed by admins only. Also: the
change feed carries the household (`family`: its photo, a member's own rights) and its people
(`member`: pictures, `blocked`, someone gone); `Idempotency-Key` on the creates; paging of the
reports; who may (self, a household admin, another member, another household); problem shapes.
"""
from __future__ import annotations

import json
import urllib.parse

from core import Ctx
from suites.v2_books import Feed, _last, _png, _problem
from suites.v2_sync import _upload

NAME = "v2_household"
PW = "V2HouseholdTest123!"


class House:
    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx
        self.n = 0

    def token(self, email: str, password: str):
        data = urllib.parse.urlencode({"grant_type": "password", "username": email, "password": password,
                                       "device_name": "Conformance"}).encode()
        return self.ctx.request("POST", "/api/v2/auth/token", body=data, headers={"Content-Type": "application/x-www-form-urlencoded"})

    def signs_in(self, email: str, password: str) -> bool:
        return self.token(email, password)[0] == 200

    def member_of(self, admin: str, email: str, token: str) -> None:
        """Brings an account into the admin's household on a server with several."""
        if self.ctx.feature("one_family"):
            return
        inv = self.ctx.call("POST", "/api/v2/family/invites", admin, {"kind": "email", "email": email, "role": "member"}, expect=(201,))
        self.ctx.call("POST", "/api/v2/family/join", token, {"code": inv["code"]})

    def picture_upload(self, token: str, shade: int = 0) -> str:
        content = _png(shade)
        made = self.ctx.call("POST", "/api/v2/uploads", token, {"kind": "track_cover", "filename": "face.png",
                                                               "content_type": "image/png", "size_bytes": len(content)}, expect=(201,))
        self.ctx.put_bytes(made["url"], content, made["headers"]["Content-Type"])
        return made["upload_id"]

    def raw_picture(self, method: str, path: str, token: str, shade: int = 0):
        return self.ctx.request(method, path, token, _png(shade), headers={"Content-Type": "image/png"})

    def provision(self, admin: str, label: str, **extra) -> dict:
        self.n += 1
        body = {"display_name": f"Made {label} {self.ctx.sfx}", "login_email": f"made-{label}-{self.n}-{self.ctx.sfx}@family.example", **extra}
        made = self.ctx.call("POST", "/api/v2/family/members", admin, body, expect=(201,))
        self.ctx.track_user(made["member"]["user_id"], body["login_email"])
        return made

    def members(self, token: str) -> dict:
        return {m["user_id"]: m for m in self.ctx.call("GET", "/api/v2/family/members", token)}


def run(ctx: Ctx) -> None:
    h = House(ctx)
    one_household = ctx.feature("one_family")
    admin_id, admin_email, admin = ctx.make_family_admin("admin", PW)
    member_id, member_email, member = ctx.make_user("member", PW)
    h.member_of(admin, member_email, member)
    outsider = None
    if not one_household:
        _xid, _xe, outsider = ctx.make_family_admin("outsider", PW)

    pictures(ctx, h, admin, admin_id, member, member_id, outsider)
    rights_in_feed(ctx, h, admin, member, member_id)
    blocking(ctx, h, admin, admin_id, member)
    claiming(ctx, h, admin, member, outsider)
    leaving(ctx, h, admin, admin_id, member, member_id, outsider)
    settings(ctx, admin, member)
    reports(ctx, h, admin, admin_id, member, member_id, outsider)
    snapshot(ctx, member, member_id)
    naming(ctx, admin, member)
    joining(ctx, h, admin)


def naming(ctx: Ctx, admin: str, member: str) -> None:
    old = ctx.call("GET", "/api/v2/family", admin)["name"]
    renamed = ctx.call("PATCH", "/api/v2/family", admin, {"name": f"Renamed {ctx.sfx}"})
    ctx.check("PATCH /family renames the household → 200 Family, as its members see it",
              renamed["name"] == f"Renamed {ctx.sfx}" and ctx.call("GET", "/api/v2/family", member)["name"] == f"Renamed {ctx.sfx}")
    ctx.check("…a member may not → 403", _problem(ctx.request("PATCH", "/api/v2/family", member, {"name": "Mine"}), 403))
    ctx.call("PATCH", "/api/v2/family", admin, {"name": old})


def joining(ctx: Ctx, h: House, admin: str) -> None:
    home = ctx.call("GET", "/api/v2/family", admin)["id"]
    uid, email, tok = ctx.make_user("joiner", PW)
    inv = ctx.call("POST", "/api/v2/family/invites", admin, {"kind": "email", "email": email, "role": "member"}, expect=(201,))
    st, _h, raw = ctx.request("POST", "/api/v2/family/join", tok, {"code": inv["code"]})
    joined = json.loads(raw) if st == 200 else {}
    ctx.check("POST /family/join → 200, the household joined, with them in it",
              joined.get("id") == home and uid in h.members(admin), f"{st} {raw[:200]!r}")
    ctx.check("…the code works once", ctx.request("POST", "/api/v2/family/join", tok, {"code": inv["code"]})[0] in (400, 410))


def pictures(ctx: Ctx, h: House, admin: str, admin_id: str, member: str, member_id: str, outsider: str | None) -> None:
    check = ctx.check
    feed = Feed(ctx, member, "family,member")
    fam = ctx.call("GET", "/api/v2/family", admin)

    st, _h, raw = h.raw_picture("PUT", "/api/v2/family/image", admin)
    put = json.loads(raw) if st == 200 else {}
    check("PUT /family/image (the image as the body) → 200, the household with a link", st == 200 and put.get("id") == fam["id"]
          and "/api/v2/images/" in (put.get("image") or ""), f"{st} {raw[:200]!r}")
    if put.get("image"):
        got = ctx.request("GET", put["image"])
        check("…and the link serves the picture", got[0] == 200 and got[1].get("content-type", "").startswith("image/"))
    row = _last(feed.read(), "family", fam["id"])
    check("…and the household's new photo reaches a member's /sync/changes as `family`",
          row is not None and row["op"] == "upsert" and row["record"]["image"] and row["record"]["me"]["user_id"] == member_id)

    put2 = ctx.call("PUT", "/api/v2/family/image", admin, {"upload_id": h.picture_upload(admin, 40)})
    check("…or `{upload_id}` of an upload: a new picture, a new link", put2.get("image") and put2["image"] != put.get("image"))
    check("a member may not change it → 403",
          _problem(h.raw_picture("PUT", "/api/v2/family/image", member), 403)
          and _problem(ctx.request("DELETE", "/api/v2/family/image", member), 403))
    check("neither JSON nor an image → 415 unsupported_media_type",
          _problem(ctx.request("PUT", "/api/v2/family/image", admin, b"hello", headers={"Content-Type": "text/plain"}), 415, "unsupported_media_type"))
    check("an upload that is not the household's → 404",
          _problem(ctx.request("PUT", "/api/v2/family/image", admin, {"upload_id": "u/00000000-0000-0000-0000-000000000000/x.png"}), 404)
          or _problem(ctx.request("PUT", "/api/v2/family/image", admin, {"upload_id": "u/00000000-0000-0000-0000-000000000000/x.png"}), 400, "invalid_upload"))
    check("DELETE /family/image → 204, and the household has none",
          ctx.status_of("DELETE", "/api/v2/family/image", admin) == 204 and ctx.call("GET", "/api/v2/family", member)["image"] is None)
    row = _last(feed.read(), "family", fam["id"])
    check("…in the feed too", row is not None and row["record"]["image"] is None)

    # Members' pictures: their own, or an admin's doing.
    st, _h, raw = h.raw_picture("PUT", f"/api/v2/family/members/{member_id}/image", member, 10)
    mine = json.loads(raw) if st == 200 else {}
    check("PUT /family/members/{me}/image → 200, the member with a picture",
          st == 200 and mine.get("user_id") == member_id and "/api/v2/images/" in (mine.get("image") or ""), f"{st} {raw[:200]!r}")
    by_admin = ctx.call("PUT", f"/api/v2/family/members/{member_id}/image", admin, {"upload_id": h.picture_upload(admin, 80)})
    check("…a household admin sets someone else's", by_admin.get("image") and by_admin["image"] != mine.get("image"))
    check("…and it is the account's own picture (GET /me)", "/api/v2/images/" in (ctx.call("GET", "/api/v2/me", member).get("image") or ""))
    rows = Feed(ctx, admin, "member")
    ctx.call("PUT", f"/api/v2/family/members/{member_id}/image", member, {"upload_id": h.picture_upload(member, 120)})
    row = _last(rows.read(), "member", member_id)
    check("…and a new picture reaches the household's /sync/changes as `member`", row is not None and row["record"]["image"])
    check("a member may not set another's → 403",
          _problem(h.raw_picture("PUT", f"/api/v2/family/members/{admin_id}/image", member), 403)
          and _problem(ctx.request("DELETE", f"/api/v2/family/members/{admin_id}/image", member), 403))
    check("someone not in the household → 404",
          _problem(h.raw_picture("PUT", "/api/v2/family/members/00000000-0000-0000-0000-000000000000/image", admin), 404))
    check("DELETE /family/members/{me}/image → 204, and the picture is gone",
          ctx.status_of("DELETE", f"/api/v2/family/members/{member_id}/image", member) == 204
          and h.members(admin)[member_id]["image"] is None)
    row = _last(rows.read(), "member", member_id)
    check("…in the feed too", row is not None and row["record"]["image"] is None)
    if outsider:
        check("another household: a member's picture → 404",
              _problem(h.raw_picture("PUT", f"/api/v2/family/members/{member_id}/image", outsider), 404))
    else:
        ctx.skip("another household's pictures", "one household on this server")


def rights_in_feed(ctx: Ctx, h: House, admin: str, member: str, member_id: str) -> None:
    check = ctx.check
    feed = Feed(ctx, member, "family")
    other = Feed(ctx, admin, "family")
    fam_id = ctx.call("GET", "/api/v2/family", member)["id"]
    ctx.call("PATCH", f"/api/v2/family/members/{member_id}", admin, {"can_upload": False}, expect=(204,))
    row = _last(feed.read(), "family", fam_id)
    check("a member's own rights reach their /sync/changes as `family` (`me.can_upload`)",
          row is not None and row["record"]["me"]["can_upload"] is False)
    check("…and nobody else's feed", _last(other.read(), "family", fam_id) is None)
    ctx.call("PATCH", f"/api/v2/family/members/{member_id}", admin, {"can_upload": True}, expect=(204,))
    row = _last(feed.read(), "family", fam_id)
    check("…given back", row is not None and row["record"]["me"]["can_upload"] is True)


def blocking(ctx: Ctx, h: House, admin: str, admin_id: str, member: str) -> None:
    check = ctx.check
    bid, bemail, _v1 = ctx.make_user("blocked", PW)
    btok = ctx.call("POST", "/api/v1/auth/login", body={"email": bemail, "password": PW})["token"]
    h.member_of(admin, bemail, btok)
    st, _h, raw = h.token(bemail, PW)
    signed = json.loads(raw) if st == 200 else {}
    check("the member signs in with v2 first", st == 200 and signed.get("access_token"))
    v2tok = signed.get("access_token")
    feed = Feed(ctx, admin, "member")
    check("…and reads their change feed", ctx.request("GET", "/api/v2/sync/changes", v2tok)[0] == 200)

    check("PATCH /family/members/{id} {blocked: true} → 204",
          ctx.status_of("PATCH", f"/api/v2/family/members/{bid}", admin, {"blocked": True}) == 204)
    check("…their signed-in device is cut off: /sync/changes → 401",
          _problem(ctx.request("GET", "/api/v2/sync/changes", v2tok), 401)
          and _problem(ctx.request("GET", "/api/v2/sync/changes", btok), 401))
    refreshed = urllib.parse.urlencode({"grant_type": "refresh_token", "refresh_token": signed.get("refresh_token", "")}).encode()
    check("…their refresh token too", ctx.request("POST", "/api/v2/auth/token", body=refreshed,
                                                 headers={"Content-Type": "application/x-www-form-urlencoded"})[0] == 400)
    check("…and signing in is refused (400 invalid_grant)", _problem(h.token(bemail, PW), 400, "invalid_grant"))
    check("…the household sees them inactive", h.members(admin)[bid]["active"] is False)
    row = _last(feed.read(), "member", bid)
    check("…and their `member` record says `blocked` in /sync/changes", row is not None and row["record"].get("blocked") is True)

    check("blocking yourself → 400 cannot_block_self",
          _problem(ctx.request("PATCH", f"/api/v2/family/members/{admin_id}", admin, {"blocked": True}), 400, "cannot_block_self"))
    instance_admin = ctx.call("GET", "/api/v2/me", ctx.admin_token)["id"]
    if ctx.feature("one_family"):
        check("an administrator of the server → 403 cannot_block_instance_admin",
              _problem(ctx.request("PATCH", f"/api/v2/family/members/{instance_admin}", admin, {"blocked": True}), 403, "cannot_block_instance_admin"))
    else:
        check("the server's administrator, in another household → 404",
              _problem(ctx.request("PATCH", f"/api/v2/family/members/{instance_admin}", admin, {"blocked": True}), 404))
    check("a member may not block → 403", _problem(ctx.request("PATCH", f"/api/v2/family/members/{bid}", member, {"blocked": False}), 403))
    check("someone not in the household → 404",
          _problem(ctx.request("PATCH", "/api/v2/family/members/00000000-0000-0000-0000-000000000000", admin, {"blocked": True}), 404))

    check("{blocked: false} → 204, and they sign in again",
          ctx.status_of("PATCH", f"/api/v2/family/members/{bid}", admin, {"blocked": False}) == 204 and h.signs_in(bemail, PW))
    row = _last(feed.read(), "member", bid)
    check("…their record says so", row is not None and row["record"].get("blocked") is False)


def claiming(ctx: Ctx, h: House, admin: str, member: str, outsider: str | None) -> None:
    check = ctx.check
    sfx = ctx.sfx
    feed = Feed(ctx, admin, "member")
    login = f"made-kid-{sfx}@family.example"
    body = {"display_name": f"Kid {sfx}", "login_email": login, "label": "Kiddo", "age_bracket": "child"}
    key = {"Idempotency-Key": f"provision-{sfx}"}
    st, _h, raw = ctx.request("POST", "/api/v2/family/members", admin, body, headers=key)
    made = json.loads(raw) if st == 201 else {}
    kid_id = made.get("member", {}).get("user_id")
    if kid_id:
        ctx.track_user(kid_id, login)
    check("POST /family/members → 201, a pending member and a claim invitation",
          st == 201 and made["member"]["pending"] and made["member"]["label"] == "Kiddo" and made["invite"]["kind"] == "claim"
          and made["invite"]["code"] and made["invite"]["member_user_id"] == kid_id, f"{st} {raw[:300]!r}")
    check("…a child, who may not upload or generate",
          made.get("member", {}).get("age_bracket") == "child" and made["member"]["can_upload"] is False and made["member"]["can_generate"] is False)
    again = ctx.request("POST", "/api/v2/family/members", admin, body, headers=key)
    check("the same Idempotency-Key answers the first result and makes nothing more",
          again[0] == 201 and json.loads(again[2])["member"]["user_id"] == kid_id
          and sum(1 for m in h.members(admin).values() if m["email"] == login) == 1)
    check("…and with another body → 422 idempotency_key_reused",
          _problem(ctx.request("POST", "/api/v2/family/members", admin, {**body, "display_name": "Other"}, headers=key), 422, "idempotency_key_reused"))
    row = _last(feed.read(), "member", kid_id or "")
    check("…and the new member reaches /sync/changes", row is not None and row["op"] == "upsert")
    check("a blank name → 400 invalid_display_name",
          _problem(ctx.request("POST", "/api/v2/family/members", admin, {"display_name": " ", "login_email": f"x-{sfx}@f.example"}), 400, "invalid_display_name"))
    check("a login that is not shaped like an address → 400 invalid_email",
          _problem(ctx.request("POST", "/api/v2/family/members", admin, {"display_name": "X", "login_email": "kid"}), 400, "invalid_email"))
    check("a child given uploads → 400 invalid_body",
          _problem(ctx.request("POST", "/api/v2/family/members", admin, {"display_name": "X", "login_email": f"y-{sfx}@f.example",
                                                                         "age_bracket": "child", "can_upload": True}), 400, "invalid_body"))
    check("a login already taken → 409 email_taken",
          _problem(ctx.request("POST", "/api/v2/family/members", admin, {"display_name": "X", "login_email": login}), 409, "email_taken"))
    check("a member may not make one → 403",
          _problem(ctx.request("POST", "/api/v2/family/members", member, {"display_name": "X", "login_email": f"z-{sfx}@f.example"}), 403))
    invites = ctx.call("GET", "/api/v2/family/invites", admin)
    check("the claim invitation is listed with whom it lets in",
          any(i["kind"] == "claim" and i.get("member_user_id") == kid_id for i in invites))

    code = made.get("invite", {}).get("code", "")
    st, _h, raw = ctx.request("POST", "/api/v2/family/join/preview", None, {"code": code})
    prev = json.loads(raw) if st == 200 else {}
    fam = ctx.call("GET", "/api/v2/family", admin)
    check("POST /family/join/preview without signing in → the household, the inviter, the account",
          st == 200 and prev["kind"] == "claim" and prev["family"]["name"] == fam["name"] and prev["role"] == "member"
          and prev["claim"]["login_email"] == login and prev["uses_left"] == 1 and prev.get("inviter_name"), f"{st} {raw[:300]!r}")
    check("an unknown code → 404", _problem(ctx.request("POST", "/api/v2/family/join/preview", None, {"code": "0" * 24}), 404))
    check("the account cannot sign in before it is claimed", _problem(h.token(login, PW), 400, "invalid_grant"))
    check("a weak password → 400 weak_password",
          _problem(ctx.request("POST", "/api/v2/auth/claim", None, {"code": code, "password": "short"}), 400, "weak_password"))
    st, _h, raw = ctx.request("POST", "/api/v2/auth/claim", None, {"code": code, "password": PW})
    me = json.loads(raw) if st == 201 else {}
    check("POST /auth/claim → 201, the account", st == 201 and me.get("id") == kid_id and me.get("email") == login, f"{st} {raw[:200]!r}")
    check("…then it signs in with POST /auth/token", h.signs_in(login, PW))
    check("…and is no longer pending", h.members(admin)[kid_id]["pending"] is False if kid_id else False)
    check("the same code again → 410 invite_used",
          _problem(ctx.request("POST", "/api/v2/auth/claim", None, {"code": code, "password": PW}), 410, "invite_used")
          and _problem(ctx.request("POST", "/api/v2/family/join/preview", None, {"code": code}), 410, "invite_used"))
    check("an unknown claim code → 404", _problem(ctx.request("POST", "/api/v2/auth/claim", None, {"code": "0" * 24, "password": PW}), 404))
    link = ctx.call("POST", "/api/v2/family/invites", admin, {"kind": "link", "role": "member"}, expect=(201,))
    st, _h, raw = ctx.request("POST", "/api/v2/family/join/preview", None, {"code": link["code"]})
    check("a link invitation previews as `invite`", st == 200 and json.loads(raw)["kind"] == "invite")
    check("…and is no claim code → 400 not_a_claim_code",
          _problem(ctx.request("POST", "/api/v2/auth/claim", None, {"code": link["code"], "password": PW}), 400, "not_a_claim_code"))
    ctx.call("DELETE", f"/api/v2/family/invites/{link['id']}", admin, expect=(204,))

    # A new code replaces the old one.
    second = h.provision(admin, "regen")
    old = second["invite"]["code"]
    renewed = ctx.call("POST", f"/api/v2/family/invites/{second['invite']['id']}/regenerate", admin)
    check("POST /family/invites/{id}/regenerate → a new code",
          renewed["id"] == second["invite"]["id"] and renewed["code"] != old and renewed["kind"] == "claim")
    check("…the old code is unknown now (404), the new one previews",
          _problem(ctx.request("POST", "/api/v2/family/join/preview", None, {"code": old}), 404)
          and ctx.request("POST", "/api/v2/family/join/preview", None, {"code": renewed["code"]})[0] == 200)
    check("regenerating a used invitation → 409 invite_used",
          _problem(ctx.request("POST", f"/api/v2/family/invites/{made.get('invite', {}).get('id')}/regenerate", admin), 409, "invite_used"))
    check("an unknown invitation → 404",
          _problem(ctx.request("POST", "/api/v2/family/invites/00000000-0000-0000-0000-000000000000/regenerate", admin), 404))
    check("a member may not → 403",
          _problem(ctx.request("POST", f"/api/v2/family/invites/{second['invite']['id']}/regenerate", member), 403))
    if outsider:
        check("another household's admin → 404",
              _problem(ctx.request("POST", f"/api/v2/family/invites/{second['invite']['id']}/regenerate", outsider), 404))

    # An expired code, by moving its expiry into the past.
    if ctx.db_available:
        third = h.provision(admin, "expired")
        ctx.sql(f"UPDATE family_invites SET expires_at = now() - interval '1 day' WHERE id = '{third['invite']['id']}'")
        check("an expired code → 410 invite_expired, to preview and to claim",
              _problem(ctx.request("POST", "/api/v2/family/join/preview", None, {"code": third["invite"]["code"]}), 410, "invite_expired")
              and _problem(ctx.request("POST", "/api/v2/auth/claim", None, {"code": third["invite"]["code"], "password": PW}), 410, "invite_expired"))
    else:
        ctx.skip("an expired code", "needs --compose-dir to move an expiry into the past")

    # An account nobody claimed goes with its removal; its record stays nameless.
    pending = second["member"]["user_id"]
    check("DELETE /family/members/{pending} → 204, and the account is gone",
          ctx.status_of("DELETE", f"/api/v2/family/members/{pending}", admin) == 204 and pending not in h.members(admin))
    row = _last(feed.read(), "member", pending)
    check("…its `member` record in the feed has left and keeps no name",
          row is not None and (row["op"] == "delete" or (row["record"].get("left_at") and row["record"].get("display_name") is None)))


def leaving(ctx: Ctx, h: House, admin: str, admin_id: str, member: str, member_id: str, outsider: str | None) -> None:
    check = ctx.check
    check("someone else, by a member → 403", _problem(ctx.request("DELETE", f"/api/v2/family/members/{admin_id}", member), 403))
    check("someone not in the household → 404",
          _problem(ctx.request("DELETE", "/api/v2/family/members/00000000-0000-0000-0000-000000000000", admin), 404))
    if ctx.feature("one_family"):
        check("leaving the one household of this server → 409 one_household",
              _problem(ctx.request("DELETE", f"/api/v2/family/members/{member_id}", member), 409, "one_household"))
        check("…nor may an admin take a member out (block or delete instead) → 409 one_household",
              _problem(ctx.request("DELETE", f"/api/v2/family/members/{member_id}", admin), 409, "one_household"))
        # The last admin: the suite's admin goes back to member for a moment, if that leaves one.
        admins = [m for m in h.members(admin).values() if m["role"] == "admin"]
        instance_admin = ctx.call("GET", "/api/v2/me", ctx.admin_token)["id"]
        if len(admins) == 2 and {a["user_id"] for a in admins} == {admin_id, instance_admin}:
            ctx.call("PATCH", f"/api/v2/family/members/{admin_id}", ctx.admin_token, {"role": "member"}, expect=(204,))
            try:
                check("the last admin may not leave → 409 last_admin",
                      _problem(ctx.request("DELETE", f"/api/v2/family/members/{instance_admin}", ctx.admin_token), 409, "last_admin"))
            finally:
                ctx.call("PATCH", f"/api/v2/family/members/{admin_id}", ctx.admin_token, {"role": "admin"}, expect=(204,))
        else:
            ctx.skip("the last admin", f"the household has {len(admins)} admins besides this suite's")
        ctx.skip("leaving a household", "one household on this server")
        return

    # Several households: a fresh one, so leaving does not take this suite's member away.
    head_id, _he, head = ctx.make_family_admin("head", PW)
    stay_id, stay_email, stay = ctx.make_user("stay", PW)
    go_id, go_email, go = ctx.make_user("go", PW)
    h.member_of(head, stay_email, stay)
    h.member_of(head, go_email, go)
    home = ctx.call("GET", "/api/v2/family", head)["id"]
    feed = Feed(ctx, head, "member")
    check("the last admin may not leave while others are in it → 409 last_admin",
          _problem(ctx.request("DELETE", f"/api/v2/family/members/{head_id}", head), 409, "last_admin"))
    check("a member leaves: DELETE /family/members/{me} → 204", ctx.status_of("DELETE", f"/api/v2/family/members/{go_id}", go) == 204)
    check("…into a household of their own", ctx.call("GET", "/api/v2/family", go)["id"] != home and go_id not in h.members(head))
    row = _last(feed.read(), "member", go_id)
    check("…and the household's feed keeps their name with `left_at`",
          row is not None and row["record"].get("left_at") and row["record"].get("display_name"))
    check("an admin takes a member out → 204", ctx.status_of("DELETE", f"/api/v2/family/members/{stay_id}", head) == 204)
    check("alone, the admin may leave: nothing changes → 204", ctx.status_of("DELETE", f"/api/v2/family/members/{head_id}", head) == 204
          and ctx.call("GET", "/api/v2/family", head)["id"] == home)
    if outsider:
        check("another household's admin takes nobody out → 404",
              _problem(ctx.request("DELETE", f"/api/v2/family/members/{member_id}", outsider), 404))


def settings(ctx: Ctx, admin: str, member: str) -> None:
    check = ctx.check
    st, _h, raw = ctx.request("GET", "/api/v2/family/settings", member)
    before = json.loads(raw) if st == 200 else {}
    check("GET /family/settings: a member reads them",
          st == 200 and isinstance(before.get("audio_analysis", {}).get("enabled"), bool) and "source" in before.get("book_source", {})
          and "google_books_api_key" not in json.dumps(before), f"{st} {raw[:200]!r}")
    check("a member may not change them → 403",
          _problem(ctx.request("PATCH", "/api/v2/family/settings", member, {"audio_analysis": {"enabled": True}}), 403))
    was = before.get("audio_analysis", {}).get("enabled", False)
    try:
        on = ctx.call("PATCH", "/api/v2/family/settings", admin, {"audio_analysis": {"enabled": not was}})
        check("PATCH /family/settings (audio analysis) → 200, changed", on["audio_analysis"]["enabled"] is (not was)
              and on["book_source"]["source"] == before["book_source"]["source"])
    finally:
        ctx.call("PATCH", "/api/v2/family/settings", admin, {"audio_analysis": {"enabled": was}})
    check("an empty PATCH changes nothing", ctx.call("PATCH", "/api/v2/family/settings", admin, {})["book_source"] == before["book_source"])
    if before.get("book_source", {}).get("configurable"):
        source = before["book_source"]["source"]
        other = "apple" if source != "apple" else "openlibrary"
        try:
            got = ctx.call("PATCH", "/api/v2/family/settings", admin, {"book_source": {"source": other}})
            check("the book source, chosen", got["book_source"]["source"] == other
                  and ctx.call("GET", "/api/v2/family/settings", member)["book_source"]["source"] == other)
            check("a key that cannot be one → 400 invalid_key",
                  _problem(ctx.request("PATCH", "/api/v2/family/settings", admin, {"book_source": {"source": "google", "google_books_api_key": "not a key!"}}),
                           400, "invalid_key"))
            if not before["book_source"]["google_key_set"] and not before["book_source"]["server_google_key"]:
                check("Google without a key → 400 key_required, and nothing changed",
                      _problem(ctx.request("PATCH", "/api/v2/family/settings", admin, {"book_source": {"source": "google"}, "audio_analysis": {"enabled": not was}}),
                               400, "key_required")
                      and ctx.call("GET", "/api/v2/family/settings", admin)["audio_analysis"]["enabled"] is was)
        finally:
            ctx.call("PATCH", "/api/v2/family/settings", admin, {"book_source": {"source": source}})
    else:
        check("a book source on a server that offers no choice → 501 feature_unavailable",
              _problem(ctx.request("PATCH", "/api/v2/family/settings", admin, {"book_source": {"source": "apple"}}), 501, "feature_unavailable"))


def reports(ctx: Ctx, h: House, admin: str, admin_id: str, member: str, member_id: str, outsider: str | None) -> None:
    check = ctx.check
    sfx = ctx.sfx
    shared = ctx.call("POST", "/api/v2/tracks", admin, {"upload_id": _upload(ctx, admin, "track", "r.wav"), "filename": "r.wav",
                                                          "title": f"Reported {sfx}", "artist": f"Report Band {sfx}", "visibility": "family"},
                      expect=(201,))
    private = ctx.call("POST", "/api/v2/tracks", admin, {"upload_id": _upload(ctx, admin, "track", "p.wav"), "filename": "p.wav",
                                                           "title": f"Kept {sfx}", "artist": f"Report Band {sfx}", "visibility": "private"},
                       expect=(201,))
    show = ctx.call("POST", "/api/v2/podcasts", admin, {"title": f"Report Show {sfx}", "visibility": "family"}, expect=(200, 201))
    body = {"kind": "track", "item_id": shared["id"], "reason": "offensive", "note": "  not for the kids  "}
    key = {"Idempotency-Key": f"report-{sfx}"}
    st, _h, raw = ctx.request("POST", "/api/v2/family/reports", member, body, headers=key)
    made = json.loads(raw) if st == 201 else {}
    check("POST /family/reports by a member → 201, the report",
          st == 201 and made["kind"] == "track" and made["item_id"] == shared["id"] and made["reason"] == "offensive"
          and made["note"] == "not for the kids" and made["reporter_id"] == member_id and made["resolved"] is False, f"{st} {raw[:200]!r}")
    again = ctx.request("POST", "/api/v2/family/reports", member, body, headers=key)
    check("…the same Idempotency-Key answers the same report", again[0] == 201 and json.loads(again[2])["id"] == made.get("id"))
    pod = ctx.call("POST", "/api/v2/family/reports", member, {"kind": "podcast", "item_id": show["id"], "reason": "inaccurate"}, expect=(201,))
    check("…a podcast too", pod["kind"] == "podcast" and pod["note"] is None)
    check("something the reporter cannot see → 404",
          _problem(ctx.request("POST", "/api/v2/family/reports", member, {**body, "item_id": private["id"]}), 404))
    check("a kind that cannot be reported → 400 unsupported_kind",
          _problem(ctx.request("POST", "/api/v2/family/reports", member, {**body, "kind": "music"}), 400, "unsupported_kind"))
    check("a reason that is not one → 400 invalid_reason",
          _problem(ctx.request("POST", "/api/v2/family/reports", member, {**body, "reason": "boring"}), 400, "invalid_reason"))

    check("a member does not read the reports → 403", _problem(ctx.request("GET", "/api/v2/family/reports", member), 403))
    open_now = ctx.call("GET", "/api/v2/family/reports", admin)["items"]
    check("GET /family/reports: an admin reads the open ones, newest first",
          [r["id"] for r in open_now[:2]] == [pod["id"], made.get("id")])
    check("an unknown state → 400 invalid_state", _problem(ctx.request("GET", "/api/v2/family/reports?state=closed", admin), 400, "invalid_state"))

    rid = made.get("id")
    done = ctx.call("PATCH", f"/api/v2/family/reports/{rid}", admin, {"resolved": True, "note": "taken out of the shared library"})
    check("PATCH /family/reports/{id} {resolved: true, note} → 200, closed by the admin",
          done["resolved"] and done["resolved_by"] == admin_id and done["resolution_note"] == "taken out of the shared library" and done["resolved_at"])
    check("…gone from the open list, in the resolved one",
          all(r["id"] != rid for r in ctx.call("GET", "/api/v2/family/reports", admin)["items"])
          and any(r["id"] == rid for r in ctx.call("GET", "/api/v2/family/reports?state=resolved", admin)["items"]))
    twice = ctx.call("PATCH", f"/api/v2/family/reports/{rid}", admin, {"resolved": True})
    check("…closing it again keeps when it was closed, and the note", twice["resolved_at"] == done["resolved_at"]
          and twice["resolution_note"] == done["resolution_note"])
    reopened = ctx.call("PATCH", f"/api/v2/family/reports/{rid}", admin, {"resolved": False})
    check("{resolved: false} opens it again", reopened["resolved"] is False and reopened["resolved_at"] is None
          and any(r["id"] == rid for r in ctx.call("GET", "/api/v2/family/reports", admin)["items"]))
    check("a member may not close one → 403", _problem(ctx.request("PATCH", f"/api/v2/family/reports/{rid}", member, {"resolved": True}), 403))
    check("an unknown report → 404",
          _problem(ctx.request("PATCH", "/api/v2/family/reports/00000000-0000-0000-0000-000000000000", admin, {"resolved": True}), 404))
    if outsider:
        check("another household: not in its list, not to close, nothing to report → 404",
              all(r["id"] != rid for r in ctx.call("GET", "/api/v2/family/reports?state=all", outsider)["items"])
              and _problem(ctx.request("PATCH", f"/api/v2/family/reports/{rid}", outsider, {"resolved": True}), 404)
              and _problem(ctx.request("POST", "/api/v2/family/reports", outsider, body), 404))
    else:
        ctx.skip("another household's reports", "one household on this server")

    # Paging: a few more, one at a time against one page.
    for reason in ("other", "not_for_children", "inaccurate"):
        ctx.call("POST", "/api/v2/family/reports", member, {"kind": "track", "item_id": shared["id"], "reason": reason}, expect=(201,))

    def walk(limit: int) -> tuple[list, int]:
        items, cursor, pages = [], None, 0
        while True:
            q = {"state": "all", "limit": str(limit)}
            if cursor:
                q["cursor"] = cursor
            page = ctx.call("GET", "/api/v2/family/reports?" + urllib.parse.urlencode(q), admin)
            items += page["items"]
            cursor, pages = page.get("next"), pages + 1
            if not cursor or pages > 500:
                return items, pages

    full, _p = walk(500)
    small, pages = walk(1)
    check("GET /family/reports pages without loss or repeats",
          [r["id"] for r in small] == [r["id"] for r in full] and pages >= 5 and len({r["id"] for r in small}) == len(small))
    for r in full:
        if not r["resolved"] and r["item_id"] in (shared["id"], show["id"]):
            ctx.call("PATCH", f"/api/v2/family/reports/{r['id']}", admin, {"resolved": True})


def snapshot(ctx: Ctx, member: str, member_id: str) -> None:
    raw = ctx.call("GET", "/api/v2/sync/snapshot?kinds=family,member", member, raw=True)
    lines = [json.loads(line) for line in raw.decode().splitlines() if line.strip()]
    records = [line for line in lines if line.get("type") == "record"]
    fam = ctx.call("GET", "/api/v2/family", member)
    families = [r for r in records if r["kind"] == "family"]
    ctx.check("the snapshot holds the household once, as `GET /family` answers it, after its people",
              len(families) == 1 and families[0]["id"] == fam["id"] and families[0]["record"]["me"]["user_id"] == member_id
              and records[0]["kind"] == "member" and lines[-1].get("type") == "end")
    ctx.check("…and every `member` record says whether they are blocked",
              all(isinstance(r["record"].get("blocked"), bool) for r in records if r["kind"] == "member"))
    ids = ctx.call("GET", "/api/v2/sync/snapshot?kinds=family&fields=ids", member, raw=True)
    ctx.check("…the ids-only snapshot names it too",
              any(json.loads(line).get("kind") == "family" for line in ids.decode().splitlines() if line.strip()))
