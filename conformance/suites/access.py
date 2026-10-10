# SPDX-License-Identifier: AGPL-3.0-or-later
"""Deny by default (security hardening plan §5.3): every operation the OpenAPI document marks
as bearer-protected refuses an anonymous caller with 401, and the instance-admin routes refuse a
plain member with 403 — not 404, not a validation error, and never a body.

Walks `docs/api/openapi.json`, the document the server's `#[utoipa::path]` annotations produce
and CI keeps current, so a route added without thinking about who may call it is caught the
first time this runs. Path parameters are filled with ids that exist nowhere; the answer has to
be about the caller, not the id.
"""
from __future__ import annotations

import json
import pathlib
import uuid

from core import Ctx

NAME = "access"
PW = "AccessTest12345!"

OPENAPI = pathlib.Path(__file__).resolve().parents[2] / "docs" / "api" / "openapi.json"

# What a path parameter is filled with. Anything not listed is a UUID.
PARAM_VALUES = {
    "code": "ZZZZZZZZ",
    "user_code": "ZZZZ-ZZZZ",
    "kind": "music",
    "role": "user",
    "batch": "1",
}

# Operations a plain member may not call: everything under the instance-admin prefixes, and
# the two that read or create accounts across families.
ADMIN_PREFIXES = ("/api/v1/admin/",)
ADMIN_OPS = {("GET", "/api/v1/users"), ("POST", "/api/v1/auth/admin-create-user")}


def _fill(path: str) -> str:
    out = path
    for name in _params(path):
        out = out.replace("{" + name + "}", PARAM_VALUES.get(name, str(uuid.uuid4())))
    return out


def _params(path: str) -> list[str]:
    return [seg[1:-1] for seg in path.split("/") if seg.startswith("{") and seg.endswith("}")]


def _ops(doc: dict) -> list[tuple[str, str, dict]]:
    return [(m.upper(), p, o) for p, methods in doc["paths"].items() for m, o in methods.items()
            if m in ("get", "post", "put", "patch", "delete")]


def _protected(op: dict) -> bool:
    return any(s for s in op.get("security", []))


def run(ctx: Ctx) -> None:
    if not OPENAPI.exists():
        ctx.skip("suite", f"{OPENAPI} is not here (run from the server repository)")
        return
    doc = json.loads(OPENAPI.read_text())
    ops = _ops(doc)

    # 1. Anonymous callers: 401 on every protected operation.
    leaks = []
    for method, path, op in ops:
        if not _protected(op):
            continue
        body = {} if method in ("POST", "PUT", "PATCH") else None
        status = ctx.status_of(method, _fill(path), None, body)
        if status != 401:
            leaks.append(f"{method} {path} → {status}")
    ctx.check(f"every protected operation answers 401 without a token ({sum(_protected(o) for _, _, o in ops)} checked)",
              not leaks, "; ".join(leaks[:12]))

    # 2. A plain member: 403 on the instance-admin operations, whatever id is asked about.
    _uid, _email, tok = ctx.make_user("plain", PW)
    wrong = []
    for method, path, op in ops:
        if not (path.startswith(ADMIN_PREFIXES) or (method, path) in ADMIN_OPS):
            continue
        body = {} if method in ("POST", "PUT", "PATCH") else None
        status = ctx.status_of(method, _fill(path), tok, body)
        if status != 403:
            wrong.append(f"{method} {path} → {status}")
    ctx.check("every instance-admin operation answers 403 to a plain member", not wrong, "; ".join(wrong[:12]))

    # 3. No operation is left undeclared: a route without a security entry is a route
    # nobody decided about.
    undeclared = [f"{m} {p}" for m, p, o in ops if "security" not in o]
    ctx.check("every operation declares whether it needs a token", not undeclared, "; ".join(undeclared[:12]))
