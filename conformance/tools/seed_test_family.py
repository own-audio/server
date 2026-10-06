#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later

"""Seed a test family — one admin plus five members — on a running instance.

Stdlib only, like the other scripts here. Idempotent: accounts that already
exist are left alone, so re-running after a partial failure is safe.

Members are created through family invites rather than open self-registration:
an invite authorizes the registration *and* joins the new account to the
inviting family, which is the only way to end up with six people in one family
instead of six personal families.

    python3 conformance/tools/seed_test_family.py --base-url https://audio.example.com \
        --admin-email admin@example.com --admin-password '...' --family-name 'Test Family'

Prints a table of the accounts it ensured. Passwords are supplied by the
caller and never generated here, so nothing secret is invented behind your
back.
"""

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.request

MEMBERS = [
    ("anna@example.com", "Anna"),
    ("petr@example.com", "Petr"),
    ("eva@example.com", "Eva"),
    ("jan@example.com", "Jan"),
    ("lucie@example.com", "Lucie"),
]


class ApiError(RuntimeError):
    def __init__(self, method: str, path: str, status: int, body: str) -> None:
        self.status = status
        self.body = body
        super().__init__(f"{method} {path} -> {status}: {body[:300]}")


def call(base: str, method: str, path: str, body=None, token: str | None = None):
    url = f"{base}/api/v1{path}"
    data = json.dumps(body).encode() if body is not None else None
    # Cloudflare answers urllib's default User-Agent with a 403 "error code:
    # 1010" browser-signature block long before the request reaches the API.
    headers = {"Content-Type": "application/json", "User-Agent": "audio2-seed/1.0"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    req = urllib.request.Request(url, data=data, headers=headers, method=method)
    try:
        with urllib.request.urlopen(req) as r:
            raw = r.read().decode()
            return json.loads(raw) if raw else {}
    except urllib.error.HTTPError as e:
        raise ApiError(method, path, e.code, e.read().decode(errors="replace")) from None


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--base-url", required=True)
    ap.add_argument("--admin-email", required=True)
    ap.add_argument("--admin-password", required=True)
    ap.add_argument("--member-password", required=True)
    ap.add_argument("--display-name", default="Kornel")
    ap.add_argument("--family-name", default="Test Family")
    args = ap.parse_args()
    base = args.base_url.rstrip("/")

    status = call(base, "GET", "/setup/status")
    if not status.get("setup_complete"):
        call(base, "POST", "/setup/complete", {
            "email": args.admin_email,
            "password": args.admin_password,
            "display_name": args.display_name,
        })
        print(f"created first admin {args.admin_email}")
    else:
        print("setup already complete — using the existing admin")

    token = call(base, "POST", "/auth/login", {
        "email": args.admin_email, "password": args.admin_password,
    })["token"]

    call(base, "PUT", "/family", {"name": args.family_name}, token)
    print(f"family named {args.family_name!r}")

    existing = {m["email"].lower() for m in call(base, "GET", "/family", token=token)["members"]}
    rows = [(args.admin_email, args.display_name, "family_admin", "existing")]

    for email, name in MEMBERS:
        if email.lower() in existing:
            rows.append((email, name, "member", "existing"))
            continue
        invite = call(base, "POST", "/family/invites", {"email": email, "role": "member"}, token)
        call(base, "POST", "/auth/register", {
            "email": email,
            "password": args.member_password,
            "display_name": name,
            "invite_code": invite["code"],
        })
        rows.append((email, name, "member", "created"))

    family = call(base, "GET", "/family", token=token)
    print(f"\n{family['name']} — {len(family['members'])} members")
    for email, name, role, state in rows:
        print(f"  {email:24} {name:8} {role:13} {state}")

    if len(family["members"]) != len(MEMBERS) + 1:
        print("\nWARNING: unexpected member count", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
