# audio2 Backend Specification for Mobile (iOS/Android) Clients

Date: 2026-07-18
Status: living reference — update as the backend changes.
Related:
- [android-client-guide.md](android-client-guide.md) — practical guide for building the Android app
- [backend-gap-analysis-abs-navidrome.md](backend-gap-analysis-abs-navidrome.md)

This document describes the audio2 backend's HTTP API as it exists today,
well enough to build a native mobile client against it, and lists what a
serious ("AAA-class") all-in-one audiobook/podcast/music app would need —
both from this backend and from the client itself. The backend is
platform-neutral: everything here works the same for iOS and Android, and
where the platforms differ (push transport, background scheduling) both are
supported.

---

## 1. Transport & conventions

- Base path: `/api/v1` for the native REST API. A separate OpenSubsonic-
  compatible surface is mounted at `/rest` for music only (see §7).
- Discovery: `GET /api/v1/server` (public) → `{ name, edition, version,
  api: { version: 1, revision: N }, features: { … }, deprecations: [] }`.
  Clients gate optional UI on `features.*` (missing ⇒ false, unknown ⇒
  ignored) and may compare `api.revision`. The contract and its change
  rules: `own-audio-foss/docs/API_COMPATIBILITY.md`.
- Hosted-only routes on a server that does not serve them answer
  `501 { "error": "feature_unavailable", "feature": "<features key>" }`;
  unknown routes `404 { "error": "not_found" }`.
- Rate limits (per client IP; `SERVER__RATE_LIMIT__*`): `429
  { "error": "rate_limited", "retry_after_secs": N }` + `Retry-After` on
  `/auth/login`, `/auth/refresh`, `/auth/device/*`, `/join/*`,
  `/setup/complete`.
- All request/response bodies are JSON unless otherwise noted (file uploads
  use `multipart/form-data`).
- Multipart **text** fields must be UTF-8. The server decodes UTF-8 first and
  Windows-1250 second, and returns 400 `form field is not valid text` for
  anything else rather than storing a lossily-decoded value.
- All IDs are UUIDv4, serialized as strings.
- All timestamps are RFC3339 strings (UTC).
- Errors: `{ "error": "<message>" }` with a matching HTTP status
  (400 bad request, 401 unauthorized/session invalid, 403 forbidden,
  404 not found, 409 conflict, 429 rate limited, 501 feature unavailable,
  500 internal).
- CORS is currently permissive (`CorsLayer::permissive()`); no mobile-specific
  CORS concerns, but tighten before shipping publicly.

## 2. Authentication

**Model**: short-lived access JWT + long-lived rotating refresh token
(added 2026-07-18, Phase 0 of the family plan).

- Header: `Authorization: Bearer <jwt>`
- Claims: `sub` (user id), `role`, `iat`, `exp`, `jti` (session id), `chain`
  (device chain id, present on tokens minted via login/refresh).
- Access TTL: `access_ttl_secs` (falls back to the deprecated
  `session_ttl_secs`, default 7 days, until deployments opt into e.g. 1h).
  Refresh TTL: `refresh_ttl_secs`, default 90 days.
- Refresh tokens rotate on every use; presenting an already-used token is
  treated as theft and revokes the whole device chain.
- Session revocation is enforced by middleware on every request — logout,
  device sign-out, admin revoke, and password change take effect
  immediately, not at token expiry.

### Endpoints

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/setup/status` | none | Whether an admin exists yet + DB/storage health. First-run gate. |
| POST | `/setup/complete` | none | Create the first admin account (fails if any user already exists). Returns token + user. |
| POST | `/auth/login` | none | `{email, password, device_name?, device_kind?}` → `{token, refresh_token, user}`. `device_kind` ∈ `web\|ios\|android\|macos\|windows`; anything else is stored as `other`. |
| POST | `/auth/refresh` | none | `{refresh_token}` → `{token, refresh_token, user}` with a **rotated** refresh token. Reusing the old one 401s and kills the device chain. |
| POST | `/auth/fork` | none | `{refresh_token, device_name?, device_kind?}` → `{token, refresh_token, user}` for a **new, independent** device chain; the presented token is checked but not rotated (a rotated one 401s and kills its chain, as on refresh). For a helper process that must refresh on its own — the Mac's Finder extension (file-sync-plan §7.1): two processes sharing one chain would present each other's rotated tokens. |
| POST | `/auth/register` | none | `{email, password, display_name}` → `{token, refresh_token, user}` (only if `registration_open`). |
| GET | `/auth/registration-status` | none | `{registration_open: bool}` — check before showing a register screen. |
| POST | `/auth/logout` | bearer | Signs out this device: revokes the session and its refresh chain. |
| GET | `/auth/me` | bearer | Current user info. |
| POST | `/auth/password` | bearer | Change password; revokes all *other* sessions/devices, keeps the current one. |
| GET | `/auth/sessions` | bearer | Signed-in devices: `{chain_id, device_name, device_kind, signed_in_at, last_used_at, expires_at, current}`. |
| DELETE | `/auth/sessions/{chain_id}` | bearer | Sign out one device ("log out my old iPhone"). |
| POST | `/auth/admin-create-user` | bearer (admin) | Admin creates a user directly. |
| GET | `/auth/providers` | none | `{local, google: {enabled, desktop_client_id, web_client_id}, apple: {enabled, web_client_id}, microsoft: {enabled, desktop_client_id}}` — check before showing SSO buttons; all are `enabled: false` until configured (see `docs/sso-payments-plan.md`). **`microsoft` was added 2026-09-13**: a client must tolerate its absence, since an older server omits the key rather than sending it disabled. An unset client id is `null`, never `""`. |
| POST | `/auth/google` | none | Native Google sign-in — dormant (`501`) until `AUTH__GOOGLE__*` is set. Body: either `{id_token}` or `{code, code_verifier, redirect_uri}` (loopback+PKCE — the backend does the code exchange). Optional `device_name?`, `device_kind?`, `invite_code?`. → `{token, refresh_token, user}`, `201` on first sign-in (new account), `200` otherwise. |
| POST | `/auth/microsoft` | none | Native Microsoft sign-in — dormant (`501`) until `AUTH__MICROSOFT__*` is set and `AUTH__MICROSOFT__ENABLED=true`. **Body and responses are identical to `/auth/google`**, including the loopback-only `redirect_uri` rule. Uses the `common` authority, so work/school and personal accounts both sign in. Microsoft issues no `email_verified` claim and the identity is stored unverified; where no `email` claim is present, `preferred_username` is used when it looks like an address, and a token with neither is rejected with `400`. |
| POST | `/auth/apple` | none | Native Sign in with Apple — dormant (`501`) until `AUTH__APPLE__CLIENT_IDS` is set. Body: `{identity_token, full_name?, device_name?, device_kind?, invite_code?}` (`full_name` only arrives on the user's first-ever authorization). Same response shape as `/auth/google`. The web app uses the same endpoint with the `id_token` from Sign in with Apple JS, passing `apple.web_client_id` from `/auth/providers` as the SDK's `clientId`. |
| GET/POST | `/auth/google/redirect`, `/auth/google/callback`, `/auth/microsoft/redirect`, `/auth/microsoft/callback` | — | **Stubs only** (`"TODO"` text response) — the browser-redirect flow for a future web console. Unrelated to `POST /auth/microsoft`, which is real; native clients use that. |

Both SSO endpoints answer `501 { "error": "provider not configured" }` while
their config section is unset — treat that identically to `enabled: false`
from `/auth/providers`, never as a real error.

New-account creation via SSO is gated exactly like `/auth/register`: either
open registration or a valid `invite_code`. An SSO identity whose email
matches an existing account links to it automatically, but only when the
provider reports the email as verified — an unverified match answers `409`
(`IdentityConflict`) rather than silently taking over the account.

**Mobile implication**: store the refresh token in the iOS Keychain /
Android Keystore, refresh the access JWT on 401 or proactively before
expiry, and replace the stored refresh token on every refresh response
(rotation). A 401 from `/auth/refresh` means the device chain is dead —
force a credential re-login.

## 3. Users

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/users/me` | bearer | Own profile. |
| PATCH | `/users/me` | bearer | Update any of `display_name`, `recommendations_enabled` (see below) and `discovery_languages` (sent whole; `[]` means every language; regional tags are folded to their base, `en-US` → `en`). Answers **`204` with no body** — re-read `GET /auth/me` for the stored values. |
| DELETE | `/users/me` | bearer | Self-delete (blocked if last admin). |
| GET | `/users/me/subsonic-key` | bearer | Fetch/create the per-user OpenSubsonic API key (for `/rest`). |
| POST | `/users/me/subsonic-key/regenerate` | bearer | Rotate that key. |
| POST | `/users/me/avatar` | bearer | `multipart/form-data`, field `avatar` (image, ≤5MB). Sets the caller's own profile picture. Self-serve only — see `/family/members/{id}/avatar` below for the one admin-managed exception. |
| DELETE | `/users/me/avatar` | bearer | Removes the caller's own photo. |
| GET | `/users/{id}/avatar` | bearer, fellow family member | Raw image bytes. Family-scoped, not the self-or-instance-admin rule `GET /users/{id}` uses — any member of the same family can see it. |
| GET | `/users/` | bearer (admin) | List all users. |

**`recommendations_enabled`** (on `GET /users/me`, settable via `PATCH
/users/me`) is **false for every account until the user turns it on**. While it
is false a client must compute and display nothing that is derived from
listening history — not compute it and hide it. Nothing is stored server-side
when it goes on and nothing is deleted when it goes off, because there is no
server-side profile either way; that is what the switch is protecting.

It is deliberately **not** on `PATCH /users/{id}`: an admin can deactivate an
account or change its role, but cannot decide for someone else that their
listening may be used to suggest things. See
`docs/podcast-recommendations-plan.md` §0.1.
| GET/PATCH/DELETE | `/users/{id}` | bearer (self or admin) | Read/update/delete a specific user. |
| POST | `/users/{id}/revoke-sessions` | bearer (admin) | Force logout a user. |

`UserResponse`/`MemberResponse` carry `avatar_url` (`/api/v1/users/{id}/avatar`
when a photo is set, else `null`) alongside every other field.

`users.role` is a flat `"admin" | "user"` string describing **instance**
privileges. Family-scoped roles live separately (see §4a) — a `family_admin`
is not an instance admin and vice versa, though instance admins are treated
as family admins for management actions.

## 4a. Families

Every account belongs to exactly one family; solo users get a "personal
family of one" they administer, so clients never branch on "has a family".

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/family/` | bearer | My family: `{id, name, my_role, my_user_id, avatar_url, members[], created_at}`. |
| PUT | `/family/` | family_admin | Rename the family. |
| POST | `/family/avatar` | family_admin | `multipart/form-data`, field `avatar` (image, ≤5MB). Sets the family's own shared photo — distinct from any member's personal one. |
| DELETE | `/family/avatar` | family_admin | Removes the family photo. |
| GET | `/family/avatar` | bearer | Raw image bytes — any family member, not just admins. |
| GET | `/family/members` | bearer | Members with `{user_id, email, display_name, display_label, role, is_active, pending, avatar_url, joined_at}`. `pending` = a provisioned account nobody has claimed yet (no auth identity, can't sign in). |
| POST | `/family/members/provision` | family_admin | `{display_name, login_email, display_label?}` → `{member, invite}`. Creates an account with no password for a member with no mailbox of their own (see §4a-join); `invite` is a `claim`-kind code to hand them. |
| PUT | `/family/members/{user_id}` | family_admin | Set `role` (`family_admin\|member`) and/or `display_label` (send `null` to clear). |
| DELETE | `/family/members/{user_id}` | family_admin, or self | Remove a member / leave. A claimed member is re-homed into a fresh personal family; an unclaimed provisioned account is deleted outright (nothing to preserve). |
| POST | `/family/members/{user_id}/block` | family_admin | Deactivates the account (`is_active = false`) and immediately revokes every session and refresh token — an already-signed-in device is cut off too, not just future logins. Doesn't touch family membership; a blocked member is still listed, just `is_active: false`. A family_admin cannot block themselves. |
| POST | `/family/members/{user_id}/unblock` | family_admin | Reactivates the account. They still have to sign in again. |
| POST | `/family/members/{user_id}/avatar` | family_admin | `multipart/form-data`, field `avatar` (image, ≤5MB). Sets a fellow member's photo on their behalf — the one admin-managed exception to the self-serve `/users/me/avatar` rule. |
| DELETE | `/family/members/{user_id}/avatar` | family_admin | Removes a fellow member's photo. |
| GET | `/family/invites` | family_admin | Pending (unexhausted) invites. |
| POST | `/family/invites` | family_admin | `{kind?, email?, role?, max_uses?, label?}` → `{id, kind, email, code, role, label, max_uses, use_count, expires_at, join_url, …}`. See §4a-join. |
| DELETE | `/family/invites/{id}` | family_admin | Revoke a pending invite. |
| POST | `/family/invites/{id}/regenerate` | family_admin | Mint a fresh code + TTL for a pending invite; the old code stops resolving immediately. |
| POST | `/family/invites/accept` | bearer | `{code}` — join the inviting family. `email`-kind invites must match the caller's address; `link`-kind invites accept anyone; `claim`-kind invites are rejected here (see `/join/{code}/claim` below). |

**Invites and registration**: `POST /auth/register` accepts an optional
`invite_code`, for `email` and `link` invites only. A valid code authorizes
account creation **even when `registration_open` is false**, and places the
new account directly in the inviting family.

**Guards worth knowing client-side**: the last `family_admin` cannot be
demoted or removed while other members remain (400), and non-admin members
get 403 on any family-admin route.

### 4a-join. Frictionless join — QR / link / claim (docs/family-join-qr-plan.md)

`POST /family/invites` takes three `kind`s:

- **`email`** (default) — `{email, role?}`. Single-use, 14-day TTL, bound to
  one address, may grant `family_admin`. Mailed automatically when the
  server has `mail.*` configured (JMAP — see `crate::mail`); the response
  always includes the code/`join_url` too, so out-of-band delivery still
  works if mail is unset.
- **`link`** — `{max_uses?, label?}`. No email, 7-day TTL, **always
  `member` role** (a shareable/QR code must never be able to grant admin —
  requesting `role: "family_admin"` on a link invite is a 400). `max_uses`
  1–20 (default 1) for a code meant to onboard several people (a fridge QR).
- **`claim`** — never created directly; `POST /family/members/provision`
  makes one. 30-day TTL, single-use, bound to the account it activates.

Every invite response carries `join_url` (`{server.base_url}/join/{code}`,
`null` if `server.base_url` is unset — fall back to building the URL from
whatever origin the client is connected to) alongside the bare `code` for
manual entry/paste.

**Public join endpoints** (no auth) — the landing page a QR/link opens:

| Method | Path | Purpose |
|---|---|---|
| GET | `/join/{code}` | Preview: `{status: "valid"\|"expired"\|"exhausted", kind, family_name, inviter_name, role, member_count, expires_at, uses_left, claim}`. Unknown codes are a plain 404. `expired`/`exhausted` reveal only `status` — nothing about the family. `claim` is `{display_name, login_email}` for `kind: "claim"`, else `null`. |
| POST | `/join/{code}/claim` | `claim`-kind only. `{password, device_name?, device_kind?}` → `{token, refresh_token, user}` (same shape as login). Sets the provisioned account's password and signs it in; possession of the code is the only credential needed, same as every other invite kind. |

## 4b. Private vs family content, and parental controls

Every item has an owner (`user_id`) and lives in one of two folders:

- **private** — visible only to its owner. *Not* visible to family admins.
- **family** — shared with the owner's family, subject to the rules below.

Uploads and creates take a `visibility` field (`private` | `family`). **When
the field is absent the server stores the item as private**, so a client that
does not know about the feature can never accidentally expose content.

| Method | Path | Purpose |
|---|---|---|
| PUT | `/music/tracks/{id}/visibility` | `{visibility}` — move between folders. Owner only. |
| PUT | `/music/playlists/{id}/visibility` | As above. |
| PUT | `/audiobooks/{id}/visibility` | As above. |
| PUT | `/podcasts/{id}/visibility` | As above. |
| GET | `/library/private` | The caller's never-shared items across all kinds: `{kind, id, title, subtitle, cover_url}`, songs also `album` (sorted by album, then title). Always self-scoped. |

Content responses carry `visibility` (`private`/`family`), `is_owner` and `owner_id` (the owning member, for family shortcuts §8f).
**Non-owners can play shared items but cannot edit, delete, or re-share
them** — those return 404.

### Parental controls (family_admin only)

These govern **shared content only** and can never reach into a private
folder.

| Method | Path | Purpose |
|---|---|---|
| GET | `/family/members/{user_id}/access` | Effective `policies` + `grants` for a member. |
| PUT | `/family/members/{user_id}/policy` | `{media_kind, policy}` — per-kind default, `allow_all` (implicit) or `deny_all`. |
| PUT | `/family/members/{user_id}/grants` | `{media_kind, allow[], deny[]}` — bulk replace item-level overrides. |
| GET | `/family/content/{kind}/{item_id}/audience` | Who in the family can currently play this item. |
| PUT | `/family/content/{kind}/{item_id}/audience` | Set who may play it: `{can_listen: [user_id]}`, the **whole** audience rather than a delta — anyone left out is denied. Family admin only; the item must be shared with the family (400 otherwise). Returns the audience as the rules now resolve it, which is not always what was asked: the item's owner and every family admin keep access unconditionally, so leaving one out is accepted and simply has no effect on them. Writes the minimum needed — a member whose own `allow_all`/`deny_all` default already gives the wanted answer gets no grant row, so changing that default later still works. |
| POST | `/family/content/{kind}/{item_id}/request-access` | Any family member — even one this item is currently hidden from. Notifies every family admin (kind `access_request` in the notification inbox, `data: {media_kind, item_id, requester_id}`) rather than one specific person, since a family has no single permissions owner. 404 when the item is not shared with the caller's family (same answer as a nonexistent item, on purpose). No de-duplication — a second tap queues a second notification; the client disables the button after success. |

`media_kind` is `audiobook`, `podcast`, or `music`. Resolution order for a
shared item: **owner always wins** → family admin sees all shared content →
otherwise an item-level grant, falling back to the member's per-kind policy,
which defaults to `allow_all`.

Playlists, collections, and series are shareable but **not** individually
grantable: they follow the member's policy for their parent kind (music for
playlists, audiobook for collections and series).

## 4c. Storage & credit

Storage is billed against a **credit ledger**. Every family starts with a
one-time **$5 welcome credit per member** (granted once per user, ever, the
first time they get a personal family — joining an existing family does not
grant a second one), and a background sweep debits the ledger once per UTC
day for that day's storage at **$0.05/GB/month** (decimal GB, 10⁹ bytes).
Money is always **micro-USD integers** (`1_000_000` = $1) to avoid rounding a
sub-cent daily charge to zero. Real top-ups are optional and **dormant until
configured** (see `payments` below and `docs/sso-payments-plan.md`) — until
then every balance is still play money, exactly as before.

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/family/billing` | bearer (any member) | Storage breakdown, cost estimates, credit balance, recent ledger entries, and top-up availability. |
| POST | `/family/billing/topup` | bearer (any member) | `{amount_micro}` (bounds from `payments.min_micro`/`max_micro`) → `{checkout_url}` — a Stripe Checkout session to open in a browser. `501` while Stripe is unconfigured. |

Unlike the parental-control endpoints above, this one is **not**
`family_admin`-gated — the credit is family-pooled money and every member can
see what the family's library costs.

Response:

```json
{
  "storage": {
    "total_bytes": 4434327403,
    "audiobooks_bytes": 3445708296,
    "podcasts_bytes": 28227357,
    "music_bytes": 960391750,
    "other_bytes": 0,
    "unsized_objects": 0
  },
  "pricing": { "currency": "USD", "price_per_gb_month_micro": 50000, "gb_bytes": 1000000000 },
  "balance_micro": 29992847,
  "estimated_daily_cost_micro": 7153,
  "estimated_monthly_cost_micro": 221717,
  "days_remaining": 4193,
  "runs_out_on": "2038-02-14",
  "depleted": false,
  "last_charge": { "charge_date": "2026-08-23", "storage_bytes": 4434327403, "amount_micro": -7153 },
  "entries": [
    { "id": "…", "entry_type": "storage_charge", "amount_micro": -7153,
      "storage_bytes": 4434327403, "charge_date": "2026-08-23",
      "note": null, "created_at": "2026-08-23T10:54:52Z" }
  ],
  "alerts": {
    "min_balance_micro": 5000000, "min_days_remaining": 14,
    "balance_alert_active": false, "days_alert_active": false
  },
  "payments": {
    "enabled": false, "currency": "USD",
    "presets_micro": [5000000, 10000000, 25000000],
    "min_micro": 5000000, "max_micro": 100000000
  }
}
```

- `storage.unsized_objects` counts referenced objects with no recorded size
  (a legacy-upload edge case) — a non-zero value means `total_bytes` is a
  known undercount, never an overcount.
- `days_remaining` / `runs_out_on` are `null` when `estimated_daily_cost_micro`
  is 0 (empty library — cost never depletes the balance).
- `depleted` is **informational only**. Clients must not gate playback,
  upload, or any other action on it — nothing is enforced while the credit is
  play money. Treat it the same as a low-battery indicator: shown, not acted
  on.
- `entries` is the 30 most recent ledger rows, newest first (`welcome_grant`,
  `storage_charge`, `topup`, and reserved for later use: `grant`,
  `adjustment`). `topup` entries carry `external_ref` (the Stripe
  checkout-session id); every other kind has it `null`.
- `payments.enabled` mirrors whether Stripe is configured — hide the
  add-credit UI while false, same pattern as `/auth/providers`. Never
  hardcode `presets_micro`/`min_micro`/`max_micro` client-side; read them
  from here so a server-side bounds change doesn't need a client rebuild.
  After calling `/family/billing/topup`, open `checkout_url` in a browser;
  Stripe redirects it to `/billing/success` or `/billing/cancel` (plain HTML
  pages the backend serves, outside `/api/v1`) when done. The webhook
  usually credits the ledger within seconds — poll or refetch
  `GET /family/billing` rather than trusting the redirect alone, since nothing
  guarantees the browser tab is still open when the webhook lands.
- Storage only counts objects still referenced by a live library row —
  deleting content is not (yet) reflected as freed storage on the object
  store itself, but is immediately reflected here since the reference is gone.
- `alerts` (see below) is `null` for a non-admin caller — it's a management
  setting, not general billing info, so it's quietly absent rather than
  403-ing the whole response.

### Credit alerts

| Method | Path | Auth | Purpose |
|---|---|---|---|
| GET | `/family/billing/alerts` | family_admin | Configured thresholds. Absent settings read as "all off", not 404. |
| PUT | `/family/billing/alerts` | family_admin | `{min_balance_micro, min_days_remaining}` — set or clear both. `null` turns a rule off. |

A background sweep evaluates every family with at least one threshold
configured once per day (immediately after the storage charge — see
`db::billing::days_remaining` and `execute_storage_billing` in
`jobs/worker.rs`), and queues an in-app notification (`kind: "credit_low"`,
via the existing `GET /devices/notifications` inbox — **there is no push**,
CLAUDE.md §3) to every family admin when a threshold is first crossed:

```json
{
  "min_balance_micro": 5000000,
  "min_days_remaining": 14,
  "balance_alert_active": false,
  "days_alert_active": false
}
```

- `balance_alert_active` / `days_alert_active` are **edge-trigger latches**,
  not a log of whether the family is currently low. An alert fires once on
  the transition above→below threshold, then stays silent — including
  every subsequent day the family remains below it — until the balance
  recovers above the threshold, which clears the latch **without** a second
  notification. A client showing these flags is showing "has this already
  been sent", not "is the family currently low" (compute that yourself from
  `balance_micro` / `days_remaining` on `GET /family/billing`).
- Setting either threshold (even to the same value) resets both latches, so
  a just-changed or just-re-enabled rule can fire again immediately.
- The balance rule fires at `balance_micro <= min_balance_micro`; the days
  rule fires at `days_remaining < min_days_remaining` (matching "less than N
  days" wording) — and never fires at all when `days_remaining` is `null`
  (infinite runway is never "low", however small the threshold).
- The `credit_low` notification's `data` payload:
  `{"balance_micro": …, "days_remaining": …|null, "rule": "balance"|"days"}`.
- Same standing rule as `depleted`: **display only, never enforce.** A low
  or even zero balance blocks nothing.

## 4d. Trash

docs/file-sync-plan.md §5.1. Deleting a book, track, playlist or stored
episode copy moves it to the trash for **30 days**; the daily `trash_purge`
job then deletes the row and its storage objects. Trashed items are invisible
to every other endpoint, the Subsonic surface included, and the delete writes
the usual `/library/changes` tombstone. A restore bumps `updated_at`, so the
item comes back through `/library/changes` like any change.

**Permissions** (delete, restore, purge): the owner, or a family admin when
the item is shared with their family (`family_id` set). Seen but not
permitted → `403`; not visible → `404`. The delete routes still answer `204`.

**`X-Trash-Batch: <uuid>`** (optional header on every delete route) groups one
user gesture; without it each delete is its own batch.

| Method | Path | Purpose |
|---|---|---|
| GET | `/trash?scope=mine\|family` | Trashed items. `mine` (default): owned by the caller. `family`: shared with the caller's family — family admins only, else `403`. |
| POST | `/trash/{kind}/{id}/restore` | `{restored, charged_micro}`. 404 when not in the trash or not the caller's to restore. |
| POST | `/trash/batches/{batch}/restore` | Restores what the caller may restore from one batch. 404 when nothing was. |
| DELETE | `/trash/{kind}/{id}` | Delete forever now; objects nothing else references are removed. `204`. |
| POST | `/trash/empty?scope=mine\|family` | `{purged}`. |
| GET | `/admin/families/trash` | Instance admin: per family `trashed_items`, `trashed_bytes`, `restores_30d`, `restored_bytes_30d`, `max_restores_one_item_30d`, `library_bytes`, `flagged`. |

`kind`: `audiobook` · `music_track` · `playlist` · `podcast_episode` ·
`companion_file` (§8f; its `title` is the file name). Row:
`{kind, id, title, owner{id,display_name}, trashed_by{id,display_name}|null,
trashed_at, purge_at, size_bytes, batch, restore_charge_micro}`.

**Billing.** Trashed items are not counted in the daily storage charge. A
restore charges the whole days the item spent in the trash at the daily rate
(`daily_charge_micro × days`, clamped to the balance), as a ledger entry
`trash_restore_charge` — zero on the day it was deleted, and zero when
`BILLING__CHARGES_ENABLED=false`. `restore_charge_micro` in the list is that
figure for a restore right now.

**Notification** `item_trashed` goes to the owner when someone else (a family
admin) trashed their item: `data: {kind, id, title, by, purge_at}`.

**Account deletion** removes the account's content immediately, trash
included, together with the storage objects that leaves unreferenced.

## 4. Library (cross-content)

| Method | Path | Purpose |
|---|---|---|
| GET | `/library/continue` | Up to 20 most-recent, in-progress (not completed) items across podcasts + audiobooks, merged and sorted by `updated_at`. Feeds a mobile "Continue Listening" home row. |
| GET | `/library/search?q=&limit=` | Case-insensitive search across podcast feeds, episodes, audiobooks, **and music tracks**. Results are tagged by `kind` and scoped to what the caller may play. |
| GET | `/library/private` | The caller's private (never-shared) items — see §4b. |

## 5. Podcasts

| Method | Path | Purpose |
|---|---|---|
| GET | `/podcasts/` | List subscribed feeds. |
| POST | `/podcasts/search` | `{q, language?}` → search the **self-hosted** Podcast Index catalogue. Was a proxy to `apollo.rss.com` until 2026-08-29; now nothing about a search leaves the deployment. `q` must be ≥2 characters (400 otherwise). `language` is a subtag (`en`, **not** `en-US`) and is optional — unset means no filter. |
| GET | `/podcasts/discover/categories` | The catalogue's 107 categories with `feed_count` and `active_count`. Show `active_count`: three quarters of the catalogue last published over a year ago. |
| GET | `/podcasts/discover/browse?category=&language=&offset=` | Best feeds in a category, already filtered against what the household follows. The cold-start surface — works for a user who has listened to nothing. |
| GET | `/podcasts/{id}/similar` | Shows like this one, filtered against what the household follows. Empty list (not an error) when the feed has no catalogue entry. **Takes no listening history and returns none** — see docs/podcast-recommendations-plan.md §0. |
| POST | `/podcasts/subscribe` | `{feed_url}` → subscribe (also accepts YouTube channel URLs, treated as a podcast-like feed). |
| GET/DELETE | `/podcasts/{id}` | Feed detail / unsubscribe. |
| — | *(feed fields)* | Feed objects carry `categories` (lowercased, from the catalogue or the feed's own iTunes tags) and `language_base` (the language subtag). **Group and filter on `language_base`, never on `language`** — the wild contains 112 spellings of English. |
| GET | `/podcasts/{id}/image` | Proxied cover art (no auth — UUID acts as capability token). |
| GET | `/podcasts/{id}/episodes?limit=&offset=&q=` | Paginated episode list, each annotated with the caller's `progress_secs`/`completed`. `q` keeps episodes whose title or description holds it (any case, Czech accents ignored). |
| POST | `/podcasts/{id}/refresh` | Re-pull the RSS/YouTube feed synchronously, ingest new episodes. |
| PUT | `/podcasts/{id}/auto-store` | `{enabled}` → the feed. While on, the server stores every episode **published from then on** itself (an `episode_download` job per episode, no backlog), so a paid feed's episode is kept for the family even if its link expires. An episode whose stored copy was deleted is never stored again. Subscriber, or a family admin when the feed is shared; else `403`. Feed objects carry `auto_store`, and from `GET /podcasts` and `GET /podcasts/{id}` also `has_transcripts` — some episode publishes a transcript, so the show's episodes can be translated (false in other responses). |
| POST | `/podcasts/{id}/store-all` | `{preview?, latest?}` → `{episodes, estimated_bytes}`. Stores the back catalogue on the server — every episode not stored yet, or the newest `latest` of them — one `episode_download` job each; `preview` only counts and sizes (durations at 128 kbit/s). An episode is never queued twice, and one whose copy was deleted is not stored again. Same permission as auto-store. |
| POST | `/podcasts/{id}/sync-images` | Backfill missing channel/episode artwork into S3. |
| POST | `/podcasts/{id}/episodes/{ep_id}/download` | Fetch the episode's enclosure and persist it to S3 (idempotent). |
| DELETE | `/podcasts/{id}/episodes/{ep_id}/download` | Move the stored copy to the trash (§4d); the episode stays listed with `has_local: false`. Subscriber, or a family admin when the feed is shared. Returns the episode. |
| GET | `/podcasts/{id}/episodes/{ep_id}/stream` | `{url, expires_in_secs}` — presigned S3 URL, **requires prior `/download`**. Before the download has finished: `409 {"error": "episode_not_downloaded"}` (it was a `500` before 1.0.0-alpha.5); play the episode's `audio_url` meanwhile. |
| GET | `/podcasts/{id}/episodes/{ep_id}/image` | Proxied episode artwork. |

Background refresh also happens via the `feed_refresh` job type (see §8),
but there is **no push mechanism** to tell a mobile client "new episode
available" — clients must poll `/episodes` or trigger `/refresh` themselves.

## 6. Audiobooks

Base path `/audiobooks`.

| Method | Path | Purpose |
|---|---|---|
| GET / POST | `/` | List / create a book (metadata-only create). |
| POST | `/upload` | Multipart: `title, author, narrator, description, manifest (JSON array of {relative_path, duration_secs}), files[], cover` — creates a book plus all its audio files in one call, sorted by manifest order. **Subject to the deployment's request-body limit — see §6a.** |
| POST | `/from-uploads` | JSON twin of `/upload` for files already PUT straight to storage — see §6a. |
| GET/PUT/DELETE | `/{id}` | Book detail / update / delete. DELETE moves it to the trash (§4d). |
| GET | `/{id}/cover`, POST `/{id}/upload-cover` | Cover art proxy / upload. |
| GET | `/{id}/files` | List a book's files (position, title, duration). |
| PUT | `/{id}/files/{file_id}` | `{title}` — rename one file. Owner, or a family admin when the book is shared with their family (same rule as `/metadata/apply`). 400 on an empty title. |
| POST | `/{id}/upload-file` | Add/replace one file at a given `position` (multipart). |
| GET | `/{id}/files/{file_id}/stream` | `{url, expires_in_secs}` presigned S3 URL. |
| PUT | `/{id}/files/reorder` | `{file_ids: [...]}` — reorder multi-file books. |
| POST | `/{id}/metadata/search` | Google Books candidates for this book — see §6b. |
| POST | `/{id}/metadata/apply` | Write a chosen match onto the book — see §6b. |
| GET | `/{id}/chapters` | List chapters (**read-only** — no chapter creation/editing endpoint, and chapters are not auto-extracted from embedded M4B/ID3 metadata; they must be populated some other way, currently unclear from the router which write path exists). |

## 6b. Identify a book — Google Books

`POST /audiobooks/{id}/metadata/search` — body `{title?, author?, limit?}`, at
least one of title/author or the server answers 400. The book id scopes
visibility only; the search runs on what the client sends, so seed the fields
from the book and let the user edit them before searching — a book worth
identifying usually has the wrong title stored. Any viewer may search.

Returns candidates ordered by our own 0–100 `score` (Google returns relevance
order but no score):

```json
[{"volume_id":"zyTCAlFPjgYC","title":"Dune","subtitle":null,"author":"Frank Herbert",
  "publisher":"Ace","published_year":1965,"description":"…","isbn":"9780441013593",
  "page_count":896,"categories":["Fiction"],"cover_url":"https://…","score":100}]
```

`description` is plain text, cleaned server-side: Google returns web copy, and the
tags, HTML entities and soft hyphens are stripped before it leaves here. Paragraphs
are separated by a blank line, so render it with the newlines preserved. It is not
truncated — a publisher's blurb runs to a couple of thousand characters, so clamp it
in the UI rather than assuming a short string. `page_count` is absent, never `0`,
when Google does not know it.

`POST /audiobooks/{id}/metadata/apply` — body
`{volume_id, fields?: {title, author, description, publisher, published_year,
isbn, cover}}`, all booleans defaulting to `true`. Owner only; `cover: true`
additionally needs upload permission. Only the volume id travels — the server
re-fetches the volume rather than trusting a client's copy of a candidate — and
an unpicked field keeps its current value rather than being cleared. Returns the
updated book.

`BookResponse` gained `google_books_volume_id`, `isbn`, `publisher` and
`published_year`; `google_books_volume_id` non-null is what "already identified"
means.

**`narrator` is never written by an apply.** Google Books describes the print
edition: no narrator, no runtime, no chapters, no series position.

## 6a. Direct-to-storage uploads — `/uploads`

The multipart routes send the bytes through the API container. In the hosted
deployment the API is reached through a proxy that **rejects request bodies
over 100 MB**, so `/upload`, `/upload-file` and `/upload-cover` fail with a
413 for anything larger. They still work for small files and on a local
instance.

The direct path avoids the API entirely for the bytes:

| Method | Path | Purpose |
|---|---|---|
| POST | `/uploads/presign` | `{kind, filename, content_type, size_bytes?}` → `{object_key, url, method, content_type, expires_in_secs}`. `kind` is one of `audiobook_file`, `audiobook_cover`, `music_track`, `music_cover`, `companion_file` (§8f). |
| POST | `/uploads/complete` | `{object_key}` → `{media_object_id, object_key, content_type, size_bytes}`. Confirms the object exists and registers it. |

Flow for a book:

1. `POST /uploads/presign` once per file (and once for the cover).
2. `PUT` the file to the returned `url` **with the exact `Content-Type` the
   response carries** — it is part of the signature, and any other value
   fails as a signature mismatch. No `Authorization` header on this request;
   the URL carries its own credentials.
3. `POST /audiobooks/from-uploads` with the collected keys:

```json
{
  "title": "…", "author": "…", "narrator": "…", "description": "…",
  "visibility": "private",
  "cover_object_key": "f/…/uploads/audiobook-covers/…/cover.jpg",
  "files": [
    { "object_key": "f/…/uploads/audiobooks/…/01.m4a",
      "relative_path": "Book/01.m4a", "title": "Chapter 1", "duration_secs": 1830 }
  ]
}
```

Notes that have already caused bugs elsewhere:

- **Play order comes from `relative_path`**, not array order — the server
  sorts, exactly as it does for the multipart manifest.
- Keys are minted server-side under the caller's family prefix. Sending a key
  from outside it is refused with the API's generic not-found error — which,
  as everywhere else in this codebase, is rendered as **401** with
  `{"error":"account not found"}`, not 404. Don't treat that particular 401 as
  "session expired."
- `from-uploads` re-checks each object exists in storage before creating the
  book, so calling `/uploads/complete` first is optional for this flow. Use
  `complete` when you need a `media_object_id` on its own.
- Presigned PUT URLs are valid 6 hours; a single object may not exceed 5 GiB
  (no multipart upload support yet).
- A track goes the same way: presign with kind `music_track`, then
  `POST /music/tracks/from-upload` (§7).
- **More files for a book you own**: `POST /audiobooks/{id}/files/from-uploads`
  `{files:[{object_key, relative_path, title?, duration_secs?}]}` → `[{id, relative_path}]`.
  `relative_path` is required and kept; the book's play order is sorted by path again. A path
  the book already has (any case) is not added twice — its existing file is returned, so a
  retried upload is safe. Owner only.
- **`path`** (optional, on both) places the item in the own.audio folder
  (§8f): the book's folder under `Audiobooks/`, with each file's
  `relative_path` then kept as its name inside it; a track's file under
  `Music/`. A client syncing a folder sends it; everyone else leaves it out
  and the server picks a default. The response carries the final `path` —
  with ` (2)` when another of the owner's items already holds it. A single
  loose book file: `path` is the file and its `relative_path` is left out.

### Authors & tags — `/audiobooks/authors`

Full CRUD for authors (`name, sort_name, bio`, image via `image_object_id`),
many-to-many book↔author links with a `role` (author/narrator/etc.), and a
tag system (`GET/PUT /tags/book/{book_id}` — tags are get-or-create by name).

### Collections, series, favorites — `/audiobooks/organize`

- `/collections` — CRUD, `is_public` flag, add/remove books.
- `/series` — CRUD, add/remove books with a fractional `position` (for
  reordering/inserting between existing entries).
- `/favorites` — add/remove/check/list, independent of collections.

This subset already covers most of Audiobookshelf's organizational model.
What's missing is the **ingestion** side: no folder scan, no external
metadata-provider matching, no e-book files.

## 7. Music

Base path `/music`.

| Method | Path | Purpose |
|---|---|---|
| GET | `/tracks` | List the user's tracks. |
| POST | `/tracks/upload` | Multipart: `title, artist, album, album_artist, genre, track_number, duration_secs, file, cover`. Fields left out are filled from the file's tags, incl. album artist (`TPE2`/`aART`/`ALBUMARTIST`) and the compilation flag. Needs the upload permission (`403` without it). |
| POST | `/tracks/from-upload` | The presigned twin of `/tracks/upload` (§6a), for files over 100 MB: `{object_key, original_filename, path?, visibility?, title?, artist?, album?, album_artist?, genre?, track_number?, duration_secs?}` → the track plus `path` (§8f). Tags are read from the stored file exactly as for the multipart route. A file without a disc tag in a folder named `CD 2`/`Disc 2` is on disc 2. |
| GET/PUT/DELETE | `/tracks/{id}` | Track detail / update / delete. DELETE moves it to the trash (§4d). |
| POST | `/tracks/discs` | Reads the disc number of up to 20 of the caller's tracks uploaded before it was kept → `{checked, left}`; call again until `left` is 0. Only the disc fields change. |
| GET | `/tracks/{id}/stream` | `{url, expires_in_secs}` presigned URL. |
| GET | `/tracks/{id}/cover`, POST `/tracks/{id}/upload-cover` | Cover proxy / upload. |
| GET/PUT | `/tracks/{id}/progress` | Playback position for a track (yes — music has its own progress table, separate from `/playback`). |
| GET/POST | `/playlists` | List / create. Create takes `{name, description?, visibility?, generated?, track_ids?}`: `track_ids` fills it in order in the same request (up to 1000; ids the caller may not play are skipped), `generated: true` marks a playlist saved from the smart-playlist generator. Playlists carry `generated_at` and, once kept, `kept_at` (both absent on ordinary ones). |
| GET | `/starred` | Ids of the songs the caller starred. |
| PUT/DELETE | `/tracks/{id}/star` | Star / unstar a song the caller can play (idempotent). The same stars Subsonic's `star` sets; the "Forgotten favourites" smart playlist reads them. |
| GET | `/playlists/{id}/audience` | Owner only: every other family member with `can_listen` (can play the playlist now) and `locked` (a family admin, who sees anything shared with the family). |
| PUT | `/playlists/{id}/share` | Owner only. `{mode: "live"|"copy", user_ids}`. `live`: the chosen members play the owner's playlist and see every change (others in the family get a deny grant; nobody chosen makes it private again). `copy`: each chosen member gets a playlist of their own with the same songs. Either way the owner's private songs in it are first shared with the chosen members only. Each newly reached member gets a `playlist_shared` notification (`data: {playlist_id, from, name, mode}`). Returns `{shared_tracks, copies}`. |
| PUT | `/playlists/{id}/keep` | The owner keeps a generated playlist (sets `kept_at`); 404 if it is not theirs or not generated. |
| GET/PUT/DELETE | `/playlists/{id}` | Detail / rename / delete. DELETE moves it to the trash (§4d). |
| GET/POST | `/playlists/{id}/tracks` | List entries / add a track. |
| DELETE | `/playlists/{id}/tracks/{entry_id}` | Remove one entry. |
| PUT | `/playlists/{id}/tracks/reorder` | `{entry_ids: [...]}` full reorder. |
| POST | `/tracks/{id}/metadata/search` | `{title?, artist?, album?, limit?}` → MusicBrainz recording candidates. |
| POST | `/tracks/{id}/metadata/apply` | `{mb_recording_id, mb_release_id?, fetch_cover?}` → updated track. |

**Album artist.** Every track carries `album_artist`: the explicit value
(file tag, a compilation → `"Various Artists"`, a manual edit) or, when none
is set, `artist` without its guests (`feat.`/`ft.`/`featuring …` cut, `&`
kept). `/albums` and `/artists` group by it, so an album summary's `artist` is
the album artist (`/artists` also lists artists who only appear on another
artist's album, with `album_count` 0); an album's tracks are those whose `album_artist` and `album`
match. `PUT /tracks/{id}` takes optional `album_artist` — absent keeps it,
`null`/`""` clears it back to derived. `GET /tracks/{id}/metadata/file-tags`
also returns `album_artist` and `is_compilation`. `metadata/apply` sets it to
the MusicBrainz release's artist credit (a hits compilation → "Various
Artists"); the `music_release_backfill` job does the same for tracks
identified before that. See `docs/album-artist-plan.md`.

Playlists are static, manually-curated lists only — no smart/rule-based
playlists, no "recently added"/"most played" auto-playlists.

**Metadata search/apply.** Backend-side MusicBrainz + Cover Art Archive
client (`src/metadata/`) — no client ever calls MusicBrainz directly, so
there's one place holding the user-agent and the ~1 req/s rate limit
(process-wide, not per-user). `search` requires at least one of
`title`/`artist`/`album` (400 otherwise) and returns candidates carrying an
**unverified** speculative `cover_art_url` (may 404). `apply` re-fetches the
chosen recording from MusicBrainz by id — it does not trust client-supplied
copies of a search result — writes `title`/`artist`/`album`/`genre`/
`track_number` plus three new nullable columns
(`musicbrainz_recording_id`/`_release_id`/`_artist_id`) on `music_tracks`,
and best-effort fetches + stores cover art (a failed cover fetch never fails
the apply). Owner-only, same auth as `PUT /tracks/{id}`.

### OpenSubsonic-compatible surface — `/rest` (music only)

A sibling of `/api/v1`, for interoperating with existing Subsonic-ecosystem
apps (DSub, Symfonium, Tempo, play:Sub, ultrasonic, etc.) instead of, or in
addition to, a bespoke client. Auth is `u` + (`t`+`s` token, or legacy `p`
password) verified against the user's Subsonic API key from §3 — **never**
the real account password.

Implemented: `ping`, `getLicense`, `getOpenSubsonicExtensions`,
`getMusicFolders`, `getArtists`, `getIndexes`, `getArtist`, `getAlbumList2`,
`getAlbum`, `getSong`, `search3`, `stream`, `download`, `getCoverArt`,
`scrobble` (updates local progress only — does **not** relay to Last.fm/
ListenBrainz), `getPlaylists`, `getPlaylist`, `createPlaylist`,
`updatePlaylist`, `deletePlaylist`. Every route also has a `.view` alias.

**Not implemented** (would matter for a full-featured Subsonic client, or
for reusing this API from an Android app via an existing Subsonic SDK):
`getGenres`, `getSongsByGenre`, `getRandomSongs`, `getTopSongs`,
`getSimilarSongs`, `getNowPlaying`, `star`/`unstar`/`setRating`/
`getStarred2`, `savePlayQueue`/`getPlayQueue`, `getShares`/`createShare`,
`getScanStatus`/`startScan`, podcast endpoints, `getUser`/`getUsers`,
`changePassword`, `getLyrics`.

**For a from-scratch mobile app it's simplest to build against `/api/v1`
directly** and treat `/rest` purely as an interop escape hatch for
third-party Subsonic clients, not as the primary mobile API — `/api/v1`
covers audiobooks and podcasts, which `/rest` deliberately does not.

## 8. Playback (progress, bookmarks, settings)

Base path `/playback`. Note: **music tracks use their own progress
endpoints** under `/music/tracks/{id}/progress` (§6) — this module only
covers podcast episodes and audiobooks.

| Method | Path | Purpose |
|---|---|---|
| GET/PUT | `/episodes/{episode_id}/progress` | `{position_secs, completed}`. **GET answers `404` when nothing is saved yet** — that is the normal first-play case, not an error. |
| GET | `/books/progress` | Every book this user has started, in one call — `{book_id, position_secs, file_id, file_position_secs, completed, updated_at}`. `position_secs` is **book-wide** (file offsets already summed server-side, same as `/library/continue`); `file_position_secs` is the raw per-file value a player seeks to. The only way a client sees progress made on another device: `/library/changes` carries none. |
| GET/PUT | `/books/{book_id}/progress` | `{position_secs, completed, file_id}` — `file_id` is required on PUT since a book can span multiple files; the position is scoped to whichever file is currently playing. **GET answers `404` when nothing is saved yet** (until 2026-08-27 this was a `401`, which read as an expired session). |
| GET/POST | `/bookmarks` | List all bookmarks / create one (`book_id` or `episode_id`, `file_id`, `position_secs`, `label`). |
| PUT/DELETE | `/bookmarks/{id}` | Update label / delete. |
| GET | `/books/{book_id}/bookmarks` | Bookmarks scoped to one book. |
| GET | `/settings` | Global playback prefs: `playback_speed`, `skip_intro_secs`, `skip_outro_secs`, `ab_skip_forward_secs`, `ab_skip_backward_secs`, `ab_playback_speed`. |
| PUT | `/settings/audiobook-defaults` | Update the `ab_*` audiobook-specific defaults (skip amounts clamped 1–120s, speed clamped 0.5–3.0×). |

There is no cross-device **play queue** endpoint (a "what's next" queue
synced server-side), and no listening-session/history log beyond the single
latest-position row per item — see gap analysis for the stats implication.

## 8a. Listening history & statistics

Progress rows hold only the latest position, so a separate append-only
session log backs the statistics board.

| Method | Path | Purpose |
|---|---|---|
| POST | `/playback/sessions` | Report listening spans. Batched (≤500) and idempotent. |
| GET | `/stats/me?range=&tz_offset_minutes=` | Totals, per-kind split, per-day series (`by_day`, and `by_day_kind` split by kind), streak, top items. `range` ∈ `7d\|30d\|90d\|365d\|all` (default `30d`). |
| GET | `/stats/me/history?limit=&offset=` | Raw session log, newest first. |
| PUT | `/stats/me/visibility` | `{stats_visibility}` — `private` (default) or `family_admin`. |
| GET | `/stats/family` | Per-member roll-up (family_admin). Members who kept their stats private appear as `hidden: true` with no figures. |
| PUT | `/stats/family/members/{user_id}/visibility` | Family admin may enable stats for a **restricted** member only (one carrying a `deny_all` policy); 403 otherwise. |

### Reporting sessions from a mobile client

```jsonc
POST /api/v1/playback/sessions
{ "sessions": [ {
    "media_kind": "audiobook",       // audiobook | podcast | music
    "item_id": "<book/feed/track id>",
    "part_id": "<file or episode id>",   // optional
    "started_at": "2026-07-18T09:00:00Z",
    "ended_at":   "2026-07-18T09:12:30Z",
    "seconds_listened": 720,          // audio consumed, not wall clock
    "playback_speed": 1.5,            // optional
    "device_kind": "android",         // web|ios|android|macos|windows|subsonic, else other
    "client_session_id": "uuid-v4"    // SEND THIS
} ] }
```

**`client_session_id` is what makes retries safe.** Android WorkManager and
iOS background tasks both re-run a failed job, and a batch that was actually
stored before the response was lost would otherwise be counted twice.
Generate the id when the span closes, persist it locally with the span, and
reuse it on every retry. The response reports `recorded` (rows newly stored)
vs `received`; `recorded: 0` on a retry is success, not an error.

Batch aggressively rather than posting per span: a device in Doze or offline
should accumulate spans and flush them in one request when it next has
connectivity.

**If a client reports no sessions at all**, the server derives them from
progress saves instead, so a minimal client still produces statistics. Send
`device_kind` on `PUT …/progress` to attribute those correctly. Once a client
starts reporting explicitly, derivation switches off for that user, so the
two never double-count — do not mix the two strategies within one client.

**Timezones**: send `tz_offset_minutes` (minutes east of UTC) on stats
requests so day buckets and the streak match the listener's local day.
Totals are unaffected by it.

## 8b. Cross-device play queue

| Method | Path | Purpose |
|---|---|---|
| GET | `/playback/queue` | The caller's queue. An empty queue returns a body, never 404. |
| PUT | `/playback/queue` | `{items[], current_index, position_secs, device_kind}` — replaces the queue wholesale. |

`items` are `{media_kind, item_id, part_id?}`. Semantics are last-write-wins:
a queue is small and always edited as a unit, so merging concurrent edits
would invent an order neither device asked for. The response echoes
`updated_at` and `updated_by_device`, which is enough to notice you were
overtaken ("now playing on Pixel 9") and re-read before writing again. An
out-of-range `current_index` is clamped rather than rejected.

The Subsonic surface (`savePlayQueue` / `getPlayQueue`) reads and writes the
same row, so a third-party Subsonic app and the native clients share one queue.

## 8c. Incremental sync

`GET /library/changes?since=<rfc3339>`

One request returns everything a cached client needs to catch up:

```jsonc
{
  "since": "2026-07-19T08:00:00.000Z",   // echo, null on a full sync
  "now":   "2026-07-19T09:15:22.481Z",   // pass back as `since` next time
  "full_sync": false,
  "audiobooks": [ /* changed rows */ ],
  "podcasts":   [ /* … */ ],
  "tracks":     [ /* … */ ],
  "deleted":    [ { "media_kind": "music", "item_id": "…", "deleted_at": "…" } ]
}
```

- **Omit `since` for a first run** — you get a full snapshot with
  `full_sync: true` and no tombstones, so there is no separate bootstrap path.
- **`deleted` is why this exists.** A row that was removed is simply absent
  from a delta and indistinguishable from one you already hold; tombstones let
  you purge instead of keeping ghosts. They are pruned after **180 days**, so a
  client offline longer than that must drop its cache and do a full sync.
- **`now` is captured before the queries run**, so a row written mid-request
  is re-sent next time rather than falling through the gap. Expect occasional
  repeats and treat the sync as idempotent.
- Cursors are handed out in `Z` form on purpose: an RFC3339 `+00:00` offset
  becomes a space when placed in a query string unencoded. Echo `now` back
  verbatim and you are safe; if you build a cursor yourself, URL-encode it.

## 8d. Push notifications and the inbox

| Method | Path | Purpose |
|---|---|---|
| POST | `/devices/push-token` | `{platform, token, device_name?}` — `platform` is `apns` (iOS) or `fcm` (Android). |
| DELETE | `/devices/push-token` | `{token}` — call on sign-out. |
| GET | `/devices/push-tokens` | Registered devices; only a token suffix is echoed back. |
| GET | `/devices/notifications` | Undelivered notifications for this user. |
| POST | `/devices/notifications/ack` | `{ids[]}` — mark handled so they stop returning. |

Register on **every app start**: tokens rotate on both platforms, and
re-registering also refreshes `last_seen_at`. A token is globally unique — 
re-registering one that belonged to another account moves it, so handing a
device on does not keep delivering the previous owner's notifications.

> **Delivery is not implemented yet.** Registration, the queue, and the
> triggers (a feed refresh finding new episodes notifies the owner and every
> family member allowed to play that feed) all work, but no APNs/FCM sender
> ships today. **Poll `GET /devices/notifications` on resume** — it carries
> the same information, and entries survive until acknowledged, so nothing is
> lost if the app is killed. When a sender is added, no client change is
> needed.

## 8e. Bulk operations

| Method | Path | Purpose |
|---|---|---|
| POST | `/playback/episodes/progress/bulk` | `{episode_ids[], completed}` — mark many episodes played/unplayed (≤1000). |

Multi-select in a mobile UI should never fan out into N round trips over
cellular.

## 8f. File sync — the own.audio folder

docs/file-sync-plan.md §5. For clients that mirror the library as files (the
Mac's Finder extension; later a Docker agent and Windows). Phone apps do not
need any of this.

**Paths.** Every book, track and stored episode has a `path` in the folder:
`Audiobooks/…` (a book's folder), `Music/…` (a file), `Podcasts/<Show>/…`.
The path belongs to the user — a file put in the folder keeps its path and
name exactly; an item created elsewhere gets a default **once**
(`Audiobooks/<Author>/<Title>/<NN> - <File title>.<ext>`,
`Music/<Album artist>/<Album>/<NN> - <Title>.<ext>`,
`Podcasts/<Show>/<YYYY-MM-DD> - <Episode>.<ext>`). No edit, Identify or
retitle ever changes it. Paths are unique per owner, case-insensitively, among
live items; a clash gets ` (2)`. Stored NFC. A restored item whose path was
taken meanwhile comes back as ` (2)`.

| Method | Path | Purpose |
|---|---|---|
| GET | `/sync/tree?cursor=&limit=` | The folder's contents. No cursor: a full snapshot, paged (`limit` ≤ 2000, default 500). Then only changes. Pass `cursor` back verbatim; keep calling while `has_more`. |
| GET | `/sync/tree/ids` | `[{kind, id, updated_at}]` for everything in the caller's tree — for a periodic full check (on start and every few hours) that drops what the server no longer shows. |
| GET | `/sync/shortcuts` | The caller's family shortcuts. |
| POST | `/sync/shortcuts` | `{member_id?, kind, container?}` — `member_id` may be left out with a container: the book's, show's or album's owner is found — `kind` `audiobook`/`music`/`podcast`; `container` `{kind:"book"\|"show", id}` or `{kind:"album", release_group}` or `{kind:"album", album_artist?, album}`. `201` new, `200` when it existed. Yourself → `400`; outside the family, or nothing visible → `404`. |
| DELETE | `/sync/shortcuts/{id}` | `204`. |
| POST | `/sync/paths/organise` | "Organise" the caller's **own** items: `{kind: "music"\|"audiobook", preview?, ids?}` → `[{kind, id, title, from, to, companions}]`, the items moved to their default paths (`Music/<Album artist>/<Album>/<NN> - <Title>.<ext>`, the file's own extension kept; `Audiobooks/<Author>/<Title>`). `preview: true` changes nothing and lists exactly what applying would do, ` (2)` suffixes included; `ids` applies only those items. Companion files move with their book, and with an album's tracks when the whole folder moves to one place. Identifiers never change; the tree feed carries the new paths. |
| PUT | `/sync/holdings` | What **this device** keeps offline: `{items:[{kind,id}]}` replaces the set, `{added, removed}` changes it. `204`. The device is the session's refresh chain; a session without one gets `400`. |
| GET | `/sync/holdings?kind=&id=` | The caller's **own** devices holding that item: `[{chain_id, device_name, device_kind, since, current}]`. Without a query: this device's set. Signing a device out drops its rows. |
| POST | `/sync/files` | A companion file: `{object_key, path, visibility?}` (presign kind `companion_file`) → `{id, path, visibility, size_bytes, used_as}`. See below. |
| GET | `/sync/files/{id}/stream` | `{url, expires_in_secs}`, presigned, honours `Range`. |
| PUT | `/sync/files/{id}/visibility` | `{visibility}` — owner only. `204`. |
| DELETE | `/sync/files/{id}` | To the trash (§4d), owner or family admin for a shared file. |
| POST | `/sync/files/{id}/use-as-cover` | Owner: this image becomes the cover of the book whose folder it is in and of every track in its folder, replacing theirs → `{book, tracks}`. |

**Companion files** (plan §2 item 16): images (`jpg jpeg png webp gif`),
`pdf`, `lrc`, `cue`, `txt`, `nfo` kept next to music or in a book folder,
under `Music/` or `Audiobooks/` (never `Podcasts/`); other types are `400`.
They are never changed. They are items of kind `companion_file` in
`/sync/tree` (one file with `relative_path: ""`) and `/sync/tree/ids`; a
member sees a shared one under the member policy of the kind whose folder it
is in. `used_as` says what the server did with it: `cover` — an image called
`cover`/`folder`/`front` (or the only image) gave its cover to the book whose
folder it is in and to the tracks in its folder that had none (embedded art
wins); `lyrics` — a `.lrc` named like a track in the same folder became its
lyrics. A companion file may sit inside a book's folder; nesting between
items is still refused.

**The caller's shared side** (plan §2 item 17): every `/sync/tree` page
carries `me: {id, display_name}` — the name of the `Family/<Me>` folder a
sync client shows the caller's shared items under. The server path of a
shared item is the same as a private one's; where it is shown is the client's
decision.

`/sync/tree` response:

```jsonc
{
  "cursor": "…", "has_more": false,
  "reset": false,          // true: the old cursor was too old — rebuild from the pages that follow
  "items": [{
    "kind": "audiobook",   // | music_track | podcast_episode
    "id": "…", "updated_at": "…",
    "owner": {"id": "…", "display_name": "Petr"},
    "is_owner": false, "can_delete": true, "shared_with_family": true,
    "title": "Mort",       // for messages only
    "path": "Audiobooks/Terry Pratchett/Mort",
    "files": [{"id": "…", "relative_path": "01 - Chapter 1.mp3", "size_bytes": 123, "sha256": "…|null"}],
    "show":  {"id": "…", "title": "…"},                              // episodes only
    "album": {"title": "…", "album_artist": "…", "release_group": "…|null"}  // tracks only
  }],
  "removed": [{"kind": "music_track", "id": "…", "reason": "trashed"}]  // | deleted | hidden
}
```

- A track or episode has one file with `relative_path: ""` — the path is the
  file. So does a book made of one loose file.
- Download each file through the existing stream routes, which return a
  presigned `GET` that honours `Range`: `/music/tracks/{id}/stream`,
  `/audiobooks/{id}/files/{file_id}/stream`,
  `/podcasts/{show.id}/episodes/{id}/stream`. Verify against `sha256` when
  present, by size until the checksum job has filled it.
- `removed.reason`: `trashed` — in the trash, may come back; `deleted` — gone
  for good, or an episode no longer stored; `hidden` — still exists but no
  longer visible to the caller (unshared, access withdrawn).
- Treat the feed as idempotent: an item can arrive twice. The cursor is taken
  from the oldest transaction still running, so a change that commits late
  (a long upload) is never skipped. A cursor older than 180 days answers with
  `reset: true` and a fresh snapshot. A malformed cursor is `400`.
- The feed sends every visible item, family items included; **which family
  items go into `Family/` is the client's decision**, from the shortcuts.
  `can_delete` is the owner, or a family admin for a shared item.
- Per-member policy changes (a member denied all music) do not produce
  `removed` rows; the periodic `/sync/tree/ids` check catches them.

## 9. Jobs (admin-only visibility)

| Method | Path | Purpose |
|---|---|---|
| GET | `/jobs/` | Last 50 jobs (admin only). |
| GET | `/jobs/{id}` | Job detail (admin only). |

Only one job type actually runs today: `feed_refresh` (podcast polling).
Not useful for an end-user mobile client beyond an admin diagnostics screen.

## 10. Media streaming model

Every stream/download endpoint (`music/tracks/{id}/stream`,
`audiobooks/{id}/files/{file_id}/stream`, `podcasts/{id}/episodes/{ep_id}/stream`)
returns `{ "url": "<presigned S3 URL>", "expires_in_secs": 14400 }` rather
than proxying bytes itself. Implications for a mobile client:

- **No range-request negotiation through the backend** — seeking behavior,
  byte-range support, etc. are whatever Garage/S3 provides on the presigned
  URL, not something audio2 controls.
- URLs expire after 4 hours; a long-running background download or a paused
  multi-hour audiobook session needs to **re-request the stream endpoint**
  to get a fresh URL, not just retry the same one.
- There is **no adaptive bitrate / transcoding** — the original uploaded
  file is always what's served. A mobile client on a poor connection has no
  lower-quality fallback to request.
- Downloaded podcast episodes are a separate concept from "cached for
  offline" — `/download` pulls the file server-side into S3 permanently
  (shared across the user's devices), it does not mean "downloaded to this
  phone." A mobile app still needs its own on-device offline cache/download
  manager on top of this.

## 11. What's missing for a mobile client specifically

These are gaps that matter more for phone apps than for the existing web
frontend, distinct from the general ABS/Navidrome feature gaps already
tracked in [backend-gap-analysis-abs-navidrome.md](backend-gap-analysis-abs-navidrome.md):

| Gap | Impact |
|---|---|
| ~~No refresh token / silent re-auth~~ | **Closed 2026-07-18** — `POST /auth/refresh` with rotation + reuse detection. |
| ~~No push notification hooks~~ | **Partly closed 2026-07-19** — registration + notification inbox exist; actual APNs/FCM *delivery* is still unimplemented, so clients poll the inbox. |
| ~~No server-side play queue~~ | **Closed 2026-07-19** — `GET\|PUT /playback/queue`, shared with Subsonic. |
| ~~No listening history / stats~~ | **Closed 2026-07-18** — `/playback/sessions` plus the `/stats/*` board. |
| ~~No per-device/session listing~~ | **Closed 2026-07-18** — `GET /auth/sessions` + `DELETE /auth/sessions/{chain_id}`. |
| ~~No incremental/delta sync endpoint~~ | **Closed 2026-07-19** — `GET /library/changes?since=` with deletion tombstones. |
| No background-download-friendly episode auto-download rule (only manual `/download`) | App-side WorkManager job has to poll `/episodes` itself to know what to fetch. |
| ~~No bulk endpoints~~ | **Partly closed 2026-07-19** — bulk episode played/unplayed exists; bulk delete does not. |
| No ETag/If-None-Match on list endpoints | Largely obviated by `/library/changes`, which answers "what changed" directly. |
| OIDC login stubs only | If the deployment relies on Google/Microsoft login, mobile can't use it yet — local email/password is the only working path. |

None of these block building a v1 Android app — they're the difference
between "works" and "feels first-party/production-grade" once one exists.

---

## 12. Mandatory feature checklist — AAA-class all-in-one Android app

This is a target feature list for a single Android app that plays
**audiobooks + podcasts + music** against this backend, calibrated to the
bar set by Audiobookshelf's companion app + Navidrome-ecosystem apps
(Symfonium/DSub) + mainstream podcast apps (Pocket Casts/Overcast) combined.
Grouped so it's clear which items the current backend already supports vs.
which require backend work from §11/gap-analysis first.

### 12.1 Playback engine (client-side, backend-agnostic)
- [ ] Background playback service (Android `MediaSessionService` /
      Media3/ExoPlayer) surviving app-kill, with a persistent notification.
- [ ] Lock-screen / notification media controls (play, pause, skip ±N sec,
      chapter next/prev, playback speed).
- [ ] Bluetooth/headset button + car (Android Auto) support.
- [ ] Variable playback speed (0.5×–3.0×+) — **backend already stores a
      per-user default** (`ab_playback_speed`), client should read/write it.
- [ ] Sleep timer (end-of-chapter aware) — pure client-side, no backend
      dependency needed.
- [ ] Skip-silence / volume boost (client-side audio processing).
- [ ] Gapless playback across multi-file audiobooks (uses `/files` +
      `/files/reorder` ordering already exposed).
- [ ] Chapter-aware seek bar (uses `/audiobooks/{id}/chapters` — but see
      gap: chapters aren't auto-extracted server-side yet, so this is only
      as good as whatever populates that table today).
- [ ] Cross-fade / gapless between podcast episodes and playlist tracks.
- [ ] Automatic resume from last position on cold start (uses
      `/playback/books/{id}/progress`, `/playback/episodes/{id}/progress`,
      `/music/tracks/{id}/progress`).
- [ ] Bookmarks with labels, jump-to-bookmark UI (`/playback/bookmarks`
      already supports this fully).

### 12.2 Offline & sync
- [ ] On-device download manager (WorkManager) for audiobooks, episodes,
      and tracks, independent from the server-side "download episode to S3"
      concept — the app needs its **own** local cache/eviction policy.
- [ ] Storage-aware auto-cleanup (delete played episodes after N days,
      cap total offline storage).
- [ ] Auto-download new episodes per-podcast subscription rule
      ("download all new", "download none", "download first N") — **not
      backed by any server rule today; must poll `/episodes` client-side.**
- [ ] Play/pause/progress sync across the user's own devices — works today
      via `/playback/*/progress` as long as the client pushes updates
      frequently (e.g. every 10–30s and on pause/stop), not just at the end.
- [ ] Conflict resolution when the same book/episode was played offline on
      two devices before either synced (last-write-wins is acceptable v1).

### 12.3 Library browsing & organization
- [ ] Unified home screen: "Continue Listening" (`/library/continue`),
      "Recently Added" (needs a `created_at`-sorted view — already available
      client-side by sorting `/audiobooks`, `/podcasts/{id}/episodes`,
      `/music/tracks`).
- [ ] Global search across audiobooks, podcasts, **and music**
      (`/library/search` today omits music tracks — client must add a
      separate `/music/tracks` filter client-side, or backend should extend
      the endpoint).
- [ ] Audiobook library views: by author, by series (with position order),
      by collection, by tag, favorites — all backed by existing
      `/audiobooks/authors`, `/organize/series`, `/organize/collections`,
      `/organize/favorites`.
- [ ] Podcast subscription management: search & subscribe
      (`/podcasts/search`, `/podcasts/subscribe`), per-feed episode list
      with played/in-progress badges (already returned inline by
      `/podcasts/{id}/episodes`), manual refresh (`/podcasts/{id}/refresh`),
      unsubscribe.
- [x] Music library views: by artist/album/genre — first-class since
      2026-07-19: `GET /music/artists`, `/music/albums?artist=`,
      `/music/genres`, all scoped by the same visibility rules.
- [ ] Playlist management (create/rename/delete/reorder/add/remove) — fully
      supported by `/music/playlists/*`.

### 12.4 Uploading / library management from the phone
- [ ] Upload a track/audiobook file (or whole folder via SAF) straight from
      the phone — `multipart/form-data` to `/music/tracks/upload` and
      `/audiobooks/upload` already supports this; the upload flow (manifest
      construction, progress UI, retry-on-flaky-connection) is client work.
- [ ] Metadata editing (title/author/narrator/description/tags/cover) —
      fully supported (`PUT /audiobooks/{id}`, `/music/tracks/{id}`, tag and
      author endpoints).
- [ ] Add-to-collection / add-to-series / mark-favorite from a book's detail
      screen — fully supported.

### 12.5 Account & multi-user
- [ ] Login / registration screens gated by `/auth/registration-status`.
- [ ] First-run setup wizard for a fresh server (`/setup/status`,
      `/setup/complete`) — useful if the app can also *be* the way someone
      bootstraps a new self-hosted instance.
- [ ] Profile screen: display name edit, change password, delete account
      — fully supported by `/users/me`.
- [ ] Admin screens (user list, create user, revoke sessions) if the app
      targets self-hosters who want full admin from mobile — fully
      supported by `/users/*` for an `admin`-role account.
- [ ] "Log out this device only" vs "log out everywhere" — today only
      "everywhere" (`revoke-sessions`) and single-session logout
      (`/auth/logout`) exist; no per-device session list to choose from.

### 12.6 Notifications & background awareness
- [ ] Push notification on new episode for subscribed podcasts — **backend
      has no push registration endpoint or webhook**; would need FCM token
      registration + a server-side trigger from the `feed_refresh` job.
- [ ] Background periodic refresh of subscriptions (WorkManager periodic
      job hitting `/podcasts/{id}/refresh` or relying on server-side
      `feed_refresh`, whichever is authoritative — needs a policy decision).
- [ ] Download-complete / upload-complete notifications — pure client-side.

### 12.7 Android platform polish (client-side, no backend dependency)
- [ ] Android Auto / Wear OS companion support.
- [ ] Widgets (home-screen playback widget).
- [ ] Adaptive icon, dynamic color (Material You), dark theme.
- [ ] Predictive back gesture, edge-to-edge, foldable-aware layouts.
- [ ] Accessibility: TalkBack labels on all playback controls, scalable
      text, sufficient contrast.
- [ ] Chromecast support (would need the presigned stream URL to be
      reachable from the cast receiver, which it already is, being a plain
      HTTPS S3 URL).

### 12.8 Priority read for backend work
If picking backend work to unblock the mobile app fastest, in rough order
of leverage:
1. **Server-authoritative chapter extraction** for audiobooks (embedded
   M4B/ID3 chapter parsing on upload) — currently `/chapters` is read-only
   with no clear write path, so the seek-bar chapter feature has nothing to
   read.
2. **Grouped music browsing endpoints** (`/music/artists`, `/music/albums`)
   so the client doesn't have to reimplement Subsonic-style grouping logic
   against a flat track list.
3. **Refresh token / longer-lived session model** so the app doesn't force
   a full re-login weekly.
4. **Push notification registration + new-episode webhook/trigger** off the
   existing `feed_refresh` job.
5. **Unify `/library/search` to include music tracks.**
