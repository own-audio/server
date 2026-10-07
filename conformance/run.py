#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Black-box conformance suite for the own.audio server API.

    python3 conformance/run.py --base-url http://127.0.0.1:8080 \
        --admin-email admin@audio2.local --admin-password admin --compose-dir ../audio2

Runs every suite in `suites/` against the server at --base-url and exits 1 on
any failed check. Suites that need the database (--compose-dir) or need the
server to reach this machine (a loopback --base-url) are skipped, not failed,
when that is not available. See README.md.
"""
from __future__ import annotations

import argparse
import importlib
import json
import pathlib
import sys
import time
import traceback

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from core import ApiError, Ctx, Skip  # noqa: E402

SUITES_DIR = pathlib.Path(__file__).resolve().parent / "suites"
# Order matters a little: smoke first (it validates login), the rest independent.
ORDER = ["server", "smoke", "families", "join", "mobile", "stats", "trash", "filesync", "library"]


def discover_suites() -> list[str]:
    names = [p.stem for p in SUITES_DIR.glob("*.py") if not p.stem.startswith("_")]
    return sorted(names, key=lambda n: (ORDER.index(n) if n in ORDER else len(ORDER), n))


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--base-url", default="http://127.0.0.1:8080")
    ap.add_argument("--admin-email", default="admin@audio2.local")
    ap.add_argument("--admin-password", default="admin")
    ap.add_argument("--compose-dir", type=pathlib.Path, default=None,
                    help="directory with the server's docker-compose.yml; enables SQL-backed checks")
    ap.add_argument("--database", default="audio2", help="database name for the SQL-backed checks (default audio2)")
    ap.add_argument("--database-user", default="audio2", help="postgres user for the SQL-backed checks (default audio2)")
    ap.add_argument("--only", default="", help="comma-separated suite names to run")
    ap.add_argument("--skip", default="", help="comma-separated suite names to skip")
    ap.add_argument("--require-media", action="store_true", help="fail instead of skip when no streamable media exists")
    ap.add_argument("--report", type=pathlib.Path, default=None, help="write a JSON report here")
    ap.add_argument("--list", action="store_true", help="list suites and exit")
    ap.add_argument("-q", "--quiet", action="store_true")
    args = ap.parse_args()

    suites = discover_suites()
    if args.list:
        print("\n".join(suites))
        return 0
    only = {s for s in args.only.split(",") if s}
    skip = {s for s in args.skip.split(",") if s}
    if only:
        unknown = only - set(suites)
        if unknown:
            print(f"unknown suite(s): {', '.join(sorted(unknown))}", file=sys.stderr)
            return 2
        suites = [s for s in suites if s in only]
    suites = [s for s in suites if s not in skip]

    ctx = Ctx(
        base_url=args.base_url.rstrip("/"), admin_email=args.admin_email, admin_password=args.admin_password,
        compose_dir=args.compose_dir.resolve() if args.compose_dir else None,
        database=args.database, database_user=args.database_user,
        require_media=args.require_media, verbose=not args.quiet,
    )
    started = time.time()
    print(f"server   {ctx.base_url}")
    try:
        ctx.discover()
        ctx.login_admin()
    except Exception as e:  # noqa: BLE001
        print(f"FAIL     cannot talk to the server: {e}")
        return 1
    print(f"edition  {ctx.server.get('edition', 'pre-discovery')}  version {ctx.server.get('version', '?')}  "
          f"api {ctx.server.get('api', {}).get('version', 1)} rev {ctx.server.get('api', {}).get('revision', '-')}")
    on = sorted(k for k, v in ctx.features.items() if v is True)
    print(f"features {' '.join(on) or '-'}")
    print(f"database {'via ' + str(ctx.compose_dir) if ctx.db_available else 'not available (SQL checks skipped)'}")

    suite_outcomes: dict[str, str] = {}
    for name in suites:
        mod = importlib.import_module(f"suites.{name}")
        requires = set(getattr(mod, "REQUIRES", ()))
        ctx.suite = name
        print(f"\n[{name}] {mod.__doc__.strip().splitlines()[0] if mod.__doc__ else ''}")
        if "db" in requires and not ctx.db_available:
            ctx.skip("suite", "needs --compose-dir"); suite_outcomes[name] = "skip"; continue
        if "local" in requires and not (ctx.is_local and ctx.db_available):
            ctx.skip("suite", "needs a loopback --base-url and --compose-dir (server must reach this machine)")
            suite_outcomes[name] = "skip"; continue
        before = len(ctx.results)
        try:
            mod.run(ctx)
            outcome = "ok"
        except Skip as s:
            ctx.skip("rest of suite", str(s)); outcome = "skip"
        except (AssertionError, ApiError) as e:
            ctx.fail("suite aborted", str(e)); outcome = "FAIL"
        except Exception as e:  # noqa: BLE001
            ctx.fail("suite crashed", f"{type(e).__name__}: {e}")
            if not args.quiet:
                traceback.print_exc()
            outcome = "FAIL"
        finally:
            ctx.run_cleanups()
        if any(r.status == "FAIL" for r in ctx.results[before:]):
            outcome = "FAIL"
        suite_outcomes[name] = outcome

    n_ok = sum(r.status == "ok" for r in ctx.results)
    n_fail = sum(r.status == "FAIL" for r in ctx.results)
    n_skip = sum(r.status == "skip" for r in ctx.results)
    print(f"\n{'FAIL' if n_fail else 'PASS'}  {n_ok} ok, {n_fail} failed, {n_skip} skipped in {time.time() - started:.1f}s")
    for name, outcome in suite_outcomes.items():
        print(f"  {outcome:4} {name}")
    if n_fail:
        print("\nfailed checks:")
        for r in ctx.results:
            if r.status == "FAIL":
                print(f"  [{r.suite}] {r.label}{(' — ' + r.detail) if r.detail else ''}")
    if args.report:
        args.report.write_text(json.dumps({
            "base_url": ctx.base_url, "server": ctx.server, "features": ctx.features,
            "suites": suite_outcomes, "ok": n_ok, "failed": n_fail, "skipped": n_skip,
            "results": [r.__dict__ for r in ctx.results],
        }, indent=2))
    return 1 if n_fail else 0


if __name__ == "__main__":
    sys.exit(main())
