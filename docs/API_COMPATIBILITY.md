# API compatibility policy

How the own.audio server API is versioned, and what every client may rely on.
This is the contract between the server (both editions) and the nine clients.
It is binding from the first tagged release of this repository; before that it
is the target the implementation plan works toward.

---

## 1. Two editions, one API

| | Open-source server (this repo) | Hosted service (own.audio) |
|---|---|---|
| Code | `audio2` crate + default binary + web console | A private binary crate that depends on this crate and adds billing, payments and operations |
| API | `/api/v1/...`, `/rest/...` (OpenSubsonic), `/health` | The same, plus `/api/v1/family/billing*`, `/api/v1/billing/*` |
| How a client tells them apart | `GET /api/v1/server` → `edition: "foss"` | `edition: "hosted"` |

A client never branches on `edition`. It branches on `features` (§3). The
`edition` field exists for diagnostics and bug reports only.

---

## 2. Version numbers

Three numbers exist. Do not confuse them.

1. **API major version** — the path prefix, `/api/v1`. It changes only for an
   incompatible redesign. A new major is served **side by side** with the old
   one for at least 12 months, and the old one keeps receiving security fixes.
   Nothing on the roadmap needs `/api/v2`.
2. **Contract revision** — an integer in `GET /api/v1/server` (`api.revision`),
   incremented every time the v1 contract gains something: a new endpoint, a
   new field, a new enum value, a new `features` key. It never goes down and is
   the one number a client may compare against ("statuses by playlist need
   revision ≥ 7"). It is also the version of `docs/api/openapi.json`.
3. **Server release** — semver (`1.4.2`), what the Docker image is tagged with
   and what `server.version` reports. Clients never compare it; it is for
   humans and changelogs.

The OpenSubsonic surface at `/rest` keeps its own versioning rules
(`subsonic/envelope.rs`); this policy does not change them.

---

## 3. Discovery: `GET /api/v1/server`

Unauthenticated, cheap, cacheable for an hour. The only endpoint a client may
call before it knows anything about the server.

```json
{
  "id": "6f1c2a0e-3b7d-4c55-9a0e-2f8e1d7c4b19",
  "name": "own.audio",
  "edition": "foss",
  "version": "1.0.0",
  "api": { "version": 1, "revision": 12 },
  "addresses": [
    { "url": "https://music.example.com", "scope": "public" },
    { "url": "http://192.168.1.20:8080", "scope": "lan" },
    { "url": "http://100.101.102.103:8080", "scope": "vpn" }
  ],
  "features": {
    "registration_open": false,
    "auth": { "local": true, "google": false, "apple": false, "microsoft": false },
    "uploads": { "presigned": true, "multipart_max_bytes": null },
    "narration": false,
    "translation": false,
    "music_identify": true,
    "podcast_discovery": true,
    "podcast_search": true,
    "file_sync": true,
    "library_folders": false,
    "subsonic": true,
    "mail": false,
    "one_family": true,
    "billing": false,
    "payments": false
  },
  "deprecations": []
}
```

Rules:

- **Every optional capability has a key here.** A key is `true` only when the
  feature is fully configured on this server, not merely compiled in. A server
  without a TTS key reports `narration: false`, even though the routes exist.
- **Unknown keys are ignored** by clients; **missing keys mean `false`**. That
  is what lets a new client talk to an old server.
- **A 404 from this endpoint means a server older than revision 1.** Clients
  treat that as the "pre-discovery baseline": local auth, presigned uploads,
  everything else `false`, and fall back to probing `GET /auth/providers`.
- `deprecations` lists endpoints or fields scheduled for removal, each with a
  `sunset` date (§5). Normally empty.
- `podcast_search` (revision 5): `POST /podcasts/search` works. True with
  `podcast_discovery`, and also without it when the server searches Apple's
  public directory instead of its own catalogue; then categories, browse and
  similar shows stay off.
- `one_family` (revision 4): the install has one family. Every new account
  joins it, and removing a member or leaving answers `409`; clients hide
  "leave family" and offer block or delete instead. `false` (or missing): each
  account can have a family of its own.
- `id` (revision 6) is this server's identity, a UUID made once at install
  and never changed. A client that knows a server by several addresses uses it
  to tell that they reach the same server — and must not trust an address
  that answers with another id.
- `addresses` (revision 6) lists every address the server answers on:
  `SERVER__BASE_URL` first, then `SERVER__ADDRESSES` (comma-separated), each
  once, with a `scope` derived from the host — `lan` (private, link-local,
  `.local`), `vpn` (100.64.0.0/10, `*.ts.net`) or `public`. A client learns
  them through whichever address it signed in on, tries them all after each
  network change and keeps the fastest that answers with the same `id`. It
  should send credentials over plain `http` only to `lan` addresses. May be
  empty.
- `demo` (revision 2, optional) is present only on a public demo server:
  `{ "email": "…", "password": "…" }`, the shared account a visitor may sign
  in with. Clients may show it on their sign-in screen and offer to fill it
  in. It is set by `SERVER__DEMO__EMAIL` and `SERVER__DEMO__PASSWORD`. The
  account is read-only (since 1.0.0-beta.4): a write answers `403`
  `{"error": "the demo account is read-only"}`, a Subsonic write error 50.
  Listening still works — playback sessions, progress, the play queue,
  scrobbles — and so do searches and smart-playlist previews.
  `SERVER__DEMO__READ_ONLY=false` lifts it, for a script that fills the demo.

The web console in this repo and every native client hide or disable UI for a
feature whose key is `false`. There is no second source of truth (no
hard-coded "the hosted server has billing").

---

## 4. Rules for changing v1 (additive only)

Within `/api/v1`, a change is allowed only if a client built against any
earlier revision keeps working without being touched.

Allowed, with a revision bump:

- New endpoint.
- New **optional** field in a response. Clients must ignore unknown fields.
- New **optional** field in a request, with a default that preserves the old
  behaviour.
- New enum value, **only** where the contract already tells clients to treat
  unknown values as a documented fallback (e.g. an unknown `item_kind` is
  skipped, an unknown `status` is shown as "unknown"). Where it does not, the
  value needs a new field instead.
- New `features` key.
- Relaxing validation (accepting more).

Not allowed in v1, ever:

- Removing or renaming an endpoint, field, query parameter or enum value.
- Changing a field's type or nullability.
- Changing a status code for a case a client already handles.
- Making an optional request field required.
- Tightening validation so a previously valid request fails.
- Changing the meaning of `404` (which deliberately means "missing *or* not
  visible to you").

If a change you need is on the second list, either add a parallel field or
endpoint (`v2`-suffixed names are fine: `/library/changes2`) and deprecate the
old one (§5), or it is a `/api/v2` discussion.

**A contract change and its documentation land in the same commit**: the
handler, the `utoipa` annotations that regenerate `docs/api/openapi.json`, the
revision bump, the `CHANGELOG.md` entry, and the client guide if the guide
needs prose.

`docs/api/openapi.json` is generated from the handlers' `#[utoipa::path]`
annotations (`backend/src/http/openapi.rs`); routes are registered through
`routes!`, so a documented route is a real one. Its `info.version` is
`1.<revision>`. Two checks run in CI:

- `cargo test` fails when the committed file is not what the code describes
  (`OPENAPI_WRITE=1 cargo test openapi` regenerates it);
- `scripts/check-api-contract.py` compares it with the last release tag and
  fails when an operation disappeared or the document changed without a
  revision bump.

---

## 5. Deprecation

An endpoint or field being retired:

1. Is listed in `GET /api/v1/server` → `deprecations` with `since` (revision)
   and `sunset` (date, at least 6 months out and never earlier than the next
   two minor server releases).
2. Answers with `Deprecation: true` and `Sunset: <http-date>` headers.
3. Keeps working unchanged until the sunset date.
4. Is removed only in a release whose changelog names it, and only after the
   hosted service has observed zero calls to it for 30 days.

Native app stores can take weeks to roll a client out, and users do not update
promptly. Six months is the minimum, not the target.

---

## 6. Error shape and "feature unavailable"

Every error under `/api/v1` is JSON: `{ "error": "..." }`. Most handlers put
an English sentence there, not a code: clients branch on the status, never on
the text. The exceptions that are codes, and contract, are `not_found`,
`feature_unavailable` (below) and `rate_limited`.

A request for a feature this server does not offer (unconfigured narration,
billing on the open-source edition, SSO provider switched off) answers
**`501 Not Implemented`** with
`{ "error": "feature_unavailable", "feature": "<features key>" }` — not 404,
which already means "missing or hidden", and not 500. Clients that honour
`features` never see this; clients that do not get a stable, actionable answer.

A route that does not exist at all answers `404 { "error": "not_found" }`.

---

## 6a. Media URLs are opaque

`StreamResponse.url` (and every cover or download URL) is a string the client
fetches as given, within `expires_in_secs`. It may be a presigned S3 URL or a
route on the server itself (`/api/v1/media/…`), depending on where the file
lives. Clients never parse it, never persist it, and never assume its host.

## 7. Authentication tokens

- Access and refresh tokens are opaque to clients. Their format, lifetime and
  claims may change at any release without a revision bump.
- The refresh flow (`POST /auth/refresh`) and its error codes are contract.
- A server that cannot verify a token answers `401`; clients then refresh once
  and, on a second `401`, sign out. This is already how every client behaves
  and it must stay so.
- **A signed-in caller without the right answers `403`** (since 2026-10-11;
  it was `401`, which made clients refresh a good token and sign out). Nothing
  was in production, so the status changed in place.
- **Known quirk, kept for v1:** some handlers answer `401` for an item that
  does not exist, where `404` would be right: family members, invites and
  join codes (`/family/*`), users (`/users/*`), jobs, uploads and statistics
  for another member. A client must not sign out on a `401` from these routes
  when a refresh succeeded. The fix waits for `/api/v2` (§11). Newer code
  answers `404` for missing or hidden items, as `/podcasts/*`, music and
  audiobooks do.
- **Access tokens live one hour** by default (`AUTH__ACCESS_TTL_SECS`); they
  were seven days until 2026-10-11. Clients refresh, as they already do.
- **Uploads and storing podcast episodes need `can_upload`** (`403` without
  it), and `POST /podcasts/subscribe` answers `400` for a feed on a private or
  local address (since 2026-10-11).
- **A password must be 12 characters or more** when set (`400` below that);
  it was 8 until 2026-10-11. Sign-in with an older, shorter password still
  works.
- **`POST /auth/login` answers `429` for too many wrong passwords** on one
  email (`error: account_locked`, `Retry-After` in seconds) as well as for
  the per-IP limit (`error: rate_limited`). Treat both as "wait", show the
  seconds; neither says whether the email exists.
- **`402` means the family has no room**: `POST /uploads/presign`,
  `POST /uploads/complete` and storing a podcast episode answer it when a
  server's `STORAGE__FAMILY_QUOTA_BYTES` is reached (or, on a hosted server,
  the credit is used up). Show the `error` text; it says what is missing.
- **Browsers may call `/api` only from the console's origins**
  (`SERVER__CORS_ORIGINS`, defaulting to `SERVER__APP_BASE_URL` and
  `SERVER__BASE_URL`); `/rest` allows any origin. Native apps are unaffected.

---

## 8. Database and upgrades

- Migrations are forward-only and numbered. A release never edits a shipped
  migration. Downgrading a server is not supported; back up before upgrading
  (the installer prints how).
- The open-source server owns the whole core schema, including a few tables
  the hosted edition uses and this edition does not (`credit_ledger`,
  `credit_alerts`, `stripe_events`). They stay in the core sequence so that a
  database created by either edition can be run by either edition. Hosted-only
  tables created after the split live in their own Postgres schema (`hosted`)
  with their own migration table; the core never references them.
- Skipping releases is supported: all pending migrations run on start.

---

## 9. Compatibility matrix

Each release's `CHANGELOG.md` entry states the contract revision. Each client
repo's README states the **minimum contract revision** it needs and the
revision it was last tested against. The conformance suite
(`conformance/`, §10) is what "tested against" means.

Servers support every client released in the previous 12 months. Clients
support every server at or above their stated minimum revision.

---

## 10. Conformance suite

`conformance/` is a black-box test suite that takes a base URL and
credentials and exercises the contract: auth, families, library sync,
playback, uploads, trash, Subsonic. It passes against the open-source compose
stack and against the hosted canary in both repositories' CI. A `features`
key that is `false` skips that feature's tests rather than failing them.

The suite is the executable half of this document. When the suite and this
document disagree, fix whichever is wrong in the same commit.

---

## 11. For `/api/v2`, not before

Changes v1 cannot make (§4), collected so a v2 is one deliberate step:

1. `404` instead of `401` for a missing item in `/family/*`, `/users/*`,
   jobs, uploads and statistics (§7).
2. Error bodies with a machine-readable `code` on every error, beside the
   human `error` text.
