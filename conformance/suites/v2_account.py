# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 account and sign-in (completion plan phase 1): sign-up, the password and its reset, address confirmation, the security trail, two-factor, the profile and picture, the Subsonic key, deleting the account, token exchange and the code grant, discovery's sign-in fields.

Mailed links are tested by putting a link's hash into the database (`--compose-dir`), the way
the server stores them; without database access those checks are skipped. A server without
mail must say so: discovery's `auth.password_reset` and `auth.email_verification`, and `501
feature_unavailable` from the routes that would mail. The per-address sign-in bucket on the new
routes is proved by the server's handler tests (CI raises the limit for the whole suite).

Sign in with Apple in a browser (`auth.apple_web`) is walked as far as a server goes without
Apple: the start, Apple's answer for an unknown, cancelled, unverifiable or expired sign-in, and —
with database access — the code grant, with Apple's verified answer put in the database the way
the server keeps it. The app return address used is `audio.own.musicplayer:/apple` (the default
allowlist), or `CONFORMANCE_APPLE_REDIRECT_URI`.
"""
from __future__ import annotations

import base64
import hashlib
import hmac
import json
import os
import struct
import time
import urllib.error
import urllib.parse
import urllib.request
import uuid

from core import MAIL_DOMAIN, Ctx

NAME = "v2_account"
PW = "V2AccountTest123!"
NEW_PW = "a new password of some words"
PROBLEM = "application/problem+json"
EXCHANGE = "urn:ietf:params:oauth:grant-type:token-exchange"
REFRESH_TYPE = "urn:ietf:params:oauth:token-type:refresh_token"
ACCESS_TYPE = "urn:ietf:params:oauth:token-type:access_token"
APPLE_CODE = "urn:own-audio:params:oauth:grant-type:apple-code"
APPLE_REDIRECT = os.environ.get("CONFORMANCE_APPLE_REDIRECT_URI", "audio.own.musicplayer:/apple")


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


def _json(answer):
    try:
        return json.loads(answer[2]) if answer[2] else {}
    except ValueError:
        return {}


def _totp(secret_b32: str, at: float) -> str:
    key = base64.b32decode(secret_b32 + "=" * (-len(secret_b32) % 8))
    digest = hmac.new(key, struct.pack(">Q", int(at // 30)), "sha1").digest()
    off = digest[19] & 0x0F
    return f"{(int.from_bytes(digest[off:off + 4], 'big') & 0x7FFFFFFF) % 1_000_000:06d}"


def _png(side: int = 48) -> bytes:
    import zlib
    raw = b"".join(b"\x00" + b"".join(bytes((x % 256, y % 256, 99)) for x in range(side)) for y in range(side))

    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    return b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")


class Account:
    """Calls for one test: the token endpoint as OAuth clients send it, and the rest."""

    def __init__(self, ctx: Ctx) -> None:
        self.ctx = ctx

    def token(self, fields: dict):
        data = urllib.parse.urlencode(fields).encode()
        return self.ctx.request("POST", "/api/v2/auth/token", body=data,
                                headers={"Content-Type": "application/x-www-form-urlencoded"})

    def sign_in(self, email: str, password: str, device: str = "Conformance") -> dict:
        st, _h, body = self.token({"grant_type": "password", "username": email, "password": password, "device_name": device})
        assert st == 200, f"sign-in as {email}: {st} {body[:200]!r}"
        return json.loads(body)

    def oauth_error(self, answer, status: int, error: str) -> bool:
        return _problem(answer, status, error) and _json(answer).get("error") == error


def run(ctx: Ctx) -> None:
    a = Account(ctx)
    _, _, disco_raw = ctx.request("GET", "/.well-known/own-audio")
    disco = json.loads(disco_raw)
    discovery(ctx, disco)
    register(ctx, a, disco)
    password(ctx, a, disco)
    email_verification(ctx, a, disco)
    security_events(ctx, a)
    two_factor(ctx, a)
    profile(ctx, a)
    picture(ctx, a)
    subsonic(ctx, a)
    deletion(ctx, a)
    exchange(ctx, a)
    code_grant(ctx, a, disco)
    apple_web(ctx, a, disco)
    personal_tokens(ctx, a)
    everywhere(ctx, a)


def everywhere(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-out", PW)
    phone = a.sign_in(email, PW, "Phone")
    laptop = a.sign_in(email, PW, "Laptop")
    ctx.check("DELETE /me/sessions → 204", ctx.status_of("DELETE", "/api/v2/me/sessions", phone["access_token"]) == 204)
    ctx.check("…every device is signed out, this one too, and no refresh token renews",
              all(_problem(ctx.request("GET", "/api/v2/me", t["access_token"]), 401) for t in (phone, laptop))
              and all(a.oauth_error(a.token({"grant_type": "refresh_token", "refresh_token": t["refresh_token"]}), 400, "invalid_grant")
                      for t in (phone, laptop)))
    ctx.check("…and the password signs in again", a.token({"grant_type": "password", "username": email, "password": PW})[0] == 200)


def discovery(ctx: Ctx, disco: dict) -> None:
    auth = disco.get("auth") or {}
    grants = auth.get("grants") or []
    ctx.check("discovery lists the token grants, token exchange among them",
              {"refresh_token", EXCHANGE, "urn:ietf:params:oauth:grant-type:device_code"} <= set(grants)
              and ("password" in grants) == bool(auth.get("password")), str(grants))
    ctx.check("discovery says whether this server mails reset and confirmation links",
              isinstance(auth.get("password_reset"), bool) and isinstance(auth.get("email_verification"), bool))
    providers = ctx.call("GET", "/api/v1/auth/providers")
    clients = auth.get("clients") or {}
    agree = all(bool(clients.get(p)) == bool((providers.get(p) or {}).get("enabled")) for p in ("google", "microsoft", "apple"))
    agree = agree and (clients.get("google") or {}).get("web") == ((providers.get("google") or {}).get("web_client_id"))
    ctx.check("discovery's provider client ids agree with v1's /auth/providers", agree, f"{clients} / {providers}")
    ctx.check("the code grant is offered only with a desktop client to exchange codes for",
              ("authorization_code" in grants) == any((clients.get(p) or {}).get("desktop") for p in ("google", "microsoft")))
    ctx.check("discovery: setup is complete on a server with an admin", (disco.get("setup") or {}).get("complete") is True)
    server = ctx.call("GET", "/api/v1/server")
    ctx.check("discovery's demo agrees with v1's /server", disco.get("demo") == server.get("demo"),
              f"{disco.get('demo')} / {server.get('demo')}")


def register(ctx: Ctx, a: Account, disco: dict) -> None:
    sfx = ctx.sfx
    email = f"v2acc-reg-{sfx}@{MAIL_DOMAIN}"
    body = {"email": email, "password": PW, "display_name": "Signed Up"}
    reg = lambda b: ctx.request("POST", "/api/v2/auth/register", body=b)  # noqa: E731
    ctx.check("sign-up: a short password is 400 weak_password on its field",
              _problem(reg({**body, "password": "short"}), 400, "weak_password"))
    ctx.check("sign-up: not an address is 400 invalid_email", _problem(reg({**body, "email": "nobody"}), 400, "invalid_email"))
    ctx.check("sign-up: a blank name is 400 invalid_display_name",
              _problem(reg({**body, "display_name": " "}), 400, "invalid_display_name"))
    ctx.check("sign-up: an unknown invitation is 400 invalid_invite",
              _problem(reg({**body, "invite_code": "not-a-code"}), 400, "invalid_invite"))
    closed = not (disco.get("modules") or {}).get("registration_open")
    admin = None
    if closed:
        ctx.check("sign-up without an invitation on a closed server: 403 registration_closed",
                  _problem(reg(body), 403, "registration_closed"))
        _id, _e, admin = ctx.make_family_admin("v2acc-inviter", PW)
        invite = ctx.call("POST", "/api/v2/family/invites", admin, {"kind": "link", "role": "member", "max_uses": 1}, expect=(201,))
        body = {**body, "invite_code": invite["code"]}
    st, _h, raw = reg(body)
    me = json.loads(raw) if st == 201 else {}
    if me.get("id"):
        ctx.track_user(me["id"], email)
    ctx.check("sign-up: 201 with the account (an invitation where registration is closed)",
              st == 201 and me.get("email") == email and me.get("display_name") == "Signed Up"
              and (me.get("family") or {}).get("role") == "member" and me.get("two_factor", {}).get("enabled") is False,
              f"{st} {raw[:200]!r}")
    ctx.check("…and the new account signs in with the token endpoint",
              a.token({"grant_type": "password", "username": email, "password": PW})[0] == 200)
    again = {**body, "email": email.upper()}
    if closed:
        ctx.check("a used invitation does not let a second account in",
                  _problem(reg({**body, "email": f"v2acc-reg2-{sfx}@{MAIL_DOMAIN}"}), 400, "invalid_invite"))
        fresh = ctx.call("POST", "/api/v2/family/invites", admin, {"kind": "link", "role": "member", "max_uses": 1}, expect=(201,))
        again["invite_code"] = fresh["code"]
    ctx.check("sign-up again with the same address, in any case: 409 email_taken (a retry makes nothing)",
              _problem(reg(again), 409, "email_taken"))


def _reset_link(ctx: Ctx, uid: str, table: str) -> str:
    token = uuid.uuid4().hex + uuid.uuid4().hex
    digest = hashlib.sha256(token.encode()).hexdigest()
    ctx.sql(f"INSERT INTO {table} (user_id, token_hash, expires_at) VALUES ('{uid}', '{digest}', now() + interval '10 minutes')")
    return token


def password(ctx: Ctx, a: Account, disco: dict) -> None:
    uid, email, _ = ctx.make_user("v2acc-pw", PW)
    phone = a.sign_in(email, PW, "Phone")
    laptop = a.sign_in(email, PW, "Laptop")
    put = lambda tok, b: ctx.request("PUT", "/api/v2/me/password", tok, b)  # noqa: E731
    ctx.check("password: a weak new one is 400 weak_password",
              _problem(put(phone["access_token"], {"current_password": PW, "new_password": "short"}), 400, "weak_password"))
    ctx.check("password: a wrong current one is 403 invalid_credentials",
              _problem(put(phone["access_token"], {"current_password": "not-the-password", "new_password": NEW_PW}), 403, "invalid_credentials"))
    ctx.check("password: changed, 204", put(phone["access_token"], {"current_password": PW, "new_password": NEW_PW})[0] == 204)
    ctx.check("…this device stays signed in", ctx.request("GET", "/api/v2/me", phone["access_token"])[0] == 200)
    ctx.check("…every other device is signed out", _problem(ctx.request("GET", "/api/v2/me", laptop["access_token"]), 401)
              and a.oauth_error(a.token({"grant_type": "refresh_token", "refresh_token": laptop["refresh_token"]}), 400, "invalid_grant"))
    ctx.check("…the old password is refused, the new one works",
              a.oauth_error(a.token({"grant_type": "password", "username": email, "password": PW}), 400, "invalid_grant")
              and a.token({"grant_type": "password", "username": email, "password": NEW_PW})[0] == 200)
    ctx.check("password: the anonymous caller gets 401", _problem(put(None, {"current_password": NEW_PW, "new_password": PW}), 401))

    # Forgotten passwords.
    reset = lambda b: ctx.request("POST", "/api/v2/auth/password-reset", body=b)  # noqa: E731
    if (disco.get("auth") or {}).get("password_reset"):
        ctx.check("password reset: 202 for an address with an account", reset({"email": email})[0] == 202)
        ctx.check("…and 202 for one without (nothing told apart)", reset({"email": f"nobody-{ctx.sfx}@{MAIL_DOMAIN}"})[0] == 202)
    else:
        got = reset({"email": email})
        ctx.check("password reset on a server without mail: 501 feature_unavailable, as discovery says",
                  _problem(got, 501, "feature_unavailable") and _json(got).get("feature") == "password_reset")
    confirm = lambda b: ctx.request("POST", "/api/v2/auth/password-reset/confirm", body=b)  # noqa: E731
    ctx.check("reset link: an unknown one is 400 invalid_token",
              _problem(confirm({"token": "f" * 64, "password": PW}), 400, "invalid_token"))
    ctx.check("reset link: the password rule comes first, 400 weak_password",
              _problem(confirm({"token": "f" * 64, "password": "short"}), 400, "weak_password"))
    if not ctx.db_available:
        ctx.skip("reset link: a mailed link sets the password", "needs --compose-dir to plant a link")
        return
    token = _reset_link(ctx, uid, "password_resets")
    ctx.check("reset link: sets the password, 204", confirm({"token": token, "password": PW})[0] == 204)
    ctx.check("…and signs every device out, this one included",
              _problem(ctx.request("GET", "/api/v2/me", phone["access_token"]), 401))
    ctx.check("…the new password works", a.token({"grant_type": "password", "username": email, "password": PW})[0] == 200)
    ctx.check("…and the link works once", _problem(confirm({"token": token, "password": NEW_PW}), 400, "invalid_token"))


def email_verification(ctx: Ctx, a: Account, disco: dict) -> None:
    uid, email, _ = ctx.make_user("v2acc-mail", PW)
    tok = a.sign_in(email, PW)["access_token"]
    verify = lambda b: ctx.request("POST", "/api/v2/auth/email-verification", body=b)  # noqa: E731
    ctx.check("confirmation link: an unknown one is 400 invalid_token", _problem(verify({"token": "e" * 64}), 400, "invalid_token"))
    ctx.check("asking for a link on a confirmed address: 409 already_verified",
              _problem(ctx.request("POST", "/api/v2/me/email-verification", tok), 409, "already_verified"))
    if not ctx.db_available:
        ctx.skip("confirmation link: a mailed link confirms the address", "needs --compose-dir to plant a link")
        return
    ctx.sql(f"UPDATE users SET email_verified_at = NULL WHERE id = '{uid}'")
    me = ctx.call("GET", "/api/v2/me", tok)
    ctx.check("/me says the address is not confirmed", me.get("email_verified") is False)
    resend = ctx.request("POST", "/api/v2/me/email-verification", tok)
    if (disco.get("auth") or {}).get("email_verification"):
        ctx.check("asking for a link: 202", resend[0] == 202)
    else:
        ctx.check("asking for a link on a server without mail: 501 feature_unavailable, as discovery says",
                  _problem(resend, 501, "feature_unavailable") and _json(resend).get("feature") == "email_verification")
    token = _reset_link(ctx, uid, "email_verifications")
    ctx.check("confirmation link: 204 without signing in", verify({"token": token})[0] == 204)
    ctx.check("…and /me says so", ctx.call("GET", "/api/v2/me", tok).get("email_verified") is True)
    ctx.check("…once", _problem(verify({"token": token}), 400, "invalid_token"))


def security_events(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-trail", PW)
    _ouid, other_email, _ = ctx.make_user("v2acc-trail-other", PW)
    a.token({"grant_type": "password", "username": email, "password": "wrong-password-x"})
    tok = a.sign_in(email, PW, "Trail Phone")["access_token"]
    a.sign_in(other_email, PW, "Someone Else's Phone")
    st, _h, raw = ctx.request("GET", "/api/v2/me/security-events?limit=500", tok)
    page = json.loads(raw) if st == 200 else {}
    items = page.get("items", [])
    kinds = [e.get("type") for e in items]
    ctx.check("security events: the account's own trail, newest first",
              st == 200 and "login.ok" in kinds and "login.failed" in kinds
              and [e["at"] for e in items] == sorted((e["at"] for e in items), reverse=True), str(kinds)[:200])
    ok = next((e for e in items if e.get("type") == "login.ok" and e.get("device") == "Trail Phone"), None)
    ctx.check("…naming the device and only the network it came from",
              ok is not None and str(ok.get("ip_hint", "")).endswith("*") and ok.get("by_admin") is False, str(ok))
    ctx.check("…and nothing of another account's", all(e.get("device") != "Someone Else's Phone" for e in items))
    st, _h, raw = ctx.request("GET", "/api/v2/me/security-events?limit=1", tok)
    first = json.loads(raw) if st == 200 else {}
    nxt = first.get("next")
    second = ctx.call("GET", f"/api/v2/me/security-events?limit=1&cursor={urllib.parse.quote(nxt)}", tok) if nxt else {}
    ctx.check("…paged: one at a time, the second page goes on from the first",
              len(first.get("items", [])) == 1 and len(second.get("items", [])) == 1
              and [first["items"][0]["id"], second["items"][0]["id"]] == [e["id"] for e in items[:2]])
    ctx.check("…a made-up cursor is 400 invalid_cursor",
              _problem(ctx.request("GET", "/api/v2/me/security-events?cursor=bm9wZQ", tok), 400, "invalid_cursor"))


def two_factor(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-2fa", PW)
    tok = a.sign_in(email, PW)["access_token"]
    me = lambda: ctx.call("GET", "/api/v2/me", tok)  # noqa: E731
    ctx.check("two-factor: off on a new account", me().get("two_factor") == {"enabled": False, "recovery_codes_left": 0})
    ctx.check("two-factor: confirming with nothing pending is 409 two_factor_not_pending",
              _problem(ctx.request("PUT", "/api/v2/me/two-factor", tok, {"code": "123456"}), 409, "two_factor_not_pending"))
    ctx.check("two-factor: turning off what is not on is 409 two_factor_off",
              _problem(ctx.request("DELETE", "/api/v2/me/two-factor", tok), 409, "two_factor_off"))
    # A pending setup is dropped without a code.
    ctx.call("POST", "/api/v2/me/two-factor", tok, expect=(201,))
    ctx.check("two-factor: a pending setup is dropped without a code", ctx.request("DELETE", "/api/v2/me/two-factor", tok)[0] == 204)

    st, _h, raw = ctx.request("POST", "/api/v2/me/two-factor", tok)
    setup = json.loads(raw) if st == 201 else {}
    ctx.check("two-factor: setup answers a secret and an otpauth link, 201",
              st == 201 and bool(setup.get("secret")) and setup.get("otpauth_uri", "").startswith("otpauth://totp/"))
    secret = setup.get("secret", "")
    ctx.check("two-factor: a wrong code is 400 invalid_code",
              _problem(ctx.request("PUT", "/api/v2/me/two-factor", tok, {"code": "000000" if _totp(secret, time.time()) != "000000" else "111111"}), 400, "invalid_code"))
    st, _h, raw = ctx.request("PUT", "/api/v2/me/two-factor", tok, {"code": _totp(secret, time.time())})
    codes = json.loads(raw).get("recovery_codes", []) if st == 200 else []
    ctx.check("two-factor: on with the app's code, recovery codes shown once", st == 200 and len(codes) >= 8, f"{st} {raw[:120]!r}")
    ctx.check("…and /me says so", me().get("two_factor") == {"enabled": True, "recovery_codes_left": len(codes)})
    ctx.check("two-factor: setting up again while on is 409 two_factor_on",
              _problem(ctx.request("POST", "/api/v2/me/two-factor", tok), 409, "two_factor_on"))
    ctx.check("…and so is confirming again", _problem(ctx.request("PUT", "/api/v2/me/two-factor", tok, {"code": "123456"}), 409, "two_factor_on"))
    first = a.token({"grant_type": "password", "username": email, "password": PW})
    ctx.check("signing in now asks for the second factor", a.oauth_error(first, 403, "mfa_required"))
    st, _h, _b = a.token({"grant_type": "urn:own-audio:params:oauth:grant-type:mfa-otp",
                          "mfa_token": _json(first).get("mfa_token", ""), "otp": codes[0] if codes else ""})
    ctx.check("…a recovery code finishes it, once", st == 200 and me()["two_factor"]["recovery_codes_left"] == len(codes) - 1)
    ctx.check("two-factor: turning off needs a code (a session alone cannot)",
              _problem(ctx.request("DELETE", "/api/v2/me/two-factor", tok), 400, "invalid_code")
              and _problem(ctx.request("DELETE", "/api/v2/me/two-factor", tok, {"code": codes[0] if codes else "x"}), 400, "invalid_code"))
    ctx.check("two-factor: off with the next code from the app, 204",
              ctx.request("DELETE", "/api/v2/me/two-factor", tok, {"code": _totp(secret, time.time() + 30)})[0] == 204)
    ctx.check("…and signing in takes the password alone again",
              a.token({"grant_type": "password", "username": email, "password": PW})[0] == 200
              and me()["two_factor"]["enabled"] is False)


def profile(ctx: Ctx, a: Account) -> None:
    uid, email, _ = ctx.make_user("v2acc-me", PW)
    _ouid, other_email, _ = ctx.make_user("v2acc-me-other", PW)
    tok = a.sign_in(email, PW)["access_token"]
    other = a.sign_in(other_email, PW)["access_token"]
    before_other = ctx.call("GET", "/api/v2/me", other)
    st, _h, raw = ctx.request("PATCH", "/api/v2/me", tok, {"display_name": "  Renamed  ", "recommendations_enabled": True,
                                                          "discovery_languages": ["en-US", "cs", "en"]})
    me = json.loads(raw) if st == 200 else {}
    ctx.check("PATCH /me: the profile, answered whole", st == 200 and me.get("display_name") == "Renamed"
              and me.get("recommendations_enabled") is True and me.get("discovery_languages") == ["en", "cs"], f"{st} {raw[:200]!r}")
    v1 = ctx.call("GET", "/api/v1/users/me", tok)
    ctx.check("…the same account v1 reads", v1.get("display_name") == "Renamed" and v1.get("discovery_languages") == ["en", "cs"])
    ctx.check("…only what the body names changes",
              ctx.call("PATCH", "/api/v2/me", tok, {"recommendations_enabled": False}).get("display_name") == "Renamed")
    ctx.check("…and nobody else's", ctx.call("GET", "/api/v2/me", other) == before_other)
    ctx.check("PATCH /me: a blank name is 400 invalid_display_name",
              _problem(ctx.request("PATCH", "/api/v2/me", tok, {"display_name": " "}), 400, "invalid_display_name"))
    many = [f"{chr(97 + i // 26)}{chr(97 + i % 26)}" for i in range(21)]
    ctx.check("PATCH /me: more than 20 languages is 400 too_many_languages",
              _problem(ctx.request("PATCH", "/api/v2/me", tok, {"discovery_languages": many}), 400, "too_many_languages"))
    # Idempotency-Key: the same key and body answer the first result; another body is refused.
    key = {"Idempotency-Key": f"v2acc-{uuid.uuid4()}"}
    first = ctx.request("PATCH", "/api/v2/me", tok, {"display_name": "Keyed"}, headers=key)
    again = ctx.request("PATCH", "/api/v2/me", tok, {"display_name": "Keyed"}, headers=key)
    ctx.check("PATCH /me with an Idempotency-Key: a retry answers the first result",
              first[0] == again[0] == 200 and again[1].get("idempotent-replayed") == "true")
    ctx.check("…the same key with another body is 422 idempotency_key_reused",
              _problem(ctx.request("PATCH", "/api/v2/me", tok, {"display_name": "Other"}, headers=key), 422, "idempotency_key_reused"))
    _ = uid


def picture(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-pic", PW)
    tok = a.sign_in(email, PW)["access_token"]
    png = _png()
    put = lambda body, ctype: ctx.request("PUT", "/api/v2/me/image", tok, body, headers={"Content-Type": ctype})  # noqa: E731
    st, _h, raw = put(png, "image/png")
    me = json.loads(raw) if st == 200 else {}
    image = me.get("image") or ""
    ctx.check("PUT /me/image with the picture itself: the account, with a signed weekly link",
              st == 200 and "/api/v2/images/" in image and "e=" in image, f"{st} {raw[:200]!r}")
    if image:
        req = urllib.request.Request(image, headers={"User-Agent": "own.audio conformance"})
        try:
            with urllib.request.urlopen(req, timeout=30) as r:
                fetched = (r.status, r.read())
        except urllib.error.HTTPError as e:
            fetched = (e.code, b"")
        ctx.note("GET", image, fetched[0])
        ctx.check("…fetched without a token", fetched == (200, png))
    ctx.check("…the same link in /me", ctx.call("GET", "/api/v2/me", tok).get("image") == image)
    ctx.check("PUT /me/image: neither JSON nor an image is 415", _problem(put(b"hello", "text/plain"), 415, "unsupported_media_type"))
    ctx.check("PUT /me/image: over 5 MB is 413 too_large", _problem(put(b"\x89PNG" + b"\0" * (5 * 1024 * 1024), "image/png"), 413, "too_large"))
    ctx.check("PUT /me/image: an upload id that is not this household's is 404",
              _problem(ctx.request("PUT", "/api/v2/me/image", tok, {"upload_id": f"f/{uuid.uuid4()}/uploads/x.png"}), 404))
    # From an upload, the image pattern every resource shares.
    _aid, _ae, admin = ctx.make_family_admin("v2acc-pic-admin", PW)
    made = ctx.call("POST", "/api/v2/uploads", admin, {"kind": "track_cover", "filename": "me.png", "content_type": "image/png",
                                                       "size_bytes": len(png)}, expect=(201,))
    ctx.put_bytes(made["url"], png, made["headers"]["Content-Type"])
    st, _h, raw = ctx.request("PUT", "/api/v2/me/image", admin, {"upload_id": made["upload_id"]})
    by_upload = (json.loads(raw).get("image") if st == 200 else None) or ""
    ctx.check("PUT /me/image with {upload_id}: the account, with a new link", st == 200 and "/api/v2/images/" in by_upload)
    audio = ctx.call("POST", "/api/v2/uploads", admin, {"kind": "track", "filename": "x.mp3", "content_type": "audio/mpeg",
                                                        "size_bytes": len(ctx.audio())}, expect=(201,))
    ctx.put_bytes(audio["url"], ctx.audio(), audio["headers"]["Content-Type"])
    ctx.check("PUT /me/image with an upload that is not an image: 400 not_an_image",
              _problem(ctx.request("PUT", "/api/v2/me/image", admin, {"upload_id": audio["upload_id"]}), 400, "not_an_image"))
    if ctx.feature("one_family"):
        nid, nemail, _ = ctx.make_user("v2acc-pic-noupload", PW)
        ctx.call("PATCH", f"/api/v2/family/members/{nid}", admin, {"can_upload": False}, expect=(200, 204))
        ntok = a.sign_in(nemail, PW)["access_token"]
        ctx.check("a member who may not add to the library still sets a picture",
                  ctx.request("PUT", "/api/v2/me/image", ntok, png, headers={"Content-Type": "image/png"})[0] == 200)
    ctx.check("DELETE /me/image: 204, and /me has no picture",
              ctx.request("DELETE", "/api/v2/me/image", tok)[0] == 204 and "image" not in ctx.call("GET", "/api/v2/me", tok))


def subsonic(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-sub", PW)
    tok = a.sign_in(email, PW)["access_token"]
    key = ctx.call("GET", "/api/v2/me/subsonic-key", tok)
    ctx.check("the Subsonic key: the user name and a key, kept", key.get("username") == email and bool(key.get("key"))
              and ctx.call("GET", "/api/v2/me/subsonic-key", tok) == key)
    ctx.check("…the same as v1's", ctx.call("GET", "/api/v1/users/me/subsonic-key", tok).get("api_key") == key.get("key"))
    hdr = {"Idempotency-Key": f"v2acc-sub-{uuid.uuid4()}"}
    st, _h, raw = ctx.request("POST", "/api/v2/me/subsonic-key", tok, headers=hdr)
    new = json.loads(raw) if st == 200 else {}
    replay = ctx.request("POST", "/api/v2/me/subsonic-key", tok, headers=hdr)
    ctx.check("a new Subsonic key replaces the old", st == 200 and new.get("key") not in (None, key.get("key"))
              and ctx.call("GET", "/api/v2/me/subsonic-key", tok).get("key") == new.get("key"))
    ctx.check("…a retry with the same Idempotency-Key does not make a third",
              _json(replay).get("key") == new.get("key") and replay[1].get("idempotent-replayed") == "true")
    ping = ctx.request("GET", f"/rest/ping.view?f=json&c=conformance&v=1.16.1&apiKey={urllib.parse.quote(new.get('key', ''))}")
    old = ctx.request("GET", f"/rest/ping.view?f=json&c=conformance&v=1.16.1&apiKey={urllib.parse.quote(key.get('key', ''))}")
    status = lambda r: (_json(r).get("subsonic-response") or {}).get("status")  # noqa: E731
    ctx.check("…a Subsonic player signs in with the new key, not the old", status(ping) == "ok" and status(old) == "failed")


def deletion(ctx: Ctx, a: Account) -> None:
    uid, email, _ = ctx.make_user("v2acc-del", PW)
    tok = a.sign_in(email, PW)["access_token"]
    ctx.check("DELETE /me without the password: 400 password_required",
              _problem(ctx.request("DELETE", "/api/v2/me", tok), 400, "password_required"))
    ctx.check("DELETE /me with a wrong password: 403 invalid_credentials",
              _problem(ctx.request("DELETE", "/api/v2/me", tok, {"password": "not-the-password"}), 403, "invalid_credentials"))
    ctx.check("DELETE /me with the password: 204", ctx.request("DELETE", "/api/v2/me", tok, {"password": PW})[0] == 204)
    ctx.check("…its tokens stop working and it cannot sign in",
              _problem(ctx.request("GET", "/api/v2/me", tok), 401)
              and a.oauth_error(a.token({"grant_type": "password", "username": email, "password": PW}), 400, "invalid_grant"))
    _ = uid
    admins = [u for u in ctx.call("GET", "/api/v1/users", ctx.admin_token) if u.get("role") == "admin" and u.get("is_active")]
    if len(admins) == 1 and admins[0].get("email", "").lower() == ctx.admin_email.lower():
        got = ctx.request("DELETE", "/api/v2/me", ctx.admin_token, {"password": ctx.admin_password})
        ctx.check("the server's only admin cannot delete the account: 409 last_admin", _problem(got, 409, "last_admin"))
    else:
        ctx.skip("the last admin", "this server has more than one admin")


def exchange(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-fork", PW)
    app = a.sign_in(email, PW, "Mac")
    helper_req = {"grant_type": EXCHANGE, "subject_token": app["refresh_token"], "subject_token_type": REFRESH_TYPE,
                  "device_name": "Finder extension", "device_kind": "macos"}
    st, headers, raw = a.token(helper_req)
    helper = json.loads(raw) if st == 200 else {}
    ctx.check("token exchange: a refresh token gets a second device's tokens (RFC 8693)",
              st == 200 and helper.get("issued_token_type") == ACCESS_TYPE and bool(helper.get("access_token"))
              and helper.get("refresh_token") not in (None, app["refresh_token"]) and headers.get("cache-control") == "no-store",
              f"{st} {raw[:200]!r}")
    sessions = ctx.call("GET", "/api/v2/me/sessions", app["access_token"])
    forked = next((s for s in sessions if s.get("device_name") == "Finder extension"), None)
    ctx.check("…a device of its own in the session list", forked is not None and len(sessions) >= 2)
    st, _h, raw = a.token({"grant_type": "refresh_token", "refresh_token": app["refresh_token"]})
    ctx.check("…the presented refresh token keeps working (it was not rotated)", st == 200)
    app = json.loads(raw) if st == 200 else app
    ctx.check("…and the two refresh on their own",
              a.token({"grant_type": "refresh_token", "refresh_token": helper.get("refresh_token", "")})[0] == 200)
    if forked:
        ctx.call("DELETE", f"/api/v2/me/sessions/{forked['id']}", app["access_token"], expect=(204,))
        ctx.check("…the helper is signed out on its own, the app is not",
                  _problem(ctx.request("GET", "/api/v2/me", helper.get("access_token")), 401)
                  and ctx.request("GET", "/api/v2/me", app["access_token"])[0] == 200)
    trail = ctx.call("GET", "/api/v2/me/security-events", app["access_token"]).get("items", [])
    ctx.check("…on the trail as session.forked", any(e.get("type") == "session.forked" for e in trail))
    ctx.check("token exchange: no subject_token_type is 400 invalid_request",
              a.oauth_error(a.token({"grant_type": EXCHANGE, "subject_token": app["refresh_token"]}), 400, "invalid_request"))
    ctx.check("token exchange: an access token is not exchanged, 400 invalid_request",
              a.oauth_error(a.token({**helper_req, "subject_token": app["access_token"],
                                     "subject_token_type": ACCESS_TYPE}), 400, "invalid_request"))
    ctx.check("token exchange: an unknown refresh token is 400 invalid_grant",
              a.oauth_error(a.token({**helper_req, "subject_token": "f" * 64}), 400, "invalid_grant"))
    old = app["refresh_token"]
    a.token({"grant_type": "refresh_token", "refresh_token": old})
    ctx.check("token exchange: a rotated refresh token is 400 invalid_grant",
              a.oauth_error(a.token({**helper_req, "subject_token": old}), 400, "invalid_grant"))


def code_grant(ctx: Ctx, a: Account, disco: dict) -> None:
    grants = (disco.get("auth") or {}).get("grants") or []
    good = {"grant_type": "authorization_code", "provider": "google", "code": "c", "code_verifier": "v" * 43,
            "redirect_uri": "http://127.0.0.1:53682/"}
    ctx.check("code grant: a verifier outside RFC 7636 is 400 invalid_request",
              a.oauth_error(a.token({**good, "code_verifier": "short"}), 400, "invalid_request"))
    ctx.check("code grant: a redirect that is not loopback is 400 invalid_request",
              a.oauth_error(a.token({**good, "redirect_uri": "https://example.com/cb"}), 400, "invalid_request"))
    if "authorization_code" not in grants:
        ctx.check("code grant on a server without a desktop client: 400 invalid_request",
                  a.oauth_error(a.token(good), 400, "invalid_request"))
    else:
        ctx.check("code grant: a code the provider never issued is 400 invalid_grant",
                  a.oauth_error(a.token(good), 400, "invalid_grant"))
    ctx.skip("code grant: a real provider's code", "exchanged against a stub provider in the server's tests")


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, *args, **kwargs):
        return None


def _no_follow(ctx: Ctx, method: str, path: str, form: dict | None = None):
    """(status, Location, body) without following: Apple's page and the app's scheme are not ours."""
    data = urllib.parse.urlencode(form).encode() if form is not None else None
    req = urllib.request.Request(ctx.url(path), data=data, method=method,
                                 headers={"Content-Type": "application/x-www-form-urlencoded"} if data else {})
    try:
        with urllib.request.build_opener(_NoRedirect()).open(req, timeout=ctx.timeout) as r:
            ctx.note(method, path, r.status)
            return r.status, r.headers.get("Location", ""), r.read()
    except urllib.error.HTTPError as e:
        ctx.note(method, path, e.code)
        return e.code, e.headers.get("Location", ""), e.read()


def _pkce() -> tuple[str, str]:
    verifier = base64.urlsafe_b64encode(os.urandom(32)).rstrip(b"=").decode()
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    return verifier, challenge


def _start(ctx: Ctx, challenge: str, app_state: str, redirect: str = APPLE_REDIRECT):
    query = urllib.parse.urlencode({"redirect_uri": redirect, "code_challenge": challenge,
                                    "code_challenge_method": "S256", "state": app_state})
    return _no_follow(ctx, "GET", f"/api/v2/auth/apple/start?{query}")


def _query(location: str) -> dict:
    return dict(urllib.parse.parse_qsl(urllib.parse.urlsplit(location).query))


def _back(location: str, **expected: str) -> bool:
    """Whether a redirect goes to the app's return address with exactly these parameters."""
    return location.split("?")[0] == APPLE_REDIRECT and _query(location) == expected


def apple_web(ctx: Ctx, a: Account, disco: dict) -> None:
    auth = disco.get("auth") or {}
    on = auth.get("apple_web")
    ctx.check("discovery says whether Apple runs in a browser, and lists its grant with it",
              isinstance(on, bool) and (APPLE_CODE in (auth.get("grants") or [])) == on, str(auth.get("grants")))
    verifier, challenge = _pkce()
    if not on:
        st, _loc, body = _start(ctx, challenge, "app-state")
        ctx.check("Apple in a browser on a server without it: start is 501 feature_unavailable",
                  st == 501 and _json((st, {}, body)).get("feature") == "apple_web", f"{st} {body[:200]!r}")
        st, _loc, body = _no_follow(ctx, "POST", "/api/v2/auth/apple/return", {"state": "x", "error": "user_cancelled_authorize"})
        ctx.check("…and so is Apple's return", st == 501, f"{st} {body[:200]!r}")
        ctx.check("…and its grant is 400 invalid_request",
                  a.oauth_error(a.token({"grant_type": APPLE_CODE, "code": "c", "code_verifier": verifier}), 400, "invalid_request"))
        return

    st, loc, _ = _start(ctx, challenge, "app-state", redirect="https://example.com/apple")
    ctx.check("Apple in a browser: a web address to return to is 400", st == 400 and not loc, str(st))

    st, loc, _ = _start(ctx, challenge, "app state 1")
    q = _query(loc)
    services_id = ((auth.get("clients") or {}).get("apple") or {}).get("web")
    ctx.check("Apple in a browser: start sends the browser to Apple with the Services ID, the server's Return URL, a state and a nonce",
              st == 302 and loc.startswith("https://appleid.apple.com/auth/authorize?")
              and q.get("client_id") == services_id and q.get("redirect_uri", "").endswith("/api/v2/auth/apple/return")
              and q.get("response_type") == "code id_token" and q.get("response_mode") == "form_post"
              and q.get("scope") == "name email" and len(q.get("state", "")) >= 32 and len(q.get("nonce", "")) >= 32
              and "app state 1" not in loc and challenge not in loc, f"{st} {loc[:300]}")
    cancelled_state = q.get("state", "")

    st, loc, body = _no_follow(ctx, "POST", "/api/v2/auth/apple/return", {"state": "f" * 43, "error": "user_cancelled_authorize"})
    ctx.check("Apple's answer for a sign-in nobody started is 400, with no redirect",
              st == 400 and not loc and _json((st, {}, body)).get("code") == "unknown_sign_in", f"{st} {loc}")
    st, loc, _ = _no_follow(ctx, "POST", "/api/v2/auth/apple/return", {"state": cancelled_state, "error": "user_cancelled_authorize"})
    ctx.check("a cancelled sign-in goes back to the app with access_denied and the app's state",
              st == 303 and _back(loc, error="access_denied", state="app state 1"), f"{st} {loc}")
    st, loc, _ = _no_follow(ctx, "POST", "/api/v2/auth/apple/return", {"state": cancelled_state, "error": "user_cancelled_authorize"})
    ctx.check("…and its state is good once: a second answer is 400", st == 400 and not loc, f"{st} {loc}")

    st, loc, _ = _start(ctx, challenge, "app-state-2")
    st, loc, _ = _no_follow(ctx, "POST", "/api/v2/auth/apple/return",
                            {"state": _query(loc).get("state", ""), "id_token": "not.a.token", "code": "apple-code"})
    ctx.check("an id token that does not verify goes back to the app with server_error",
              st == 303 and _back(loc, error="server_error", state="app-state-2"), f"{st} {loc}")

    if not ctx.db_available:
        ctx.skip("Apple in a browser: an expired sign-in and the code grant", "needs database access (--compose-dir)")
        return

    def started() -> tuple[str, str]:
        _st, location, _ = _start(ctx, challenge, "app-state-3")
        server_state = _query(location).get("state", "")
        return server_state, hashlib.sha256(server_state.encode()).hexdigest()

    server_state, state_hash = started()
    ctx.sql(f"UPDATE apple_web_sign_ins SET expires_at = now() - interval '1 second' WHERE state_hash = '{state_hash}'")
    st, loc, _ = _no_follow(ctx, "POST", "/api/v2/auth/apple/return", {"state": server_state, "error": "user_cancelled_authorize"})
    ctx.check("Apple's answer after the sign-in expired (five minutes) is 400, with no redirect", st == 400 and not loc, f"{st} {loc}")

    uid, email, _tok = ctx.make_user("v2acc-apple", PW)

    def signed(code: str, *, expired: bool = False) -> None:
        """Apple's verified answer, as `/auth/apple/return` keeps it."""
        _state, h = started()
        code_hash = hashlib.sha256(code.encode()).hexdigest()
        ctx.sql("UPDATE apple_web_sign_ins SET status = 'signed', "
                f"code_hash = '{code_hash}', subject = 'conformance-apple-{ctx.sfx}', email = '{email}', email_verified = true, "
                f"expires_at = now() + interval '{'-1 second' if expired else '5 minutes'}' WHERE state_hash = '{h}'")

    grant = {"grant_type": APPLE_CODE, "code_verifier": verifier, "device_name": "Conformance Android", "device_kind": "android"}
    wrong, _ = _pkce()
    signed(f"code-wrong-verifier-{ctx.sfx}")
    ctx.check("apple-code grant: a wrong verifier is 400 invalid_grant",
              a.oauth_error(a.token({**grant, "code": f"code-wrong-verifier-{ctx.sfx}", "code_verifier": wrong}), 400, "invalid_grant"))
    ctx.check("…and spends the code: the right verifier is refused after it",
              a.oauth_error(a.token({**grant, "code": f"code-wrong-verifier-{ctx.sfx}"}), 400, "invalid_grant"))
    signed(f"code-expired-{ctx.sfx}", expired=True)
    ctx.check("apple-code grant: an expired code is 400 invalid_grant",
              a.oauth_error(a.token({**grant, "code": f"code-expired-{ctx.sfx}"}), 400, "invalid_grant"))
    ctx.check("apple-code grant: a code never issued is 400 invalid_grant",
              a.oauth_error(a.token({**grant, "code": f"code-never-{ctx.sfx}"}), 400, "invalid_grant"))
    signed(f"code-good-{ctx.sfx}")
    st, _h, raw = a.token({**grant, "code": f"code-good-{ctx.sfx}"})
    tokens = _json((st, _h, raw))
    me = ctx.call("GET", "/api/v2/me", tokens.get("access_token")) if st == 200 else {}
    ctx.check("apple-code grant: the right verifier signs in to the account with Apple's verified email",
              st == 200 and bool(tokens.get("refresh_token")) and me.get("id") == uid, f"{st} {raw[:200]!r}")
    ctx.check("…and the code is good once", a.oauth_error(a.token({**grant, "code": f"code-good-{ctx.sfx}"}), 400, "invalid_grant"))
    linked = ctx.sql(f"SELECT count(*) FROM auth_identities WHERE user_id = '{uid}' AND provider = 'apple'")
    ctx.check("…and Apple's identity is linked to the account, as with Apple's id token", linked.strip() == "1", linked)


def personal_tokens(ctx: Ctx, a: Account) -> None:
    _uid, email, _ = ctx.make_user("v2acc-pat", PW)
    tok = a.sign_in(email, PW)["access_token"]
    pat = ctx.call("POST", "/api/v2/me/tokens", tok, {"name": "account script", "scopes": ["user:write"]}, expect=(201,))["token"]
    refused = [(m, p) for m, p, b in [
        ("PUT", "/api/v2/me/password", {"current_password": PW, "new_password": NEW_PW}),
        ("POST", "/api/v2/me/two-factor", None),
        ("GET", "/api/v2/me/subsonic-key", None),
        ("POST", "/api/v2/me/subsonic-key", None),
        ("DELETE", "/api/v2/me", {"password": PW}),
    ] if not _problem(ctx.request(m, p, pat, b), 403, "forbidden")]
    ctx.check("a personal token cannot reach the account's credentials (403)", not refused, str(refused))
    ctx.check("…but may change the profile with user:write",
              ctx.request("PATCH", "/api/v2/me", pat, {"display_name": "Scripted"})[0] == 200)
    ctx.check("…and the account is still there", ctx.call("GET", "/api/v2/me", tok).get("display_name") == "Scripted")
