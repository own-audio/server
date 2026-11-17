#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Compare docs/api/openapi.json and openapi-v2.json with the ones at the last release tag.

Within /api/v1 and /api/v2 changes are additive only (docs/API_COMPATIBILITY.md §4). v2 is
still marked draft (`2.<revision>-draft`) until the release that completes
docs/api-v2-completion-plan.md, but the iPhone apps ship on it, so it is checked the same way:
- an operation that existed at the tag must still exist;
- if the document changed at all, info.version (<major>.<revision>) must be higher.

    python3 scripts/check-api-contract.py            # against the newest v* tag
    python3 scripts/check-api-contract.py v1.0.0-beta.1
"""
import json, subprocess, sys

CONTRACTS = [
    ("docs/api/openapi.json", "v1", "API_REVISION in backend/src/http/server_info.rs"),
    ("docs/api/openapi-v2.json", "v2", "V2_REVISION in backend/src/v2/discovery.rs"),
]


def git(*args):
    return subprocess.run(["git", *args], capture_output=True, text=True)


def operations(doc):
    methods = {"get", "put", "post", "delete", "patch", "head", "options"}
    return {(path, m) for path, item in doc.get("paths", {}).items() for m in item if m in methods}


def revision(doc):
    # `2.1-draft` counts as 2.1: the status is not part of the revision.
    return tuple(int(x) for x in doc["info"]["version"].split("-")[0].split("."))


def check(tag, file, name, constant):
    old_raw = git("show", f"{tag}:{file}")
    if old_raw.returncode != 0:
        print(f"{tag} has no {file}; nothing to compare")
        return False
    old, new = json.loads(old_raw.stdout), json.load(open(file))

    failed = False
    for path, method in sorted(operations(old) - operations(new)):
        print(f"REMOVED since {tag}: {method.upper()} {path} ({name} is additive only)")
        failed = True

    def body(doc):
        return json.dumps({k: v for k, v in doc.items() if k != "info"}, sort_keys=True)

    if body(old) != body(new) and revision(new) <= revision(old):
        print(f"the {name} contract changed since {tag} but info.version is still {new['info']['version']}: "
              f"bump {constant} and regenerate")
        failed = True

    if not failed:
        added = len(operations(new) - operations(old))
        print(f"{name} contract OK against {tag}: {old['info']['version']} -> {new['info']['version']}, {added} operations added")
    return failed


def main():
    tag = sys.argv[1] if len(sys.argv) > 1 else git("describe", "--tags", "--abbrev=0", "--match", "v*").stdout.strip()
    if not tag:
        print("no release tag; nothing to compare")
        return 0
    failed = False
    for file, name, constant in CONTRACTS:
        failed |= check(tag, file, name, constant)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
