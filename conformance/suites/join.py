# SPDX-License-Identifier: AGPL-3.0-or-later
"""QR/link/claim family-join flows: link invites, provision + claim, regenerate, unknown codes.

Ported from audio2/scripts/family_join_test.py (see docs/family-join-qr-plan.md
there). Builds a family via a shareable link invite, provisions a member with
no mailbox and claims their account by password, and checks the guardrails:
link invites can't grant admin, exhausted/expired codes report their status
without leaking the family, unknown codes are a plain 404, and regenerating a
code kills the old one.
"""
from __future__ import annotations

from core import Ctx

NAME = "join"
PW = "FamilyJoinTest123!"


def _track_user(ctx: Ctx, uid: str, what: str) -> None:
    """Accounts here are made through register/claim, not make_user, so cleanup is registered by hand."""
    ctx.on_cleanup(f"user {what}", lambda: ctx.delete_user(uid))


def run(ctx: Ctx) -> None:
    admin_tok = ctx.admin_token
    sfx = ctx.sfx

    # ── Link invites: email-free, multi-use, member-role only ────────────
    ctx.log("\n[link invite]")
    ctx.call("POST", "/api/v1/family/invites", admin_tok,
             {"kind": "link", "role": "family_admin"}, expect=(400,))
    ctx.ok("D3: a link invite cannot grant family_admin")

    link = ctx.call("POST", "/api/v1/family/invites", admin_tok,
                    {"kind": "link", "max_uses": 2, "label": f"fridge-{sfx}"})
    ctx.check("link invite has no email", link["email"] is None)
    ctx.check("link invite carries a join_url", link["join_url"] is not None and link["code"] in link["join_url"])

    preview = ctx.call("GET", f"/api/v1/join/{link['code']}")
    ctx.check("preview is valid and reveals the family",
              preview["status"] == "valid" and bool(preview["family_name"]))

    joiner_email = f"joiner-{sfx}@example.com"
    joined = ctx.call("POST", "/api/v1/auth/register", body={
        "email": joiner_email, "password": PW, "display_name": "Link Joiner",
        "invite_code": link["code"],
    })
    _track_user(ctx, joined["user"]["id"], joiner_email)
    ctx.ok("registering with a link code needs no email match")

    second_email = f"joiner2-{sfx}@example.com"
    joined2 = ctx.call("POST", "/api/v1/auth/register", body={
        "email": second_email, "password": PW, "display_name": "Second Joiner",
        "invite_code": link["code"],
    })
    _track_user(ctx, joined2["user"]["id"], second_email)
    ctx.ok("a second account can redeem the same multi-use link")

    exhausted = ctx.call("GET", f"/api/v1/join/{link['code']}")
    ctx.check("link reports exhausted once max_uses is spent", exhausted["status"] == "exhausted")
    ctx.check("exhausted preview reveals nothing about the family", exhausted["family_name"] is None)

    third_email = f"joiner3-{sfx}@example.com"
    ctx.call("POST", "/api/v1/auth/register", body={
        "email": third_email, "password": PW, "display_name": "Third Joiner",
        "invite_code": link["code"],
    }, expect=(400,))
    ctx.ok("registering against an exhausted link is rejected")

    # ── Provision + claim: admin creates the account, member sets the password ─
    ctx.log("\n[provision + claim]")
    login_email = f"lena-{sfx}@joinfamily.family"
    provisioned = ctx.call("POST", "/api/v1/family/members/provision", admin_tok,
                           {"display_name": "Lena", "login_email": login_email})
    lena_id = provisioned["member"]["user_id"]
    _track_user(ctx, lena_id, login_email)
    ctx.check("provisioned member starts pending", provisioned["member"]["pending"] is True)
    ctx.check("provisioning mints a claim invite", provisioned["invite"]["kind"] == "claim")
    claim_code = provisioned["invite"]["code"]

    claim_preview = ctx.call("GET", f"/api/v1/join/{claim_code}")
    ctx.check("claim preview names the account being claimed",
              claim_preview["claim"]["login_email"] == login_email)

    ctx.call("POST", f"/api/v1/join/{claim_code}/claim", body={"password": "short"}, expect=(400,))
    ctx.ok("claim rejects a too-short password")

    ctx.call("POST", "/api/v1/auth/login",
             body={"email": login_email, "password": "LenaPass123!"}, expect=(401,))
    ctx.ok("unclaimed account cannot log in yet")

    claimed = ctx.call("POST", f"/api/v1/join/{claim_code}/claim", body={"password": "LenaPass123!"})
    ctx.check("claiming returns a working session", "token" in claimed)

    ctx.call("POST", f"/api/v1/join/{claim_code}/claim", body={"password": "LenaPass123!"}, expect=(400,))
    ctx.ok("the same claim code cannot be redeemed twice")

    ctx.call("POST", "/api/v1/auth/login", body={"email": login_email, "password": "LenaPass123!"})
    ctx.ok("claimed account can now log in normally")

    members = ctx.call("GET", "/api/v1/family/members", admin_tok)
    lena = next(m for m in members if m["user_id"] == lena_id)
    ctx.check("member no longer shows pending after claim", lena["pending"] is False)

    # ── Deleting an unclaimed provisioned account is a clean hard delete ──
    # The ghost is deliberately not registered for cleanup: the test itself
    # deletes it through the family endpoint and asserts nothing is left.
    ctx.log("\n[unclaimed cleanup]")
    ghost = ctx.call("POST", "/api/v1/family/members/provision", admin_tok,
                     {"display_name": "Ghost", "login_email": f"ghost-{sfx}@joinfamily.family"})
    ghost_id = ghost["member"]["user_id"]
    ctx.call("DELETE", f"/api/v1/family/members/{ghost_id}", admin_tok)
    members = ctx.call("GET", "/api/v1/family/members", admin_tok)
    ctx.check("an unclaimed account is fully removed, not re-homed",
              not any(m["user_id"] == ghost_id for m in members))
    ghost_preview_code = ghost["invite"]["code"]
    ctx.call("GET", f"/api/v1/join/{ghost_preview_code}", expect=(404,))
    ctx.ok("its claim invite is gone too (cascade)")

    # ── Regenerate kills the old code outright ────────────────────────────
    ctx.log("\n[regenerate]")
    fresh = ctx.call("POST", "/api/v1/family/invites", admin_tok, {"kind": "link", "max_uses": 5})
    invites = ctx.call("GET", "/api/v1/family/invites", admin_tok)
    invite_id = next(i["id"] for i in invites if i["code"] == fresh["code"])
    regenerated = ctx.call("POST", f"/api/v1/family/invites/{invite_id}/regenerate", admin_tok)
    ctx.check("regeneration mints a different code", regenerated["code"] != fresh["code"])
    ctx.call("GET", f"/api/v1/join/{fresh['code']}", expect=(404,))
    ctx.ok("the old code is dead immediately")
    still_valid = ctx.call("GET", f"/api/v1/join/{regenerated['code']}")
    ctx.check("the new code works", still_valid["status"] == "valid")

    # ── Unknown codes never leak anything ─────────────────────────────────
    ctx.log("\n[unknown codes]")
    ctx.call("GET", "/api/v1/join/0000000000000000deadbeef", expect=(404,))
    ctx.ok("an unknown code is a plain 404")
