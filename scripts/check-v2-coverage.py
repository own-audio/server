#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Check docs/api-v1-v2-coverage.md against the two OpenAPI documents.

The coverage table is the one place that says where each /api/v1 function lives in /api/v2
(docs/api-v2-completion-plan.md §4.3). This keeps it honest:

- every operation of docs/api/openapi.json has a row, and every row names one (the hosted
  edition's rows, in the section of that name, stand for routes this repository does not have);
- every v2 operation a row names in backticks (`GET /me`) is in docs/api/openapi-v2.json, and
  every field it names (`Member.image`) is a property of that schema (the hosted section's v2
  routes are checked only with --hosted-openapi);
- a row `not carried over` says why;
- the counts table matches the rows.

Report mode (the default) then lists the rows still `missing` or `partly` and exits 0 unless the
table itself is wrong; --strict also fails on any of those (CI from the release that makes v2
stable).

    python3 scripts/check-v2-coverage.py
    python3 scripts/check-v2-coverage.py --strict
    python3 scripts/check-v2-coverage.py --hosted-openapi ../audio2/docs/api/openapi-v2-hosted.json
"""
import argparse
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parents[1]
DOC = ROOT / "docs" / "api-v1-v2-coverage.md"
V1 = ROOT / "docs" / "api" / "openapi.json"
V2 = ROOT / "docs" / "api" / "openapi-v2.json"
METHODS = ("get", "post", "put", "patch", "delete")
STATUSES = ("covered", "partly", "missing", "not carried over")
COUNT_NAMES = {"covered": "covered", "partly covered": "partly", "missing": "missing", "not carried over": "not carried over"}
HOSTED_SECTION = "Hosted edition"
OP = re.compile(r"^(GET|POST|PUT|PATCH|DELETE|HEAD) (/\S*)$")
FIELD = re.compile(r"^([A-Z]\w*)\.(\w+)$")


def shape(path: str) -> str:
    """A path with its parameters unnamed and its query dropped, so `{id}` and `{user_id}` agree."""
    return re.sub(r"\{[^}]*\}", "{}", path.split("?")[0]).rstrip("/") or "/"


def operations(doc: dict, prefix: str) -> set[tuple[str, str]]:
    out = set()
    for path, item in doc["paths"].items():
        if not path.startswith(prefix):
            continue
        for method in item:
            if method in METHODS:
                out.add((method.upper(), shape(path[len(prefix):])))
    return out


def names(op: tuple[str, str], ops: set[tuple[str, str]]) -> bool:
    """Whether `op` is one of `ops`; a row may fill a parameter in (`/me/progress/book/{id}`)."""
    method, path = op
    want = path.split("/")
    for m, p in ops:
        have = p.split("/")
        if m == method and len(have) == len(want) and all(h == w or h == "{}" for h, w in zip(have, want)):
            return True
    return False


def schemas(doc: dict) -> dict[str, set[str]]:
    return {name: set((s.get("properties") or {}).keys()) for name, s in doc.get("components", {}).get("schemas", {}).items()}


def rows(text: str):
    """(section, line number, cells) for every row of the operation tables."""
    section = ""
    for n, line in enumerate(text.splitlines(), 1):
        if line.startswith("## "):
            section = line[3:].strip()
        if not line.startswith("| `"):
            continue
        cells = [c.strip() for c in line.strip().strip("|").split("|")]
        if len(cells) >= 4:
            yield section, n, cells


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--strict", action="store_true", help="fail on any row still missing or partly covered")
    ap.add_argument("--hosted-openapi", type=pathlib.Path, help="the hosted edition's v2 document, to check its rows too")
    args = ap.parse_args()

    text = DOC.read_text()
    v1_doc, v2_doc = json.loads(V1.read_text()), json.loads(V2.read_text())
    v1_ops = operations(v1_doc, "/api/v1")
    v2_ops = operations(v2_doc, "/api/v2") | {("GET", "/.well-known/own-audio")}
    v2_schemas = schemas(v2_doc)
    if args.hosted_openapi:
        hosted = json.loads(args.hosted_openapi.read_text())
        v2_ops |= operations(hosted, "/api/v2")
        for name, props in schemas(hosted).items():
            v2_schemas.setdefault(name, set()).update(props)

    errors: list[str] = []
    by_status: dict[str, list[str]] = {s: [] for s in STATUSES}
    named: set[tuple[str, str]] = set()
    for section, n, cells in rows(text):
        v1_cell, v2_cell, status, note = cells[0], cells[1], cells[2], cells[3]
        hosted = section.startswith(HOSTED_SECTION)
        m = OP.match(v1_cell.strip("`"))
        if not m:
            errors.append(f"line {n}: the first column is not one v1 operation: {v1_cell}")
            continue
        op = (m.group(1), shape(m.group(2)))
        if not hosted:
            if op not in v1_ops:
                errors.append(f"line {n}: {v1_cell} is not an operation of docs/api/openapi.json")
            if op in named:
                errors.append(f"line {n}: {v1_cell} has a row already")
            named.add(op)
        if status not in STATUSES:
            errors.append(f"line {n}: status `{status}` is not one of {', '.join(STATUSES)}")
            continue
        by_status[status].append(f"{v1_cell} — {note}" if note else v1_cell)
        if status == "not carried over" and not note:
            errors.append(f"line {n}: {v1_cell} is not carried over without a reason")
        if hosted and not args.hosted_openapi:
            continue
        for ref in re.findall(r"`([^`]+)`", v2_cell):
            om, fm = OP.match(ref), FIELD.match(ref)
            if om and not names((om.group(1), shape(om.group(2))), v2_ops):
                errors.append(f"line {n}: {ref} is not an operation of docs/api/openapi-v2.json")
            elif fm and fm.group(2) not in v2_schemas.get(fm.group(1), set()):
                errors.append(f"line {n}: {ref} is not a property of the v2 schema {fm.group(1)}")

    for method, path in sorted(v1_ops - named):
        errors.append(f"no row for {method} {path}")

    counted = {}
    for line in text.splitlines():
        m = re.match(r"^\| (covered|partly covered|missing|not carried over) \| (\d+) \|$", line.strip())
        if m:
            counted[COUNT_NAMES[m.group(1)]] = int(m.group(2))
        m = re.match(r"^\| \*\*total\*\* \| \*\*(\d+)\*\* \|$", line.strip())
        if m:
            counted["total"] = int(m.group(1))
    actual = {s: len(v) for s, v in by_status.items()}
    actual["total"] = sum(actual.values())
    for key, value in actual.items():
        if counted.get(key) != value:
            errors.append(f"the counts table says {key} {counted.get(key)}, the rows say {value}")

    open_rows = [("partly", r) for r in by_status["partly"]] + [("missing", r) for r in by_status["missing"]]
    print(f"{actual['total']} v1 operations: {actual['covered']} covered, {actual['partly']} partly, "
          f"{actual['missing']} missing, {actual['not carried over']} not carried over")
    for status, row in open_rows:
        print(f"  {status:7} {row}")
    for e in errors:
        print(f"ERROR {e}")
    if errors:
        return 1
    if args.strict and open_rows:
        print(f"--strict: {len(open_rows)} rows are not covered yet")
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
