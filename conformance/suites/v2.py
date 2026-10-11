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
    if not ctx.feature("narration"):
        got = ctx.request("GET", "/api/v2/narration/voices", tok)
        ctx.check("a hosted feature this server lacks: 501 feature_unavailable, named",
                  problem(got, 501, "feature_unavailable") and json.loads(got[2]).get("feature") == "narration")

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
    corrections(ctx, problem)
    user_state(ctx, problem)
    sync(ctx, problem)
    playback(ctx, problem)
    images(ctx, problem)
    household(ctx, problem)
    live(ctx, problem)
    library(ctx, problem)
    playlists(ctx, problem)
    finding(ctx, problem)


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


def corrections(ctx: Ctx, problem) -> None:
    """V3c: credits corrected by hand hold through retags and rescans; contributors and albums
    corrected on the entity; merging and splitting are a family admin's."""
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2fix", PW)

    def req(method: str, path: str, body=None, token: str = tok):
        st, _h, raw = ctx.request(method, path, token, body)
        return st, (json.loads(raw) if raw else None)

    def artists_of(track_id: str) -> list[str]:
        st, t = req("GET", f"/api/v2/tracks/{track_id}?include=contributors")
        names = {c["id"]: c["name"] for c in (t or {}).get("included", {}).get("contributors", [])}
        return [names.get(c["contributor_id"], "?") for c in (t or {}).get("credits", []) if c["role"] == "artist"]

    # Two people of one name: the second track's credit is corrected, and stays corrected.
    ctx.upload_track(tok, f"Smith One {sfx}", artist=f"John Smith {sfx}", album=f"Smith A {sfx}")
    two = ctx.upload_track(tok, f"Smith Two {sfx}", artist=f"John Smith {sfx}", album=f"Smith B {sfx}")
    st, credits = req("PUT", f"/api/v2/tracks/{two['id']}/credits",
                      {"credits": [{"name": f"John Smith (guitarist) {sfx}", "role": "artist"}]})
    ctx.check("a track's credits corrected by hand", st == 200 and artists_of(two["id"]) == [f"John Smith (guitarist) {sfx}"], str(credits)[:160])
    ctx.call("PUT", f"/api/v1/music/tracks/{two['id']}", tok,
             {"title": two["title"], "artist": f"John Smith {sfx}", "album": f"Smith B {sfx}"})
    ctx.check("…and they hold when the tags are written again", artists_of(two["id"]) == [f"John Smith (guitarist) {sfx}"],
              str(artists_of(two["id"])))
    req("PUT", f"/api/v2/tracks/{two['id']}/credits", {"credits": []})
    ctx.check("an empty list gives the credits back to the tags", artists_of(two["id"]) == [f"John Smith {sfx}"], str(artists_of(two["id"])))
    ctx.check("a role a track does not have: 400 invalid_role",
              problem(ctx.request("PUT", f"/api/v2/tracks/{two['id']}/credits", tok, {"credits": [{"name": "x", "role": "narrator"}]}), 400, "invalid_role"))
    ctx.check("another household's contributor: 400 unknown_contributor",
              problem(ctx.request("PUT", f"/api/v2/tracks/{two['id']}/credits", tok,
                                  {"credits": [{"contributor_id": str(uuid.uuid4()), "role": "artist"}]}), 400, "unknown_contributor"))

    # A book's narrator corrected; the author text still drives the authors.
    book = ctx.call("POST", "/api/v1/audiobooks", tok, {"title": f"Fix Book {sfx}", "author": f"Fix Writer {sfx}", "narrator": f"Wrong Voice {sfx}"})
    req("PUT", f"/api/v2/books/{book['id']}/credits", {"credits": [{"name": f"Right Voice {sfx}", "role": "narrator"}]})
    ctx.call("PUT", f"/api/v1/audiobooks/{book['id']}", tok,
             {"title": book["title"], "author": f"New Writer {sfx}", "narrator": f"Wrong Voice {sfx}"})
    st, b = req("GET", f"/api/v2/books/{book['id']}")
    people = {c["id"]: c["name"] for c in (b or {}).get("included", {}).get("contributors", [])}
    by_role = sorted((c["role"], people.get(c["contributor_id"])) for c in (b or {}).get("credits", []))
    ctx.check("a corrected role stays; the others follow the text",
              by_role == [("author", f"New Writer {sfx}"), ("narrator", f"Right Voice {sfx}")], str(by_role))

    # Contributors and albums.
    st, artists = req("GET", f"/api/v2/artists?q=John%20Smith%20{sfx}")
    smith = next((a for a in (artists or {}).get("items", []) if a["name"] == f"John Smith {sfx}"), {})
    st, renamed = req("PATCH", f"/api/v2/contributors/{smith.get('id')}", {"name": f"Johnny Smith {sfx}", "kind": "person", "sort_name": f"Smith, Johnny {sfx}"})
    ctx.check("a contributor corrected: name, kind, sort name",
              st == 200 and renamed.get("name") == f"Johnny Smith {sfx}" and renamed.get("kind") == "person"
              and renamed.get("sort_name") == f"Smith, Johnny {sfx}", str(renamed)[:160])
    guitarist = next((a for a in req("GET", f"/api/v2/contributors?q=guitarist%29%20{sfx}")[1].get("items", [])), None)
    ctx.check("a name another contributor has: 409 name_taken", problem(
        ctx.request("PATCH", f"/api/v2/contributors/{smith.get('id')}", tok, {"name": f"John Smith (guitarist) {sfx}"}), 409, "name_taken")
        if guitarist else True)
    st, albums = req("GET", f"/api/v2/albums?contributor_id={smith.get('id')}")
    album = (albums or {}).get("items", [{}])[0]
    req("PATCH", f"/api/v2/albums/{album.get('id')}", {"title": f"Smith Best {sfx}", "label": "Own Label"})
    req("PUT", f"/api/v2/albums/{album.get('id')}/credits", {"credits": [{"contributor_id": smith.get("id"), "role": "artist"},
                                                                            {"name": f"Smith Partner {sfx}", "role": "artist"}]})
    st, fixed = req("GET", f"/api/v2/albums/{album.get('id')}")
    ctx.check("an album corrected: title, label, credits",
              st == 200 and fixed.get("title") == f"Smith Best {sfx}" and fixed.get("label") == "Own Label"
              and len([c for c in fixed.get("credits", []) if c["role"] == "artist"]) == 2, str(fixed)[:200])

    st, parts = req("POST", f"/api/v2/contributors/{smith.get('id')}/split", {"names": [f"Jo {sfx}", f"Hn {sfx}"]})
    ctx.check("a split answers the contributors it became", st == 200 and [p["name"] for p in parts] == [f"Jo {sfx}", f"Hn {sfx}"], str(parts)[:160])
    if st == 200:
        st, kept = req("POST", f"/api/v2/contributors/{parts[1]['id']}/merge", {"into": parts[0]["id"]})
        ctx.check("a merge answers the one that stays", st == 200 and kept.get("id") == parts[0]["id"])
    _mid, _memail, member = ctx.make_user("v2fix-member", PW)
    ctx.check("merging and splitting are a family admin's", all(
        ctx.request("POST", path, member, body)[0] in (403, 404) for path, body in [
            (f"/api/v2/contributors/{uuid.uuid4()}/merge", {"into": str(uuid.uuid4())}),
            (f"/api/v2/contributors/{uuid.uuid4()}/split", {"names": ["a", "b"]}),
            (f"/api/v2/albums/{uuid.uuid4()}/merge", {"into": str(uuid.uuid4())})]))
    shared = ctx.upload_track(tok, f"Shared {sfx}", artist=f"Share Artist {sfx}", album=f"Share Album {sfx}", visibility="family")
    expected = 403 if ctx.feature("one_family") else 404
    ctx.check(f"only a track's owner corrects its credits ({expected} for another account)", problem(
        ctx.request("PUT", f"/api/v2/tracks/{shared['id']}/credits", member, {"credits": []}), expected))


def user_state(ctx: Ctx, problem) -> None:
    """V4: favourites, ratings, progress with each device's last position, plays, bookmarks,
    and offline changes pushed in one batch."""
    import datetime as dt
    sfx = ctx.sfx
    uid, email, _tok = ctx.make_family_admin("v2state", PW)

    def sign_in(device: str) -> str:
        data, headers = _form({"grant_type": "password", "username": email, "password": PW, "device_name": device, "device_kind": "ios"})
        return json.loads(ctx.request("POST", "/api/v2/auth/token", body=data, headers=headers)[2])["access_token"]

    phone, car = sign_in("Phone"), sign_in("Car")

    def req(method: str, path: str, body=None, token: str = phone):
        st, _h, raw = ctx.request(method, path, token, body)
        return st, (json.loads(raw) if raw else None)

    track = ctx.upload_track(phone, f"State Song {sfx}", artist=f"State Band {sfx}", album=f"State Album {sfx}")
    tid = track["id"]

    st, _ = req("PUT", f"/api/v2/me/favourites/track/{tid}")
    st2, favs = req("GET", "/api/v2/me/favourites?kind=track")
    ctx.check("a favourite, listed", st == 204 and any(f["id"] == tid for f in favs["items"]))
    req("DELETE", f"/api/v2/me/favourites/track/{tid}")
    ctx.check("…and taken back", not any(f["id"] == tid for f in req("GET", "/api/v2/me/favourites?kind=track")[1]["items"]))
    ctx.check("a kind without favourites: 400 unsupported_kind",
              problem(ctx.request("PUT", f"/api/v2/me/favourites/episode/{tid}", phone), 400, "unsupported_kind"))
    _oid, _oe, other = ctx.make_user("v2state-other", PW)
    ctx.check("another account cannot favourite a private track", problem(ctx.request("PUT", f"/api/v2/me/favourites/track/{tid}", other), 404))

    req("PUT", f"/api/v2/me/ratings/track/{tid}", {"rating": 4})
    rated = req("GET", "/api/v2/me/ratings?kind=track")[1]["items"]
    ctx.check("a rating, listed", any(r["id"] == tid and r["rating"] == 4 for r in rated))
    req("PUT", f"/api/v2/me/ratings/track/{tid}", {"rating": 0})
    ctx.check("0 takes it away", not any(r["id"] == tid for r in req("GET", "/api/v2/me/ratings?kind=track")[1]["items"]))
    ctx.check("a rating of 6: 400", problem(ctx.request("PUT", f"/api/v2/me/ratings/track/{tid}", phone, {"rating": 6}), 400))

    st, saved = req("PUT", f"/api/v2/me/progress/track/{tid}", {"position_ms": 30000})
    ctx.check("progress saved", st == 200 and saved["applied"] and saved["progress"]["position_ms"] == 30000, str(saved)[:160])
    hour_ago = (dt.datetime.now(dt.timezone.utc) - dt.timedelta(hours=1)).isoformat()
    st, late = req("PUT", f"/api/v2/me/progress/track/{tid}", {"position_ms": 5000, "at": hour_ago}, car)
    ctx.check("an older position does not win", st == 200 and not late["applied"] and late["progress"]["position_ms"] == 30000, str(late)[:160])
    st, where = req("GET", f"/api/v2/me/progress/track/{tid}")
    devices = {d.get("device_name"): d for d in (where or {}).get("devices", [])}
    ctx.check("…but each device's last position is kept",
              devices.get("Car", {}).get("position_ms") == 5000 and devices.get("Phone", {}).get("current") is True
              and devices.get("Phone", {}).get("position_ms") == 30000, str(where)[:240])
    future = (dt.datetime.now(dt.timezone.utc) + dt.timedelta(days=365)).isoformat()
    req("PUT", f"/api/v2/me/progress/track/{tid}", {"position_ms": 40000, "at": future}, car)
    st, after = req("PUT", f"/api/v2/me/progress/track/{tid}", {"position_ms": 41000})
    ctx.check("a device clock in the future is clamped to the server's", after["applied"] and after["progress"]["position_ms"] == 41000)
    ctx.check("progress lists", any(p["id"] == tid for p in req("GET", "/api/v2/me/progress?kind=track")[1]["items"]))

    now = dt.datetime.now(dt.timezone.utc)
    play = {"client_id": f"play-{sfx}", "kind": "track", "id": tid, "started_at": (now - dt.timedelta(minutes=3)).isoformat(),
            "ended_at": now.isoformat(), "listened_ms": 180000, "device_kind": "ios"}
    st, results = req("POST", "/api/v2/me/plays", {"plays": [play, play, dict(play, client_id="x", id=str(uuid.uuid4()))]})
    ctx.check("plays: recorded once, the repeat a duplicate, an unknown item not_found",
              st == 200 and [r["status"] for r in results] == ["ok", "duplicate", "not_found"], str(results))

    st, mark = req("POST", "/api/v2/me/bookmarks", {"kind": "track", "item_id": tid, "position_ms": 12000, "label": "the bridge"})
    ctx.check("a bookmark", st == 201 and mark["position_ms"] == 12000 and mark["label"] == "the bridge", str(mark)[:160])
    st, moved = req("PATCH", f"/api/v2/me/bookmarks/{mark['id']}", {"label": None, "position_ms": 15000})
    ctx.check("…moved and its label cleared", st == 200 and moved["position_ms"] == 15000 and moved["label"] is None)
    listed = req("GET", f"/api/v2/me/bookmarks?kind=track&item_id={tid}")[1]["items"]
    ctx.check("…listed for its item", [b["id"] for b in listed] == [mark["id"]])
    ctx.check("another account's bookmark: 404", problem(ctx.request("DELETE", f"/api/v2/me/bookmarks/{mark['id']}", other), 404))
    ctx.check("…its own: 204", ctx.request("DELETE", f"/api/v2/me/bookmarks/{mark['id']}", phone)[0] == 204)

    st, pushed = req("POST", "/api/v2/sync/push", {"changes": [
        {"client_id": "a", "op": "favourite", "kind": "track", "id": tid},
        {"client_id": "b", "op": "rate", "kind": "track", "id": tid, "rating": 5},
        {"client_id": "c", "op": "progress", "kind": "track", "id": tid, "position_ms": 1, "at": hour_ago},
        {"client_id": f"d-{sfx}", "op": "play", "kind": "track", "id": tid, "started_at": play["started_at"],
         "ended_at": play["ended_at"], "listened_ms": 1000},
        {"client_id": "e", "op": "teleport", "kind": "track", "id": tid}]})
    ctx.check("offline changes answer one by one: ok, ok, stale, ok, unsupported_op",
              st == 200 and [r["status"] for r in pushed] == ["ok", "ok", "stale", "ok", "unsupported_op"], str(pushed))
    ctx.check("…and took effect", any(f["id"] == tid for f in req("GET", "/api/v2/me/favourites?kind=track")[1]["items"]))


def sync(ctx: Ctx, problem) -> None:
    """V5: the change feed and the snapshot."""
    import base64 as b64
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2sync", PW)
    _oid, _oe, other = ctx.make_user("v2sync-other", PW)

    def changes(cursor: str, token: str = tok) -> dict:
        st, _h, raw = ctx.request("GET", f"/api/v2/sync/changes?cursor={cursor}", token)
        return json.loads(raw) if st == 200 else {"status": st}

    st, _h, raw = ctx.request("GET", "/api/v2/sync/changes", tok)
    start = json.loads(raw) if st == 200 else {}
    ctx.check("no cursor: start from now, nothing yet", st == 200 and start.get("changes") == [] and bool(start.get("cursor")))
    other_start = json.loads(ctx.request("GET", "/api/v2/sync/changes", other)[2])["cursor"]

    track = ctx.upload_track(tok, f"Sync Song {sfx}", artist=f"Sync Band {sfx}", album=f"Sync Album {sfx}")
    ctx.request("PUT", f"/api/v2/me/favourites/track/{track['id']}", tok)
    seen, cursor = [], start.get("cursor", "")
    for _ in range(20):
        page = changes(cursor)
        seen += page.get("changes", [])
        cursor = page.get("cursor", cursor)
        if not page.get("has_more"):
            break
    by = {(c["kind"], c["id"]): c for c in seen}
    t = by.get(("track", track["id"]), {})
    ctx.check("a new track arrives with its record, and its album and artist with it",
              t.get("op") == "upsert" and t.get("record", {}).get("album_id") == track.get("album_id")
              and ("album", track.get("album_id")) in by and any(k == "contributor" for k, _ in by), str(list(by))[:200])
    ctx.check("so does the listener's favourite", by.get(("favourite.track", track["id"]), {}).get("op") == "upsert")
    page = changes(cursor)
    ctx.check("read again from the new cursor: nothing new", page.get("changes") == [], str(page)[:160])

    band = next((c for c in seen if c["kind"] == "contributor" and c.get("record", {}).get("name") == f"Sync Band {sfx}"), None)
    if band:
        ctx.request("PATCH", f"/api/v2/contributors/{band['id']}", tok, {"name": f"Sync Group {sfx}"})
        renamed = changes(cursor)
        k = [c for c in renamed.get("changes", []) if c["kind"] == "contributor" and c["id"] == band["id"]]
        ctx.check("a rename is one change, of the contributor", len(k) == 1 and k[0]["record"]["name"] == f"Sync Group {sfx}", str(renamed)[:200])
        cursor = renamed.get("cursor", cursor)
    ctx.call("DELETE", f"/api/v1/music/tracks/{track['id']}", tok, expect=(200, 204))
    gone = changes(cursor)
    ctx.check("a deleted track arrives as a delete", any(c["kind"] == "track" and c["id"] == track["id"] and c["op"] == "delete"
                                                         for c in gone.get("changes", [])), str(gone)[:200])
    theirs = changes(other_start, other)
    ctx.check("another member is sent none of a private track's records",
              not any(c["id"] == track["id"] and c["op"] == "upsert" for c in theirs.get("changes", [])), str(theirs)[:200])

    stale = b64.urlsafe_b64encode(json.dumps({"from": 1, "seq": 0, "next": 1, "at": 0}).encode()).decode().rstrip("=")
    ctx.check("a cursor older than the log: 410 sync_cursor_expired", problem(ctx.request("GET", f"/api/v2/sync/changes?cursor={stale}", tok), 410, "sync_cursor_expired"))
    ctx.check("a made-up cursor: 400 invalid_cursor", problem(ctx.request("GET", "/api/v2/sync/changes?cursor=nope", tok), 400, "invalid_cursor"))

    kept = ctx.upload_track(tok, f"Snap Song {sfx}", artist=f"Snap Band {sfx}", album=f"Snap Album {sfx}")
    ctx.request("PUT", f"/api/v2/me/favourites/track/{kept['id']}", tok)
    # Compressed, as apps ask for it: the stream must reach its `end` line through gzip.
    import gzip
    st, headers, raw = ctx.request("GET", "/api/v2/sync/snapshot?kinds=track,favourite", tok, headers={"Accept-Encoding": "gzip"})
    if headers.get("content-encoding") == "gzip":
        raw = gzip.decompress(raw)
    lines = [json.loads(l) for l in raw.decode().splitlines() if l.strip()]
    records = {(l["kind"], l["id"]) for l in lines if l.get("type") == "record"}
    ctx.check("a snapshot: begin, records, end with the cursor",
              st == 200 and headers.get("content-type", "").startswith("application/x-ndjson") and lines[0].get("type") == "begin"
              and lines[-1].get("type") == "end" and lines[0].get("cursor") == lines[-1].get("cursor")
              and ("track", kept["id"]) in records and ("favourite.track", kept["id"]) in records
              and ("track", track["id"]) not in records, f"{st} {len(lines)} lines")
    after = next((l for l in lines if l.get("type") == "record" and l["kind"] == "track"), None)
    if after:
        st, _h, rest = ctx.request("GET", f"/api/v2/sync/snapshot?kinds=track&after=track:{after['id']}&cursor={lines[0]['cursor']}", tok)
        resumed = [json.loads(l) for l in rest.decode().splitlines() if l.strip()]
        ctx.check("a snapshot resumes after the last line received",
                  st == 200 and all(r.get("id") != after["id"] for r in resumed) and resumed[-1].get("type") == "end")
    end = changes(lines[-1].get("cursor", ""))
    ctx.check("the snapshot's cursor reads the change log", "changes" in end)


def playback(ctx: Ctx, problem) -> None:
    """V6: playback-info decides and signs; the link plays without a header, once checked."""
    import urllib.error
    import urllib.request
    import math
    import struct
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2play", PW)
    # Real audio this time, which ffmpeg can decode: two seconds of a tone as WAV.
    rate, frames = 22050, 44100
    pcm = b"".join(struct.pack("<h", int(8000 * math.sin(2 * math.pi * 440 * i / rate))) for i in range(frames))
    wav = b"RIFF" + struct.pack("<I", 36 + len(pcm)) + b"WAVEfmt " + struct.pack("<IHHIIHH", 16, 1, 1, rate, rate * 2, 2, 16) \
        + b"data" + struct.pack("<I", len(pcm)) + pcm
    track = ctx.upload_track(tok, f"Play Song {sfx}", artist=f"Play Band {sfx}", album=f"Play Album {sfx}", content=wav)

    def info(caps: dict, token: str = tok):
        st, _h, raw = ctx.request("POST", "/api/v2/playback-info", token, {"kind": "track", "id": track["id"], "capabilities": caps})
        return st, (json.loads(raw) if raw else {})

    class NoRedirect(urllib.request.HTTPRedirectHandler):
        def redirect_request(self, *args, **kwargs):
            return None

    def fetch(url: str, follow: bool = True):
        url = url if url.startswith("http") else ctx.url(url)
        opener = urllib.request.build_opener() if follow else urllib.request.build_opener(NoRedirect())
        try:
            with opener.open(urllib.request.Request(url, headers={"Range": "bytes=0-4095"}), timeout=ctx.timeout) as r:
                return r.status, r.headers.get("Content-Type", ""), r.read(4096)
        except urllib.error.HTTPError as e:
            return e.code, e.headers.get("Content-Type", ""), e.read()

    st, direct = info({})
    ctx.check("a client that plays anything gets the original, signed",
              st == 200 and direct.get("decision") == "direct" and "/api/v2/stream?" in direct.get("url", "")
              and "token" not in direct.get("url", "").lower() and bool(direct.get("expires_at")), str(direct)[:200])
    code, _ct, _b = fetch(direct.get("url", ""), follow=False)
    ctx.check("the link redirects to storage, without any header", code in (302, 307), str(code))
    code, _ct, body = fetch(direct.get("url", ""))
    ctx.check("…and the bytes come", code in (200, 206) and len(body) > 0, str(code))
    url = direct.get("url", "")
    tampered = url.replace("&b=0&", "&b=320&")
    code, ctype, body = fetch(tampered, follow=False)
    ctx.check("a link changed by hand: 403 invalid_link", code == 403 and b"invalid_link" in body, f"{code} {body[:80]!r}")

    st, small = info({"direct": ["audio/flac"], "transcode": ["mp3"], "max_bitrate_kbps": 64})
    ctx.check("a client that cannot play the original gets a transcode",
              st == 200 and small.get("decision") == "transcode" and small.get("bitrate_kbps") == 64
              and "format_not_playable" in small.get("reasons", []), str(small)[:200])
    code, ctype, head = fetch(small.get("url", ""))
    ctx.check("…which streams MP3", code == 200 and ctype.startswith("audio/mpeg")
              and (head[:3] == b"ID3" or (len(head) > 1 and head[0] == 0xFF and (head[1] & 0xE0) == 0xE0)), f"{code} {ctype} {head[:8]!r}")
    st, none = info({"direct": ["audio/flac"], "transcode": ["opus"]})
    ctx.check("no format the server makes: the original, and why", none.get("decision") == "direct"
              and "no_transcode_target" in none.get("reasons", []), str(none)[:160])

    _oid, _oe, other = ctx.make_user("v2play-other", PW)
    st, _ = info({}, other)
    ctx.check("another account cannot get a link to a private track", st == 404)


def images(ctx: Ctx, problem) -> None:
    """V6: artwork by signed, cacheable link; the client picks the size."""
    import struct
    import urllib.error
    import urllib.request
    import zlib
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2img", PW)
    track = ctx.upload_track(tok, f"Cover Song {sfx}", artist=f"Cover Band {sfx}", album=f"Cover Album {sfx}")
    side = 300
    # A 300 px gradient as PNG: big enough that w=64 has to scale it.
    raw = b"".join(b"\x00" + b"".join(bytes((x % 256, y % 256, 128)) for x in range(side)) for y in range(side))
    def chunk(kind: bytes, data: bytes) -> bytes:
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)
    png = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", side, side, 8, 2, 0, 0, 0)) \
        + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b"")
    ctx.call("POST", f"/api/v1/music/tracks/{track['id']}/upload-cover", tok, multipart={"cover": ("cover.png", png)}, expect=(200, 201))
    st, _h, body = ctx.request("GET", f"/api/v2/tracks/{track['id']}", tok)
    image = json.loads(body).get("image") if st == 200 else None
    ctx.check("a track with a cover carries an image link, signed, with no token", bool(image) and "/api/v2/images/" in image
              and "&s=" in image and "token" not in image.lower(), str(image))
    st, _h, body = ctx.request("GET", f"/api/v2/albums/{track['album_id']}", tok)
    ctx.check("so does its album", st == 200 and json.loads(body).get("image") == image)

    def fetch(url: str):
        url = url if url.startswith("http") else ctx.url(url)
        try:
            with urllib.request.urlopen(urllib.request.Request(url), timeout=ctx.timeout) as r:
                return r.status, {k.lower(): v for k, v in r.headers.items()}, r.read()
        except urllib.error.HTTPError as e:
            return e.code, {k.lower(): v for k, v in e.headers.items()}, e.read()

    if image:
        st, headers, small = fetch(image + "&w=64")
        ctx.check("w=64: a small JPEG, cacheable forever", st == 200 and headers.get("content-type") == "image/jpeg"
                  and small[:2] == b"\xff\xd8" and "immutable" in headers.get("cache-control", "") and len(small) < len(png),
                  f"{st} {headers.get('content-type')} {len(small)}")
        st, _h, again = fetch(image + "&w=64")
        ctx.check("…the same bytes the second time", st == 200 and again == small)
        ctx.check("a size outside the set: 400 invalid_size", fetch(image + "&w=100")[0] == 400)
        ctx.check("a link changed by hand: 403", fetch(image.replace("&s=", "&s=x"))[0] == 403)


def household(ctx: Ctx, problem) -> None:
    """V7: the household in v2 shapes; sharing in two words."""
    sfx = ctx.sfx
    uid, _email, tok = ctx.make_family_admin("v2home", PW)
    mid, _me, member = ctx.make_user("v2home-member", PW)

    def req(method: str, path: str, body=None, token: str = tok):
        st, _h, raw = ctx.request(method, path, token, body)
        return st, (json.loads(raw) if raw else None)

    st, fam = req("GET", "/api/v2/family")
    ctx.check("the household, and the caller's place in it", st == 200 and fam["me"]["user_id"] == uid and fam["me"]["role"] == "admin", str(fam)[:160])
    st, members = req("GET", "/api/v2/family/members")
    ctx.check("its members, roles as admin or member", st == 200 and any(m["user_id"] == uid and m["role"] == "admin" for m in members)
              and all(m["role"] in ("admin", "member") for m in members))
    st, inv = req("POST", "/api/v2/family/invites", {"kind": "link", "role": "member", "max_uses": 2, "label": f"fridge {sfx}"})
    ctx.check("an invitation link", st == 201 and inv.get("code") and inv.get("role") == "member", str(inv)[:160])
    ctx.check("…listed, then withdrawn", any(i["id"] == inv.get("id") for i in req("GET", "/api/v2/family/invites")[1])
              and req("DELETE", f"/api/v2/family/invites/{inv.get('id')}")[0] == 204)
    ctx.check("a role that is not admin or member: 400", problem(ctx.request("POST", "/api/v2/family/invites", tok, {"kind": "link", "role": "owner"}), 400))
    st, storage = req("GET", "/api/v2/family/storage")
    ctx.check("the household's storage", st == 200 and "total_bytes" in storage)

    track = ctx.upload_track(tok, f"Shared Later {sfx}", artist=f"Home Band {sfx}", album=f"Home Album {sfx}")
    st, vis = req("PUT", f"/api/v2/tracks/{track['id']}/visibility", {"visibility": "family"})
    ctx.check("an owner shares a track with the household", st == 200 and vis == {"visibility": "family"})
    ctx.check("a third word: 400", problem(ctx.request("PUT", f"/api/v2/tracks/{track['id']}/visibility", tok, {"visibility": "public"}), 400))
    if ctx.feature("one_family"):
        ctx.check("…and a member sees it", req("GET", f"/api/v2/tracks/{track['id']}", token=member)[0] == 200)
        ctx.check("a member cannot rename the household", problem(ctx.request("PATCH", "/api/v2/family", member, {"name": "x"}), 403))
        ctx.check("…nor share someone else's track", req("PUT", f"/api/v2/tracks/{track['id']}/visibility", {"visibility": "private"}, member)[0] == 404)
        st, _ = req("PATCH", f"/api/v2/family/members/{mid}", {"can_upload": False, "label": f"Kid {sfx}"})
        st2, members = req("GET", "/api/v2/family/members")
        me = next((m for m in members if m["user_id"] == mid), {})
        ctx.check("an admin changes a member's rights and label", st == 204 and me.get("can_upload") is False and me.get("label") == f"Kid {sfx}", str(me)[:160])
        req("PUT", f"/api/v2/tracks/{track['id']}/visibility", {"visibility": "private"})
        ctx.check("kept private again, the member no longer sees it", problem(ctx.request("GET", f"/api/v2/tracks/{track['id']}", member), 404))
    else:
        ctx.skip("members of one household", "each account has its own household on this server")


def live(ctx: Ctx, problem) -> None:
    """V8: a change made elsewhere arrives as an event."""
    import threading
    import time
    import urllib.request
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2live", PW)
    track = ctx.upload_track(tok, f"Live Song {sfx}", artist=f"Live Band {sfx}", album=f"Live Album {sfx}")

    def listen(out: list, last_id: str | None = None) -> None:
        headers = {"Authorization": f"Bearer {tok}", "Accept": "text/event-stream"}
        if last_id:
            headers["Last-Event-ID"] = last_id
        try:
            with urllib.request.urlopen(urllib.request.Request(ctx.url("/api/v2/events"), headers=headers), timeout=20) as r:
                out.append(("type", r.headers.get("Content-Type", "")))
                for raw in r:
                    line = raw.decode().strip()
                    if line.startswith("event:") or line.startswith("id:"):
                        out.append(tuple(line.split(":", 1)))
                    if line.startswith("id:"):
                        return
        except Exception as e:  # noqa: BLE001 — a timeout means no event came
            out.append(("error", str(e)))

    got: list = []
    t = threading.Thread(target=listen, args=(got,), daemon=True)
    t.start()
    time.sleep(1.5)
    ctx.request("PUT", f"/api/v2/me/favourites/track/{track['id']}", tok)
    t.join(15)
    events = dict((k.strip(), v.strip()) for k, v in got if k != "type")
    ctx.check("a change made elsewhere arrives as a library event, with an id",
              ("type", "text/event-stream") in [(k, v.split(";")[0]) for k, v in got] and events.get("event") == "library"
              and events.get("id", "").isdigit(), str(got)[:200])
    if events.get("id", "").isdigit():
        again: list = []
        listen(again, str(int(events["id"]) - 1))
        ctx.check("Last-Event-ID resumes after it, at once", dict((k.strip(), v.strip()) for k, v in again if k != "type").get("id") == events["id"], str(again)[:160])


def library(ctx: Ctx, problem) -> None:
    """V9: adding to the library by upload, with the size declared and Idempotency-Key honoured."""
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2add", PW)
    audio = ctx.audio()

    def upload(kind: str, name: str) -> dict:
        st, _h, raw = ctx.request("POST", "/api/v2/uploads", tok,
                                  {"kind": kind, "filename": name, "content_type": "audio/mpeg", "size_bytes": len(audio)})
        made = json.loads(raw) if st == 201 else {}
        if made:
            ctx.put_bytes(made["url"], audio, made["headers"]["Content-Type"])
        return made

    ctx.check("an upload must say how big it is", problem(ctx.request("POST", "/api/v2/uploads", tok,
              {"kind": "track", "filename": "x.mp3", "content_type": "audio/mpeg"}), 422))
    ctx.check("…and stay within the limit", problem(ctx.request("POST", "/api/v2/uploads", tok,
              {"kind": "track", "filename": "x.mp3", "content_type": "audio/mpeg", "size_bytes": 6 * 1024 ** 3}), 400))
    up = upload("track", f"new-{sfx}.mp3")
    ctx.check("an upload link", bool(up.get("upload_id")) and up.get("method") == "PUT" and bool(up.get("expires_at")), str(up)[:160])
    key = f"track-{sfx}"
    body = {"upload_id": up.get("upload_id"), "filename": f"new-{sfx}.mp3", "title": f"Uploaded {sfx}", "artist": f"Up Band {sfx}"}
    st, _h, first = ctx.request("POST", "/api/v2/tracks", tok, body, headers={"Idempotency-Key": key})
    track = json.loads(first) if st == 201 else {}
    ctx.check("a track from the upload, as a v2 record", st == 201 and track.get("title") == f"Uploaded {sfx}" and track.get("album_id"), str(track)[:160])
    st, headers, again = ctx.request("POST", "/api/v2/tracks", tok, body, headers={"Idempotency-Key": key})
    ctx.check("the same request with the same key answers the same, without a second track",
              st == 201 and again == first and headers.get("idempotent-replayed") == "true")
    ctx.check("the same key with another request: 422 idempotency_key_reused",
              problem(ctx.request("POST", "/api/v2/tracks", tok, dict(body, title="Other"), headers={"Idempotency-Key": key}), 422, "idempotency_key_reused"))
    st, _h, raw = ctx.request("GET", f"/api/v2/tracks?q=Uploaded%20{sfx}", tok)
    ctx.check("…one track, not two", st == 200 and len(json.loads(raw)["items"]) == 1)

    files = [upload("book_file", f"part{i}-{sfx}.mp3") for i in (1, 2)]
    st, _h, raw = ctx.request("POST", "/api/v2/books", tok, {
        "title": f"Uploaded Book {sfx}", "author": f"Up Writer {sfx}",
        "files": [{"upload_id": f.get("upload_id"), "relative_path": f"0{i + 1}.mp3"} for i, f in enumerate(files)]})
    book = json.loads(raw) if st == 201 else {}
    ctx.check("a book from uploaded files, its files in order", st == 201 and len(book.get("files", [])) == 2
              and [f["position"] for f in book["files"]] == sorted(f["position"] for f in book["files"]), str(book)[:200])
    disco = json.loads(ctx.request("GET", "/.well-known/own-audio")[2])
    ctx.check("discovery states the upload and batch limits", disco["limits"].get("upload_max_bytes", 0) > 0 and disco["limits"].get("batch_max") == 500)


def playlists(ctx: Ctx, problem) -> None:
    """V10: playlists — entries with their own ids, the owner's changes, what others see."""
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2lists", PW)
    _mid, _me, member = ctx.make_user("v2lists-member", PW)

    def req(method: str, path: str, body=None, token: str = tok):
        st, _h, raw = ctx.request(method, path, token, body)
        return st, (json.loads(raw) if raw else None)

    a, b, c = (ctx.upload_track(tok, f"List Song {n} {sfx}", artist=f"List Band {sfx}", album=f"List Album {sfx}")["id"] for n in "ABC")
    st, made = req("POST", "/api/v2/playlists", {"name": f"  Road {sfx} ", "track_ids": [a, b, a, str(uuid.uuid4())]})
    pid = (made or {}).get("id")
    ctx.check("a playlist made with tracks, a repeat kept, an unknown one left out",
              st == 201 and made["name"] == f"Road {sfx}" and made["track_count"] == 3 and made["visibility"] == "private", str(made)[:200])
    ctx.check("…listed, and found by id", any(p["id"] == pid for p in req("GET", "/api/v2/playlists?limit=500")[1]["items"])
              and req("GET", f"/api/v2/playlists/{pid}")[0] == 200)
    st, page = req("GET", f"/api/v2/playlists/{pid}/entries?include=albums,contributors")
    entries = (page or {}).get("items", [])
    ctx.check("entries in order, each with its own id and its track",
              st == 200 and [e["track"]["id"] for e in entries] == [a, b, a] and [e["position"] for e in entries] == [1, 2, 3]
              and len({e["id"] for e in entries}) == 3 and page.get("included", {}).get("albums"), str(page)[:200])
    st, two = req("GET", f"/api/v2/playlists/{pid}/entries?limit=2")
    st2, rest = req("GET", f"/api/v2/playlists/{pid}/entries?limit=2&cursor={two.get('next')}")
    ctx.check("entries page by cursor", [e["position"] for e in two["items"] + rest["items"]] == [1, 2, 3] and "next" not in rest)

    st, after = req("POST", f"/api/v2/playlists/{pid}/entries", {"track_ids": [c]})
    ctx.check("tracks added to the end", st == 200 and after["track_count"] == 4)
    ids = [e["id"] for e in req("GET", f"/api/v2/playlists/{pid}/entries")[1]["items"]]
    st, _ = req("PUT", f"/api/v2/playlists/{pid}/entries/order", {"entry_ids": [ids[3], ids[1]]})
    order = [e["id"] for e in req("GET", f"/api/v2/playlists/{pid}/entries")[1]["items"]]
    ctx.check("reordered: those named first, the rest after them as they were", st == 204 and order == [ids[3], ids[1], ids[0], ids[2]], str(order))
    ctx.check("an entry named twice, or not this playlist's: 400 invalid_entry",
              problem(ctx.request("PUT", f"/api/v2/playlists/{pid}/entries/order", tok, {"entry_ids": [ids[0], ids[0]]}), 400, "invalid_entry")
              and problem(ctx.request("PUT", f"/api/v2/playlists/{pid}/entries/order", tok, {"entry_ids": [str(uuid.uuid4())]}), 400, "invalid_entry"))
    st, _ = req("DELETE", f"/api/v2/playlists/{pid}/entries/{ids[0]}")
    ctx.check("one entry taken out; the same again is 404",
              st == 204 and req("GET", f"/api/v2/playlists/{pid}")[1]["track_count"] == 3
              and problem(ctx.request("DELETE", f"/api/v2/playlists/{pid}/entries/{ids[0]}", tok), 404))

    st, renamed = req("PATCH", f"/api/v2/playlists/{pid}", {"name": f"Drive {sfx}", "description": "for the car"})
    ctx.check("renamed and described", st == 200 and renamed["name"] == f"Drive {sfx}" and renamed["description"] == "for the car")
    st, cleared = req("PATCH", f"/api/v2/playlists/{pid}", {"description": None})
    ctx.check("null clears the description, the name stays", st == 200 and cleared["description"] is None and cleared["name"] == f"Drive {sfx}")
    ctx.check("an empty name, or a third visibility: 400",
              problem(ctx.request("PATCH", f"/api/v2/playlists/{pid}", tok, {"name": "  "}), 400)
              and problem(ctx.request("PATCH", f"/api/v2/playlists/{pid}", tok, {"visibility": "public"}), 400))

    ctx.check("a private playlist is not another member's to see", problem(ctx.request("GET", f"/api/v2/playlists/{pid}", member), 404))
    if ctx.feature("one_family"):
        req("PATCH", f"/api/v2/playlists/{pid}", {"visibility": "family"})
        st, seen = req("GET", f"/api/v2/playlists/{pid}", token=member)
        ctx.check("shared: a member sees it, counting only the tracks they can see",
                  st == 200 and seen["visibility"] == "family" and seen["track_count"] == 0, str(seen)[:200])
        ctx.check("…and its entries: none of the private tracks", req("GET", f"/api/v2/playlists/{pid}/entries", token=member)[1]["items"] == [])
        ctx.check("a member cannot rename, fill or reorder it",
                  problem(ctx.request("PATCH", f"/api/v2/playlists/{pid}", member, {"name": "mine"}), 403)
                  and problem(ctx.request("POST", f"/api/v2/playlists/{pid}/entries", member, {"track_ids": [a]}), 403)
                  and problem(ctx.request("PUT", f"/api/v2/playlists/{pid}/entries/order", member, {"entry_ids": []}), 403))
        ctx.check("…nor delete it", problem(ctx.request("DELETE", f"/api/v2/playlists/{pid}", member), 403))
    else:
        ctx.skip("a shared playlist seen by a member", "each account has its own household on this server")
    st, _ = req("DELETE", f"/api/v2/playlists/{pid}")
    ctx.check("deleted by its owner (to the trash), then gone", st == 204 and problem(ctx.request("GET", f"/api/v2/playlists/{pid}", tok), 404))
    ctx.check("a playlist without a name: 400", problem(ctx.request("POST", "/api/v2/playlists", tok, {"name": ""}), 400))


def finding(ctx: Ctx, problem) -> None:
    """V10: search, random tracks, the caller's plays, lyrics."""
    import datetime as dt
    sfx = ctx.sfx
    _uid, _email, tok = ctx.make_family_admin("v2find", PW)
    _oid, _oe, other = ctx.make_user("v2find-other", PW)

    def req(method: str, path: str, body=None, token: str = tok):
        st, _h, raw = ctx.request(method, path, token, body)
        return st, (json.loads(raw) if raw else None)

    band, album, genre = f"Seeker{sfx}", f"Findable{sfx}", f"Genre{sfx}"
    one, two, three = (ctx.upload_track(tok, f"Quest {n} {sfx}", artist=band, album=album, genre=genre)["id"] for n in ("One", "Two", "Three"))

    st, found = req("GET", f"/api/v2/search?q=quest%20one%20{sfx}&include=albums,contributors")
    ctx.check("search finds a track by its title, any case, with what it refers to",
              st == 200 and [t["id"] for t in found["tracks"]] == [one] and found.get("included", {}).get("albums"), str(found)[:200])
    st, by_band = req("GET", f"/api/v2/search?q={band}")
    ctx.check("…an artist by name, and the tracks credited to them",
              st == 200 and [k["name"] for k in by_band["artists"]] == [band] and {t["id"] for t in by_band["tracks"]} == {one, two, three}, str(by_band)[:200])
    st, by_album = req("GET", f"/api/v2/search?q={album}&types=albums")
    ctx.check("…an album, and only what was asked for",
              st == 200 and [x["title"] for x in by_album["albums"]] == [album] and by_album["tracks"] == [] and by_album["artists"] == [], str(by_album)[:200])
    ctx.check("% is a letter, not a wildcard", one not in [t["id"] for t in req("GET", "/api/v2/search?q=%25&types=tracks&limit=100")[1]["tracks"]])
    ctx.check("nothing private of someone else", req("GET", f"/api/v2/search?q={band}", token=other)[1] == {"artists": [], "albums": [], "tracks": []})
    ctx.check("an empty search, or an unknown type: 400",
              problem(ctx.request("GET", "/api/v2/search?q=%20", tok), 400, "invalid_query")
              and problem(ctx.request("GET", "/api/v2/search?q=x&types=books", tok), 400, "invalid_type"))

    genre_id = req("GET", f"/api/v2/tracks/{one}")[1]["genre_ids"][0]
    st, drawn = req("GET", f"/api/v2/tracks/random?genre_id={genre_id}&limit=10")
    ctx.check("random tracks of a genre: all of them, no next page",
              st == 200 and sorted(t["id"] for t in drawn["items"]) == sorted([one, two, three]) and "next" not in drawn, str(drawn)[:200])
    ctx.check("a limit of 0: 400", problem(ctx.request("GET", "/api/v2/tracks/random?limit=0", tok), 400, "invalid_limit"))

    now = dt.datetime.now(dt.timezone.utc)
    def play(tid: str, minutes_ago: int, n: int) -> dict:
        end = now - dt.timedelta(minutes=minutes_ago)
        return {"client_id": f"find-{sfx}-{tid}-{n}", "kind": "track", "id": tid, "started_at": (end - dt.timedelta(minutes=3)).isoformat(),
                "ended_at": end.isoformat(), "listened_ms": 180000, "ended_reason": "finished"}
    st, results = req("POST", "/api/v2/me/plays", {"plays": [play(one, 30, 1), play(one, 20, 2), play(two, 10, 1)]})
    ctx.check("plays with an ended_reason the database does not know are kept (the reason dropped)",
              st == 200 and [r["status"] for r in results] == ["ok", "ok", "ok"], str(results))
    most = req("GET", "/api/v2/me/played?kind=track&order=most")[1]["items"]
    recent = req("GET", "/api/v2/me/played?kind=track")[1]["items"]
    ctx.check("most played first, with the counts", [(p["id"], p["play_count"]) for p in most[:2]] == [(one, 2), (two, 1)], str(most)[:200])
    ctx.check("most recently played first", [p["id"] for p in recent[:2]] == [two, one], str(recent)[:200])
    albums = req("GET", "/api/v2/me/played?kind=album&order=most")[1]["items"]
    ctx.check("per album: its tracks' plays together", albums and albums[0]["play_count"] == 3 and albums[0]["kind"] == "album", str(albums)[:200])
    ctx.check("another account's plays are its own", req("GET", "/api/v2/me/played?kind=track", token=other)[1]["items"] == [])
    ctx.check("a kind without plays here: 400", problem(ctx.request("GET", "/api/v2/me/played?kind=book", tok), 400, "unsupported_kind"))

    ctx.call("PUT", f"/api/v1/music/tracks/{one}/lyrics", tok, {"lyrics": "[00:01.00]Hello\n[00:02.50]World"})
    st, words = req("GET", f"/api/v2/tracks/{one}/lyrics")
    ctx.check("synced lyrics, line by line with their times",
              st == 200 and words["synced"] and [(l.get("start_ms"), l["text"]) for l in words["lines"]] == [(1000, "Hello"), (2500, "World")], str(words)[:200])
    st, none = req("GET", f"/api/v2/tracks/{three}/lyrics")
    ctx.check("a track without lyrics: text null, no lines", st == 200 and none["text"] is None and none["lines"] == [], str(none)[:200])
    ctx.check("someone else's private track: 404", problem(ctx.request("GET", f"/api/v2/tracks/{one}/lyrics", other), 404))
