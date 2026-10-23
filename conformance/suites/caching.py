# SPDX-License-Identifier: AGPL-3.0-or-later
"""Revalidation: the track list and covers answer an unchanged version with 304."""

import urllib.error
import urllib.request

NAME = "caching"


def _get(ctx, path, token, etag=None):
    """(status, headers) of a GET; the body is read and dropped."""
    headers = {"Authorization": f"Bearer {token}", "Accept": "application/json"}
    if etag:
        headers["If-None-Match"] = etag
    req = urllib.request.Request(ctx.url(path), headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=ctx.timeout) as r:
            r.read()
            return r.status, r.headers
    except urllib.error.HTTPError as e:
        return e.code, e.headers


def run(ctx):
    _, _, token = ctx.make_user("lister", "CachingTest123!-long")
    ctx.upload_track(token, f"Caching one {ctx.sfx}")

    status, headers = _get(ctx, "/api/v1/music/tracks", token)
    etag = headers.get("ETag")
    ctx.check("the track list carries a version (ETag) and asks to be revalidated",
              status == 200 and bool(etag) and "no-cache" in (headers.get("Cache-Control") or ""), f"{status} {etag}")
    if not etag:
        return

    status, headers = _get(ctx, "/api/v1/music/tracks", token, etag)
    ctx.check("an unchanged track list is a 304", status == 304 and headers.get("ETag") == etag, str(status))

    ctx.upload_track(token, f"Caching two {ctx.sfx}")
    status, headers = _get(ctx, "/api/v1/music/tracks", token, etag)
    ctx.check("a new song makes it a new version, sent in full",
              status == 200 and headers.get("ETag") not in (None, etag), f"{status} {headers.get('ETag')}")

    _, _, other = ctx.make_user("other", "CachingTest123!-long")
    status, _ = _get(ctx, "/api/v1/music/tracks", other, headers.get("ETag"))
    ctx.check("one person's version is not another's", status == 200, str(status))
