# SPDX-License-Identifier: AGPL-3.0-or-later
"""/api/v2 (docs/api-v2-plan.md): discovery, problem details on every error, and every operation's
declared permission (`x-permission` in docs/api/openapi-v2.json) checked against the answers.
Grows with each phase of the plan.
"""
from __future__ import annotations

import json
import pathlib
import uuid

from core import Ctx

NAME = "v2"
PW = "V2SuiteTest1234!"

OPENAPI = pathlib.Path(__file__).resolve().parents[2] / "docs" / "api" / "openapi-v2.json"
PERMISSIONS = {"public", "user", "member", "family_admin", "instance_admin", "can_upload", "can_generate", "owner"}
PROBLEM = "application/problem+json"


def _ops(doc: dict) -> list[tuple[str, str, dict]]:
    return [(m.upper(), p, o) for p, methods in doc["paths"].items() for m, o in methods.items()
            if m in ("get", "post", "put", "patch", "delete")]


def _fill(path: str) -> str:
    return "/".join(str(uuid.uuid4()) if s.startswith("{") else s for s in path.split("/"))


def run(ctx: Ctx) -> None:
    def problem(answer: tuple[int, dict, bytes], status: int, code: str | None = None) -> bool:
        got, headers, body = answer
        if got != status or not headers.get("content-type", "").startswith(PROBLEM):
            return False
        try:
            doc = json.loads(body)
        except ValueError:
            return False
        return (doc.get("status") == status and doc.get("type", "").startswith("urn:own-audio:problem:")
                and bool(doc.get("title")) and (code is None or doc.get("code") == code))

    # Discovery, without signing in.
    status, _h, body = ctx.request("GET", "/.well-known/own-audio")
    disco = json.loads(body) if status == 200 else {}
    v2 = next((a for a in disco.get("api", []) if a.get("version") == 2), {})
    ctx.check("/.well-known/own-audio answers without signing in and lists /api/v2",
              status == 200 and v2.get("path") == "/api/v2", f"{status} {str(disco)[:160]}")
    server = ctx.call("GET", "/api/v1/server")
    ctx.check("discovery names the same server as /api/v1/server",
              disco.get("id") == server.get("id") and disco.get("version") == server.get("version"))
    ctx.check("discovery states the list limits", (disco.get("limits") or {}).get("list_limit_max") == 500)

    # Problem details on every kind of error.
    ctx.check("no token: 401 problem", problem(ctx.request("GET", "/api/v2/me"), 401, "unauthorized"))
    ctx.check("a made-up token: 401 problem",
              problem(ctx.request("GET", "/api/v2/me", token="not.a.token"), 401))
    _id, _email, tok = ctx.make_family_admin("v2", PW)
    ctx.check("an unknown path: 404 problem", problem(ctx.request("GET", "/api/v2/nowhere", tok), 404, "not_found"))

    status, _h, body = ctx.request("GET", "/api/v2/me", tok)
    me = json.loads(body) if status == 200 else {}
    ctx.check("/me: the account and its household", status == 200 and me.get("id") == _id
              and (me.get("family") or {}).get("role") == "admin" and me["family"].get("can_upload") is True, str(me)[:160])

    # Every operation says who may call it, and the server agrees.
    doc = json.loads(OPENAPI.read_text())
    undeclared = [f"{m} {p}" for m, p, o in _ops(doc) if o.get("x-permission") not in PERMISSIONS]
    ctx.check("every v2 operation declares its permission", not undeclared, ", ".join(undeclared))
    _mid, _memail, member = ctx.make_user("v2member", PW)
    wrong = []
    for method, path, op in _ops(doc):
        perm = op.get("x-permission")
        if perm == "public":
            continue
        if not problem(ctx.request(method, _fill(path)), 401):
            wrong.append(f"anonymous {method} {path}")
        if perm in ("family_admin", "instance_admin"):
            got = ctx.request(method, _fill(path), member)
            if not (problem(got, 403) or problem(got, 404)):
                wrong.append(f"member {method} {path}: {got[0]}")
    ctx.check("anonymous callers get 401 and members no admin operation, as declared", not wrong, "; ".join(wrong[:8]))

    signin(ctx, problem)
    catalogue(ctx, problem)
    shelves(ctx, problem)


def _form(fields: dict) -> tuple[bytes, dict]:
    import urllib.parse
    return urllib.parse.urlencode(fields).encode(), {"Content-Type": "application/x-www-form-urlencoded"}


def signin(ctx: Ctx, problem) -> None:
    """V2: tokens in the OAuth shape for every grant, revocation, the device grant (RFC 8628),
    sessions, and scoped personal access tokens."""
    import base64 as b64
    import hmac
    import struct
    import time

    def oauth_error(answer, status: int, error: str) -> bool:
        return problem(answer, status, error) and json.loads(answer[2]).get("error") == error

    def token(fields: dict, as_json: bool = False):
        if as_json:
            return ctx.request("POST", "/api/v2/auth/token", body=fields)
        data, headers = _form(fields)
        return ctx.request("POST", "/api/v2/auth/token", body=data, headers=headers)

    uid, email, _tok = ctx.make_user("v2signin", PW)
    st, headers, body = token({"grant_type": "password", "username": email, "password": PW, "device_name": "Conformance"})
    issued = json.loads(body) if st == 200 else {}
    ctx.check("password grant: the OAuth token response, never cached",
              st == 200 and issued.get("token_type") == "Bearer" and issued.get("expires_in", 0) > 0
              and bool(issued.get("refresh_token")) and "library:read" in issued.get("scope", "").split()
              and "admin" not in issued.get("scope", "").split() and headers.get("cache-control") == "no-store",
              f"{st} {str(issued)[:120]}")
    st, _h, body = token({"grant_type": "password", "username": email, "password": PW}, as_json=True)
    ctx.check("the token endpoint takes JSON too", st == 200)
    ctx.check("a wrong password: 400 invalid_grant",
              oauth_error(token({"grant_type": "password", "username": email, "password": "wrong-password-1"}), 400, "invalid_grant"))
    ctx.check("a missing field: 400 invalid_request", oauth_error(token({"grant_type": "password", "username": email}), 400, "invalid_request"))
    ctx.check("an unknown grant: 400 unsupported_grant_type",
              oauth_error(token({"grant_type": "client_credentials"}), 400, "unsupported_grant_type"))

    st, _h, body = token({"grant_type": "refresh_token", "refresh_token": issued["refresh_token"]})
    rotated = json.loads(body) if st == 200 else {}
    ctx.check("refresh grant rotates the refresh token",
              st == 200 and rotated.get("refresh_token") not in (None, issued["refresh_token"]))
    ctx.check("a used refresh token: invalid_grant",
              oauth_error(token({"grant_type": "refresh_token", "refresh_token": issued["refresh_token"]}), 400, "invalid_grant"))
    ctx.check("…and its reuse signed the device out",
              oauth_error(token({"grant_type": "refresh_token", "refresh_token": rotated.get("refresh_token", "")}), 400, "invalid_grant"))

    _st, _h, body = token({"grant_type": "password", "username": email, "password": PW, "device_name": "Revoked"})
    revokable = json.loads(body)
    data, headers = _form({"token": revokable["refresh_token"]})
    st, _h, _b = ctx.request("POST", "/api/v2/auth/revoke", body=data, headers=headers)
    ctx.check("revoke: 200, and the refresh token is dead", st == 200 and oauth_error(
        token({"grant_type": "refresh_token", "refresh_token": revokable["refresh_token"]}), 400, "invalid_grant"))
    data, headers = _form({"token": "never-issued"})
    ctx.check("revoking an unknown token is 200 too", ctx.request("POST", "/api/v2/auth/revoke", body=data, headers=headers)[0] == 200)

    # The device grant, RFC 8628.
    _st, _h, body = token({"grant_type": "password", "username": email, "password": PW, "device_name": "Phone"})
    phone = json.loads(body)["access_token"]
    st, _h, body = ctx.request("POST", "/api/v2/auth/device", body={"device_name": "Living room TV", "device_kind": "tv"})
    tv = json.loads(body) if st == 200 else {}
    ctx.check("device authorization: codes, interval and a link with the code in it",
              st == 200 and len(tv.get("device_code", "")) >= 32 and "-" in tv.get("user_code", "")
              and tv.get("interval", 0) > 0 and (not tv.get("verification_uri")
                                                  or tv.get("verification_uri_complete", "").endswith(tv.get("user_code", ""))),
              str(tv)[:160])
    poll = {"grant_type": "urn:ietf:params:oauth:grant-type:device_code", "device_code": tv.get("device_code", "")}
    ctx.check("polling before approval: authorization_pending", oauth_error(token(poll), 400, "authorization_pending"))
    ctx.check("polling too fast: slow_down", oauth_error(token(poll), 400, "slow_down"))
    st, _h, body = ctx.request("GET", f"/api/v2/auth/device/{tv['user_code'].lower().replace('-', '')}", phone)
    ctx.check("the approver sees what asks, whatever case the code is typed in",
              st == 200 and json.loads(body).get("device_name") == "Living room TV")
    ctx.check("approve: 204", ctx.request("POST", f"/api/v2/auth/device/{tv['user_code']}/approve", phone)[0] == 204)
    st, _h, body = token(poll)
    ctx.check("the next poll gets tokens", st == 200 and bool(json.loads(body).get("access_token")))
    tv_token = json.loads(body).get("access_token") if st == 200 else None
    ctx.check("…once: expired_token after", oauth_error(token(poll), 400, "expired_token"))
    _st, _h, body = ctx.request("POST", "/api/v2/auth/device", body={"device_name": "Car"})
    car = json.loads(body)
    ctx.request("POST", f"/api/v2/auth/device/{car['user_code']}/deny", phone)
    ctx.check("a denied device: access_denied", oauth_error(
        token({"grant_type": "urn:ietf:params:oauth:grant-type:device_code", "device_code": car["device_code"]}), 400, "access_denied"))

    # Sessions.
    st, _h, body = ctx.request("GET", "/api/v2/me/sessions", phone)
    sessions = json.loads(body) if st == 200 else []
    tv_session = next((s for s in sessions if s.get("device_name") == "Living room TV"), None)
    ctx.check("sessions list the devices, this one marked current",
              any(s.get("current") and s.get("device_name") == "Phone" for s in sessions) and tv_session is not None,
              str([(s.get("device_name"), s.get("current")) for s in sessions]))
    if tv_session and tv_token:
        ctx.check("signing a device out: 204", ctx.request("DELETE", f"/api/v2/me/sessions/{tv_session['id']}", phone)[0] == 204)
        ctx.check("…and its access token stops working at once", problem(ctx.request("GET", "/api/v2/me", tv_token), 401))
    ctx.check("another account's device: 404", problem(ctx.request("DELETE", f"/api/v2/me/sessions/{uuid.uuid4()}", phone), 404))

    # Personal access tokens.
    st, _h, body = ctx.request("POST", "/api/v2/me/tokens", phone, {"name": "read-only script", "scopes": ["library:read"]})
    pat = json.loads(body) if st == 201 else {}
    ctx.check("a personal token is shown once, scoped", st == 201 and pat.get("token", "").startswith("oat_")
              and pat.get("scopes") == ["library:read"], f"{st} {str(pat)[:120]}")
    secret = pat.get("token", "")
    st, _h, body = ctx.request("GET", "/api/v2/me", secret)
    ctx.check("it reads what its scope allows", st == 200 and json.loads(body).get("id") == uid)
    ctx.check("and nothing else: 403 insufficient_scope",
              problem(ctx.request("GET", "/api/v2/me/sessions", secret), 403, "insufficient_scope"))
    ctx.check("it does not work on /api/v1", ctx.request("GET", "/api/v1/auth/me", secret)[0] == 401)
    listed = json.loads(ctx.request("GET", "/api/v2/me/tokens", phone)[2])
    ctx.check("the list never shows a secret", any(t.get("id") == pat.get("id") for t in listed)
              and all("token" not in t for t in listed))
    st, _h, body = ctx.request("POST", "/api/v2/me/tokens", phone, {"name": "token maker", "scopes": ["user:write"]})
    maker = json.loads(body).get("token", "") if st == 201 else ""
    ctx.check("a token cannot make tokens", problem(
        ctx.request("POST", "/api/v2/me/tokens", maker, {"name": "x", "scopes": ["library:read"]}), 403, "forbidden"))
    ctx.check("a scope the account lacks: 400 invalid_scope",
              problem(ctx.request("POST", "/api/v2/me/tokens", phone, {"name": "x", "scopes": ["admin"]}), 400, "invalid_scope"))
    ctx.check("revoked: 204, then 401", ctx.request("DELETE", f"/api/v2/me/tokens/{pat.get('id')}", phone)[0] == 204
              and problem(ctx.request("GET", "/api/v2/me", secret), 401))
    data, headers = _form({"token": maker})
    ctx.request("POST", "/api/v2/auth/revoke", body=data, headers=headers)
    ctx.check("a personal token revoked through /auth/revoke stops working", problem(ctx.request("GET", "/api/v2/me/tokens", maker), 401))

    # Two-factor: the first step is mfa_required, the mfa-otp grant finishes it.
    def totp(secret_b32: str, at: float) -> str:
        key = b64.b32decode(secret_b32 + "=" * (-len(secret_b32) % 8))
        digest = hmac.new(key, struct.pack(">Q", int(at // 30)), "sha1").digest()
        off = digest[19] & 0x0F
        return f"{(int.from_bytes(digest[off:off + 4], 'big') & 0x7FFFFFFF) % 1_000_000:06d}"

    _mid, mfa_email, mfa_tok = ctx.make_user("v2mfa", PW)
    setup = ctx.call("POST", "/api/v1/auth/totp/setup", mfa_tok)
    ctx.call("POST", "/api/v1/auth/totp/enable", mfa_tok, {"code": totp(setup["secret"], time.time())})
    first = token({"grant_type": "password", "username": mfa_email, "password": PW})
    challenge = json.loads(first[2]) if first[0] == 403 else {}
    ctx.check("two-factor on: 403 mfa_required with an mfa_token", oauth_error(first, 403, "mfa_required")
              and bool(challenge.get("mfa_token")))
    st, _h, body = token({"grant_type": "urn:own-audio:params:oauth:grant-type:mfa-otp",
                          "mfa_token": challenge.get("mfa_token", ""), "otp": totp(setup["secret"], time.time() + 30)})
    ctx.check("the mfa-otp grant with the next code signs in", st == 200 and bool(json.loads(body).get("access_token")))


def catalogue(ctx: Ctx, problem) -> None:
    """V3: the music catalogue — normalised records, cursor pages without loss or repeats,
    include=, and nothing of another household's or another member's private items."""
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2cat", PW)

    def get(path: str, token: str = tok):
        st, _h, body = ctx.request("GET", path, token)
        return st, (json.loads(body) if body else None)

    one = ctx.upload_track(tok, f"Alpha {sfx}", artist=f"Cat Band {sfx} feat. Cat Guest {sfx}", album=f"Cat Album A {sfx}", genre="Rock; Pop")
    ctx.upload_track(tok, f"Beta {sfx}", artist=f"Cat Band {sfx}", album=f"Cat Album B {sfx}")
    ctx.upload_track(tok, f"Gamma {sfx}", artist=f"Cat Band {sfx}", album=f"Cat Album C {sfx}")

    st, track = get(f"/api/v2/tracks/{one['id']}?include=albums,contributors,genres")
    ctx.check("a track: ids, not names, with its credits in order",
              st == 200 and track.get("album_id") == one.get("album_id")
              and [c["role"] for c in track.get("credits", [])] == ["artist", "featured"]
              and track["credits"][0]["join_phrase"] == " feat. " and "artist" not in track, str(track)[:200])
    inc = (track or {}).get("included", {})
    ctx.check("include= sends the album, the contributors and the genres once",
              [a["id"] for a in inc.get("albums", [])] == [one["album_id"]]
              and {c["name"] for c in inc.get("contributors", [])} == {f"Cat Band {sfx}", f"Cat Guest {sfx}"}
              and {g["name"] for g in inc.get("genres", [])} == {"Rock", "Pop"}, str(inc)[:200])
    ctx.check("durations in milliseconds, never seconds", "duration_ms" in (track or {}) and "duration_secs" not in (track or {}))

    st, artists = get(f"/api/v2/artists?q=Cat%20Band%20{sfx}")
    band = next((a for a in (artists or {}).get("items", []) if a["name"] == f"Cat Band {sfx}"), None)
    ctx.check("/artists finds the artist", st == 200 and band is not None, str(artists)[:160])
    st, guests = get(f"/api/v2/artists?q=Cat%20Guest%20{sfx}")
    ctx.check("…and not a featured guest, who is a contributor all the same",
              st == 200 and not guests.get("items") and get(f"/api/v2/contributors?role=featured&q=Cat%20Guest%20{sfx}")[1]["items"])
    if band:
        st, detail = get(f"/api/v2/contributors/{band['id']}")
        ctx.check("a contributor says what they are credited as", st == 200 and detail.get("roles") == ["artist"], str(detail)[:160])

    seen, cursor, pages = [], None, 0
    while pages < 10:
        path = f"/api/v2/albums?q=Cat%20Album&limit=1&contributor_id={band['id'] if band else ''}" + (f"&cursor={cursor}" if cursor else "")
        st, page = get(path)
        if st != 200:
            break
        seen += [a["title"] for a in page["items"]]
        pages += 1
        cursor = page.get("next")
        if not cursor:
            break
    ctx.check("albums page one at a time without loss or repeats",
              seen == [f"Cat Album A {sfx}", f"Cat Album B {sfx}", f"Cat Album C {sfx}"], str(seen))
    st, page = get(f"/api/v2/albums?contributor_id={band['id'] if band else uuid.uuid4()}&include=contributors,release_groups")
    first = (page or {}).get("items", [{}])[0]
    ctx.check("an album: credits, counts in what the caller sees, its release group included",
              st == 200 and first.get("track_count") == 1 and first.get("credits") and first.get("release_group_id")
              in [g["id"] for g in page.get("included", {}).get("release_groups", [])], str(first)[:200])
    st, group = get(f"/api/v2/release-groups/{first.get('release_group_id')}")
    ctx.check("a release group lists its editions", st == 200 and first.get("id") in group.get("album_ids", []))
    st, tracks = get(f"/api/v2/tracks?album_id={one['album_id']}")
    ctx.check("an album's tracks", st == 200 and [t["id"] for t in tracks["items"]] == [one["id"]])
    st, genres = get("/api/v2/genres?limit=500")
    ctx.check("genres of visible tracks", st == 200 and {"Rock", "Pop"} <= {g["name"] for g in genres["items"]})
    ctx.check("a bad cursor: 400 invalid_cursor", problem(ctx.request("GET", "/api/v2/albums?cursor=bm9wZQ", tok), 400, "invalid_cursor"))
    ctx.check("an unknown include: 400 invalid_include", problem(ctx.request("GET", "/api/v2/albums?include=tracks", tok), 400, "invalid_include"))
    ctx.check("an unknown id: 404", problem(ctx.request("GET", f"/api/v2/albums/{uuid.uuid4()}", tok), 404, "not_found"))

    # Nothing private of someone else's, from another member or another household.
    _oid, _oemail, other = ctx.make_user("v2cat-other", PW)
    for path in (f"/api/v2/tracks/{one['id']}", f"/api/v2/albums/{one['album_id']}", f"/api/v2/contributors/{band['id'] if band else uuid.uuid4()}"):
        if not problem(ctx.request("GET", path, other), 404):
            ctx.check(f"another account cannot see a private {path.split('/')[3][:-1]}", False, path)
            break
    else:
        ctx.check("another account sees none of a private track, its album or its artist", True)


def shelves(ctx: Ctx, problem) -> None:
    """V3b: books with their credits, files and chapters; the podcast list."""
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2books", PW)
    made = ctx.call("POST", "/api/v1/audiobooks", tok,
                    {"title": f"Shelf Book {sfx}", "author": f"Shelf Writer {sfx} & Shelf Cowriter {sfx}",
                     "narrator": f"Shelf Voice {sfx}"})
    st, _h, body = ctx.request("GET", f"/api/v2/books?q=Shelf%20Book%20{sfx}&include=contributors", tok)
    page = json.loads(body) if st == 200 else {}
    items = page.get("items", [])
    ctx.check("/books finds the book, credits in order and the contributors included",
              [b["id"] for b in items] == [made["id"]]
              and [(c["role"], c["position"]) for c in items[0]["credits"]] == [("author", 0), ("author", 1), ("narrator", 0)]
              and {k["name"] for k in page.get("included", {}).get("contributors", [])}
              == {f"Shelf Writer {sfx}", f"Shelf Cowriter {sfx}", f"Shelf Voice {sfx}"}, str(page)[:240])
    st, _h, body = ctx.request("GET", f"/api/v2/books/{made['id']}", tok)
    book = json.loads(body) if st == 200 else {}
    ctx.check("a book: files, chapters and series lists, its people included",
              st == 200 and isinstance(book.get("files"), list) and isinstance(book.get("chapters"), list)
              and isinstance(book.get("series"), list) and len(book.get("included", {}).get("contributors", [])) == 3, str(book)[:200])
    writer = next((c["contributor_id"] for c in book.get("credits", []) if c["role"] == "author"), None)
    st, _h, body = ctx.request("GET", f"/api/v2/authors?q=Shelf%20Writer%20{sfx}", tok)
    ctx.check("/authors lists the writer, and books filter by them",
              st == 200 and [a["id"] for a in json.loads(body)["items"]] == [writer]
              and [b["id"] for b in json.loads(ctx.request("GET", f"/api/v2/books?contributor_id={writer}", tok)[2])["items"]] == [made["id"]])
    st, _h, body = ctx.request("GET", "/api/v2/podcasts?sort=added", tok)
    ctx.check("/podcasts answers a page", st == 200 and isinstance(json.loads(body).get("items"), list))
    _oid, _oemail, other = ctx.make_user("v2books-other", PW)
    ctx.check("another account cannot see a private book", problem(ctx.request("GET", f"/api/v2/books/{made['id']}", other), 404))
