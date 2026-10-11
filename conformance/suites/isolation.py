# SPDX-License-Identifier: AGPL-3.0-or-later
"""Object-level access (security hardening plan §5.3): family B tries every id of family A.

Family A keeps a private track, a playlist and its members; the admin of family B then calls
every operation in `docs/api/openapi.json` whose path carries one of those ids. None may answer
2xx. 404 and 403 are both fine — "not yours" and "does not exist" are deliberately not told
apart — and so is a validation error on an empty body, since the check is about the id. What
this catches is a route that looks the id up without asking whose it is.
"""
from __future__ import annotations

import json
import pathlib

from core import Ctx

NAME = "isolation"
PW = "IsolationTest123!"

OPENAPI = pathlib.Path(__file__).resolve().parents[2] / "docs" / "api" / "openapi.json"

# Path prefix → which of family A's ids fills its parameter.
PREFIXES = {
    "/api/v1/music/tracks/{id}": "track",
    "/api/v1/music/playlists/{id}": "playlist",
    "/api/v1/family/members/{user_id}": "member",
    "/api/v1/users/{id}": "member",
}


def _ops(doc: dict):
    for path, methods in doc["paths"].items():
        for method, op in methods.items():
            if method in ("get", "post", "put", "patch", "delete"):
                yield method.upper(), path, op


def run(ctx: Ctx) -> None:
    if not OPENAPI.exists():
        ctx.skip("suite", f"{OPENAPI} is not here (run from the server repository)")
        return
    if ctx.feature("one_family"):
        ctx.skip("suite", "one family per install: there is no second family to try from")
        return
    doc = json.loads(OPENAPI.read_text())

    a_id, _a_email, a_tok = ctx.make_family_admin("a-owner", PW)
    _b_id, _b_email, b_tok = ctx.make_family_admin("b-owner", PW)

    track = ctx.upload_track(a_tok, f"A private {ctx.sfx}")
    playlist = ctx.call("POST", "/api/v1/music/playlists", a_tok, {"name": f"A list {ctx.sfx}"})
    ids = {"track": track["id"], "playlist": playlist["id"], "member": a_id}

    reached = []
    checked = 0
    for method, path, _op in _ops(doc):
        for prefix, kind in PREFIXES.items():
            if not path.startswith(prefix) or kind not in ids:
                continue
            filled = path.replace("{id}", ids[kind]).replace("{user_id}", ids[kind])
            # Routes that still carry another parameter cannot be filled meaningfully.
            if "{" in filled:
                continue
            body = {} if method in ("POST", "PUT", "PATCH") else None
            status = ctx.status_of(method, filled, b_tok, body)
            checked += 1
            if 200 <= status < 300:
                reached.append(f"{method} {path} → {status}")
            break
    ctx.check(f"no operation on family A's track, playlist or members answers 2xx to family B ({checked} checked)",
              not reached, "; ".join(reached[:12]))

    # The same from the other side of the fence: A's track is not in B's library or search.
    lib = ctx.call("GET", "/api/v1/music/tracks", b_tok)
    items = lib if isinstance(lib, list) else lib.get("items", lib.get("tracks", []))
    ctx.check("family A's track is not listed in family B's library", all(t.get("id") != track["id"] for t in items))
