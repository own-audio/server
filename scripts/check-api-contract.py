#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Compare docs/api/openapi.json with the one at the last release tag.

Within /api/v1 changes are additive only (docs/API_COMPATIBILITY.md §4):
- an operation that existed at the tag must still exist;
- if the document changed at all, info.version (1.<revision>) must be higher.

    python3 scripts/check-api-contract.py            # against the newest v* tag
    python3 scripts/check-api-contract.py v1.0.0-beta.1
"""
import json, subprocess, sys

FILE = "docs/api/openapi.json"


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True)


def operations(doc):
    methods = {"get", "put", "post", "delete", "patch", "head", "options"}
    return {(path, m) for path, item in doc.get("paths", {}).items() for m in item if m in methods}


def revision(doc):
    return tuple(int(x) for x in doc["info"]["version"].split("."))


def main():
    tag = sys.argv[1] if len(sys.argv) > 1 else git("describe", "--tags", "--abbrev=0", "--match", "v*").stdout.strip()
    if not tag:
        print("no release tag; nothing to compare")
        return 0
    old_raw = git("show", f"{tag}:{FILE}")
    if old_raw.returncode != 0:
        print(f"{tag} has no {FILE}; nothing to compare")
        return 0
    old, new = json.loads(old_raw.stdout), json.load(open(FILE))

    failed = False
    removed = sorted(operations(old) - operations(new))
    for path, method in removed:
        print(f"REMOVED since {tag}: {method.upper()} {path} (v1 is additive only)")
        failed = True

    def body(doc):
        return json.dumps({k: v for k, v in doc.items() if k != "info"}, sort_keys=True)

    if body(old) != body(new) and revision(new) <= revision(old):
        print(f"the contract changed since {tag} but info.version is still {new['info']['version']}: "
              "bump API_REVISION in backend/src/http/server_info.rs and regenerate")
        failed = True

    if not failed:
        added = len(operations(new) - operations(old))
        print(f"contract OK against {tag}: revision {old['info']['version']} -> {new['info']['version']}, {added} operations added")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
