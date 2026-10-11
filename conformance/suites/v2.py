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
