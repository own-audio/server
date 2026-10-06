# Conformance suite

Black-box tests of the server API, run against any own.audio server by URL:
a laptop, the compose stack, canary, production. This is the executable half
of [docs/API_COMPATIBILITY.md](../docs/API_COMPATIBILITY.md): when the suite
and the document disagree, one of them is wrong and gets fixed in the same
commit.

```bash
# local compose stack (defaults shown)
python3 conformance/run.py --base-url http://127.0.0.1:8080 \
    --admin-email admin@audio2.local --admin-password admin \
    --compose-dir ../audio2          # enables the SQL-backed suites

# a remote server: only what the API alone can prove
python3 conformance/run.py --base-url https://api-canary.example \
    --admin-email ci-smoke@example --admin-password "$CANARY_ADMIN_PASSWORD"

python3 conformance/run.py --list            # suite names
python3 conformance/run.py --only smoke,trash
python3 conformance/run.py --report out.json # machine-readable result
```

Exit code is 1 on any failed check. Skips never fail a run.

## What it needs from the server

An instance-admin account. Every suite creates its own users, families and
media under unique `…-<suffix>@example.com` addresses and deletes them
afterwards; nothing it makes is meant to survive, and it never touches
accounts it did not create. The admin's own password is not changed.

The suite reads `GET /api/v1/server` first and skips the checks for any
feature the server reports as `false`. A server older than that endpoint
(404) is treated as the pre-discovery baseline: local auth, presigned
uploads, file sync and Subsonic on, everything else off.

## Suites

| Suite | Covers | Needs |
|---|---|---|
| `server` | `GET /server` shape, `404 not_found` vs `501 feature_unavailable`, features agree with `/auth/providers` | — |
| `smoke` | health, login, users, refresh tokens, library listing, streaming, progress, smart playlists, families, Subsonic | — |
| `families` | private vs family visibility, sharing, parental policies, who can stream what | — |
| `join` | link and QR invites, account claim, invite guardrails | — |
| `mobile` | cross-device play queue, incremental sync with tombstones, grouped browsing, bulk ops, push registration | — |
| `stats` | listening history batches, idempotent retries, device attribution, day bucketing, stats privacy | — |
| `trash` | 30-day trash: delete, list, restore, purge, permissions | `--compose-dir` (backdates rows through SQL) |
| `filesync` | the own.audio folder: paths, sync feed and cursor, shortcuts, holdings, auto-stored episodes | loopback `--base-url` and `--compose-dir` (serves an RSS feed the server fetches from this machine) |

## Writing a suite

A module in `suites/` with `NAME`, a docstring whose first line is the
summary, optionally `REQUIRES = {"db"}` or `{"local"}`, and `run(ctx)`.
Everything goes through `Ctx` in `core.py`: `call`, `check`, `skip`,
`make_user` (cleans up after itself), `upload_track`, `presign_upload`,
`sql`, `on_cleanup`. Raise `Skip` to stop a suite cleanly; any
`AssertionError` (including `ApiError` from an unexpected status) aborts the
suite as a failure and still runs its cleanups.

Rules:

- A check tests the contract, not an implementation detail. If it only
  holds on one edition, gate it on `ctx.feature(...)`.
- Labels are stable: CI diffs them between runs.
- No credentials in this directory, ever. They come from the command line.

## History

Consolidated 2026-10-06 from eight scripts in the original repository
(`backend_smoke_test`, `family_sharing_test`, `family_join_test`,
`mobile_sync_test`, `stats_test`, `trash_test`, `filesync_test`;
`seed_test_family` stayed a tool). The load test was not carried over.
