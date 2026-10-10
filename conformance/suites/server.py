# SPDX-License-Identifier: AGPL-3.0-or-later
"""Discovery and error conventions: GET /server, 404 vs 501, the features contract."""

import urllib.error
import urllib.request

NAME = "server"

REQUIRED_FEATURE_KEYS = [
    "registration_open", "auth", "uploads", "music_identify", "podcast_discovery",
    "file_sync", "library_folders", "subsonic", "mail", "billing", "payments",
    "narration", "translation",
]

# A hosted-only path per feature key, to prove the 501 convention.
HOSTED_PATHS = {
    "billing": "/api/v1/family/billing",
    "narration": "/api/v1/audiobook-gen/languages",
    "translation": "/api/v1/podcast-translate/recent",
}


def _preflight(ctx, path, origin):
    """The Access-Control-Allow-Origin a browser would see for a GET from `origin`, or None."""
    req = urllib.request.Request(ctx.url(path), method="OPTIONS", headers={
        "Origin": origin, "Access-Control-Request-Method": "GET",
        "Access-Control-Request-Headers": "authorization", "User-Agent": "own-audio-conformance",
    })
    try:
        with urllib.request.urlopen(req, timeout=ctx.timeout) as r:
            return r.headers.get("Access-Control-Allow-Origin")
    except urllib.error.HTTPError as e:
        return e.headers.get("Access-Control-Allow-Origin")


def run(ctx):
    if not ctx.discovery:
        ctx.skip("GET /server", "server predates discovery (404); baseline features assumed")
    else:
        info = ctx.server
        ctx.check("GET /server names the product", info.get("name") == "own.audio", str(info.get("name")))
        ctx.check("edition is foss or hosted", info.get("edition") in ("foss", "hosted"), str(info.get("edition")))
        ctx.check("version is a non-empty string", isinstance(info.get("version"), str) and bool(info["version"]))
        api = info.get("api") or {}
        ctx.check("api.version is 1", api.get("version") == 1, str(api.get("version")))
        ctx.check("api.revision is a positive integer", isinstance(api.get("revision"), int) and api["revision"] >= 1,
                  str(api.get("revision")))
        feats = info.get("features") or {}
        missing = [k for k in REQUIRED_FEATURE_KEYS if k not in feats]
        ctx.check("every documented features key is present", not missing, ", ".join(missing))
        bool_keys = [k for k in REQUIRED_FEATURE_KEYS if k not in ("auth", "uploads")]
        wrong = [k for k in bool_keys if not isinstance(feats.get(k), bool)]
        ctx.check("feature flags are booleans", not wrong, ", ".join(wrong))
        auth = feats.get("auth") or {}
        ctx.check("features.auth lists local, google, apple, microsoft",
                  all(isinstance(auth.get(p), bool) for p in ("local", "google", "apple", "microsoft")), str(auth))
        ctx.check("deprecations is a list", isinstance(info.get("deprecations"), list))
        if "demo" in info:
            demo = info["demo"] or {}
            ctx.check("demo carries an email and a password",
                      all(isinstance(demo.get(k), str) and demo[k] for k in ("email", "password")))
            # The published account must not be able to lock everyone else out. A wrong current
            # password makes the probe harmless on a server where the account isn't read-only.
            try:
                tok = ctx.call("POST", "/api/v1/auth/login", body={"email": demo["email"], "password": demo["password"]})["token"]
            except AssertionError:
                tok = None
            if tok:
                ctx.call("GET", "/api/v1/music/tracks?limit=1", tok)
                ctx.call("POST", "/api/v1/auth/password", tok, body={"current_password": "not-the-password", "new_password": "x" * 16},
                         expect=(403,))
                ctx.check("the demo account can read but not change its password", True)
        # Signed in without the right: 403, never 401 (a 401 makes clients refresh a good
        # token and sign out on the second one). Security hardening plan, C5.
        if ctx.admin_token:
            _uid, _email, user_tok = ctx.make_user("plain", "Plain-pass-123456")
            ctx.call("GET", "/api/v1/users", user_tok, expect=(403,))
            ctx.call("GET", "/api/v1/jobs", user_tok, expect=(403,))
            ctx.check("a user without the admin role gets 403 on admin routes", True)
        # Browsers may call /api only from the console's origins; /rest stays open.
        base = ctx.base_url.rstrip("/")
        allowed = _preflight(ctx, "/api/v1/server", base)
        refused = _preflight(ctx, "/api/v1/server", "https://evil.example")
        ctx.check("CORS on /api allows the server's own origin", allowed == base, str(allowed))
        ctx.check("CORS on /api refuses another origin", refused is None, str(refused))
        ctx.check("CORS on /rest allows any origin", _preflight(ctx, "/rest/ping.view", "https://evil.example") == "*")
        ctx.check("features.auth agrees with /auth/providers",
                  all(auth.get(p) == bool((ctx.providers.get(p) or {}).get("enabled")) for p in ("google", "apple", "microsoft")))

    if ctx.discovery and (ctx.server.get("api") or {}).get("revision", 0) >= 3:
        tracks = ctx.call("GET", "/api/v1/music/tracks?limit=5", ctx.admin_token)
        tracks = tracks if isinstance(tracks, list) else (tracks or {}).get("items", [])
        books = ctx.call("GET", "/api/v1/audiobooks", ctx.admin_token)
        books = books if isinstance(books, list) else (books or {}).get("items", [])
        items = tracks + books
        ctx.check("revision 3: tracks and books say where they come from",
                  all(i.get("source") in ("upload", "folder") and isinstance(i.get("read_only"), bool) for i in items),
                  f"{len(items)} items")
        folders = ctx.call("GET", "/api/v1/library/folders", ctx.admin_token)
        ctx.check("revision 3: GET /library/folders lists folders and the scan state",
                  isinstance(folders.get("folders"), list) and isinstance(folders.get("scanning"), bool))

    if ctx.discovery and (ctx.server.get("api") or {}).get("revision", 0) >= 5:
        features = ctx.server.get("features") or {}
        ctx.check("revision 5: features.podcast_search is a boolean, true wherever discovery is",
                  isinstance(features.get("podcast_search"), bool)
                  and (features["podcast_search"] or not features.get("podcast_discovery")))

    if ctx.discovery and (ctx.server.get("api") or {}).get("revision", 0) >= 4:
        ctx.check("revision 4: features.one_family is a boolean",
                  isinstance((ctx.server.get("features") or {}).get("one_family"), bool))

    if ctx.discovery and (ctx.server.get("api") or {}).get("revision", 0) >= 6:
        import uuid as _uuid
        info = ctx.server
        try:
            _uuid.UUID(str(info.get("id")))
            valid_id = True
        except ValueError:
            valid_id = False
        ctx.check("revision 6: id is a UUID", valid_id, str(info.get("id")))
        addresses = info.get("addresses")
        ctx.check("revision 6: addresses is a list of {url, scope}",
                  isinstance(addresses, list)
                  and all(isinstance(a.get("url"), str) and a.get("scope") in ("lan", "vpn", "public") for a in addresses),
                  str(addresses)[:120])
        urls = [a.get("url") for a in addresses or []]
        ctx.check("revision 6: no address twice", len(urls) == len(set(urls)), str(urls)[:120])
        import json as _json2, urllib.request as _req
        with _req.urlopen(_req.Request(ctx.base_url + "/api/v1/server", headers={"User-Agent": "own-audio-conformance"}), timeout=30) as r:
            again = _json2.loads(r.read())
        ctx.check("revision 6: the id stays the same between requests", again.get("id") == info.get("id"))

    # Browsers always ask for gzip; a streamed body that breaks under the
    # compression layer only shows up there (1.0.0-alpha.6, the track list).
    import gzip, json as _json, urllib.request
    for path in ("/api/v1/music/tracks", "/api/v1/music/albums", "/api/v1/audiobooks", "/api/v1/library/continue"):
        req = urllib.request.Request(ctx.base_url + path, headers={
            "Authorization": f"Bearer {ctx.admin_token}", "Accept-Encoding": "gzip", "User-Agent": "own-audio-conformance"})
        try:
            with urllib.request.urlopen(req, timeout=60) as r:
                raw = r.read()
                if r.headers.get("Content-Encoding") == "gzip":
                    raw = gzip.decompress(raw)
            _json.loads(raw)
            ok, detail = True, ""
        except Exception as e:  # noqa: BLE001 — any failure is the finding
            ok, detail = False, f"{type(e).__name__}: {e}"[:120]
        ctx.check(f"{path} works for a client that asks for gzip", ok, detail)

    # Error conventions hold on every server, discovery or not.
    body = ctx.call("GET", "/api/v1/definitely-not-a-route", ctx.admin_token, expect=(404,), raw=True)
    ctx.check("unknown route is 404 not_found", b'"not_found"' in body, body[:80].decode(errors="replace"))

    for feature, path in HOSTED_PATHS.items():
        status = ctx.status_of("GET", path, ctx.admin_token)
        if ctx.feature(feature):
            ctx.check(f"{feature}: offered, so {path} is not 501", status != 501, str(status))
        elif ctx.discovery:
            got = ctx.call("GET", path, ctx.admin_token, expect=(501,), raw=True)
            ctx.check(f"{feature}: not offered, so {path} is 501 feature_unavailable",
                      b'"feature_unavailable"' in got and feature.encode() in got, got[:120].decode(errors="replace"))
        else:
            ctx.skip(f"{feature}: {path}", "pre-discovery server")

    # Wrong passwords in a row lock the email for a growing while (security hardening plan
    # §5.1): 429 `account_locked` with Retry-After, for an email with an account and for one
    # without alike. The per-IP login limit answers 429 `rate_limited` and may come first on a
    # busy run; that is not the finding, so it ends the check instead of failing it.
    uid, email, _tok = ctx.make_user("lock", "LockTest12345!")
    def _lock_outcome(who: str) -> str | None:
        for _ in range(8):
            status = ctx.status_of("POST", "/api/v1/auth/login", body={"email": who, "password": "wrong-password-1"})
            if status == 429:
                got = ctx.call("POST", "/api/v1/auth/login", body={"email": who, "password": "wrong-password-1"},
                               expect=(429,), raw=True)
                return "locked" if b'"account_locked"' in got else "rate_limited"
            if status != 401:
                return f"unexpected {status}"
        return "never"
    outcome = _lock_outcome(email)
    if outcome == "rate_limited":
        ctx.skip("account lock after wrong passwords", "the per-IP login limit answered first")
    else:
        ctx.check("an email is locked after a run of wrong passwords (429 account_locked)", outcome == "locked", outcome)
        status = ctx.status_of("POST", "/api/v1/auth/login", body={"email": email, "password": "LockTest12345!"})
        ctx.check("the right password is refused too while the lock lasts", status == 429, str(status))
        outcome = _lock_outcome(f"nobody-{ctx.sfx}@example.com")
        ctx.check("an email without an account locks the same way", outcome in ("locked", "rate_limited"), outcome)

    # "Forgot password" never says whether an email exists: 200 for an unknown email and for
    # the admin's alike (the mail itself is best effort and not observed here). A made-up
    # link is refused with 400, and a short password is refused before the link is looked at.
    for who in (f"nobody-{ctx.sfx}@example.com", ctx.admin_email):
        ctx.call("POST", "/api/v1/auth/password/forgot", body={"email": who}, expect=(200,))
    ctx.check("password/forgot answers 200 for unknown and known emails alike", True)
    ctx.call("POST", "/api/v1/auth/password/reset", body={"token": "0" * 64, "password": "LongEnough123!"}, expect=(400,))
    ctx.call("POST", "/api/v1/auth/password/reset", body={"token": "0" * 64, "password": "short"}, expect=(400,))
    ctx.check("password/reset refuses a made-up link and a short password with 400", True)
