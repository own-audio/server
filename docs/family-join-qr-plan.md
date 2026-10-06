# Frictionless Family Join & Leave — QR / link invites

Date: 2026-08-23
Status: **Phase A (backend) and Phase C (audio2-mac UI) done, 2026-08-23**
(C's URL-scheme deep link deliberately deferred — see C3). **Phase B (web
landing page) remains — and is now the blocker**: nothing lands a phone
camera scan anywhere until it exists. Every step names the file it touches
and the tests that prove it.
Related:
- [backend-family-implementation-plan.md](backend-family-implementation-plan.md)
  (Phase 1 built the invite machinery this plan extends)
- [family-billing-plan.md](family-billing-plan.md)
- [mobile-backend-api-spec.md](mobile-backend-api-spec.md) — append the new
  endpoints there when they land.

## Goal

Joining a family must feel like joining a WhatsApp group, not like being
provisioned into an LDAP directory. Two flows, both ending inside the family
in under a minute:

1. **Self-serve**: an admin shows a QR code (or sends a link). The member
   scans it with their phone camera, sees "*Join the Maráz family on audio2*",
   registers (or signs in), and is in. No email round-trip required.
2. **Admin-registers**: the admin creates the account for the member (a kid, a
   grandparent), then hands them a QR code. Scanning it asks for **one thing
   only — a password** — and signs them in. Name, role, and identifier were
   already set by the admin.

Leaving must be equally clear: one button, one confirmation that says exactly
what happens (your shared items go private, you get your own fresh family).

## What exists today (do not rebuild)

Backend (`audio2/backend`, all shipped and smoke-tested):

- `family_invites` table (migration `0018_families.sql`): email-bound bearer
  `code` (24 hex chars = 96 bits, generated in
  `families/mod.rs::generate_invite_code`), single-use via
  `accepted_at`/`accepted_by`, 14-day TTL, role `family_admin|member`.
- `POST /api/v1/family/invites` (admin), `GET /invites`, `DELETE /invites/{id}`,
  `POST /invites/accept` (authenticated, email must match).
- **Invite-aware registration**: `POST /auth/register` takes `invite_code`,
  works even when open registration is off, drops the account into the family.
- `DELETE /api/v1/family/members/{user_id}` targeting yourself **is** "leave
  family" — re-homes you into a fresh personal family, reverts your shared
  items to private, clears grants. Server side of "leave" is done.

Clients:

- `audio2-mac` `Features/FeatureFamily`: members list, invite-by-email dialog,
  `InviteShareView` (shows the bare code as text), paste-a-code join alert.
  No QR anywhere, no URL handling.
- Web console (`audio2/frontend`): **no invite/join UI at all** yet.

Constraint discovered while planning: `users.email` is `NOT NULL UNIQUE`
(migration `0002_users_auth.sql`) and login is by email. The admin-registers
flow must work for members with no email address — see decision D4.

## Competitor survey (what "best in category" means)

| Product | Join flow | Verdict |
|---|---|---|
| **Plex Home** | Invite by email/username from a settings screen; recipient accepts in their account, can create an account during acceptance; per-member library restrictions. Capped by Plex Pass tier. | Solid but email-centric; no link/QR; acceptance lives in a settings page people can't find. |
| **Wizarr** (3rd-party onboarding for Plex/Jellyfin/Emby) | Admin generates a **shareable invite link** (optionally multi-use, expiring); opening it walks the user through account creation and app install. The community built this because the servers don't have it — proof of demand. | The model to beat: link-first, self-serve, guided landing page. We add QR + native clients on top. |
| **Jellyfin Quick Connect** | 6-char code authorizes a *device* login from an already-signed-in device. Not a membership flow, but the pattern for our future "sign in a new device by scanning" (out of scope here, noted in Phase D). | Great for TVs; steal later. |
| **Spotify Family** | Email link, then every member must type the **exact same home address** (same punctuation!), sometimes with location services on. | The cautionary tale. Verification friction people actively hate. We verify nothing beyond possession of the code. |
| **Apple Family Sharing** | Invite via Messages/AirDrop or "invite in person" (they sign in on your device); invitation card appears in Settings; 15-day expiry. | Smoothest big-tech flow; works because it's OS-integrated. The "in person" mode is our QR flow in spirit. |
| **Audiobookshelf / Navidrome** | Admin creates username + password by hand and tells the member their credentials out-of-band. No invites at all. | Our direct competitors, and the bar is on the floor. A claim-QR that lets the member set their *own* password beats sharing passwords over chat. |

What we adopt: link-first invites (Wizarr), an in-person QR mode (Apple), a
named landing page that says whose family you're joining (both), admin
pre-provisioning without password sharing (better than ABS/Navidrome), zero
verification beyond the code (anti-Spotify).

## Decisions (locked unless revisited)

- **D1 — The QR encodes an HTTPS URL, not a bare code**:
  `{server.app_base_url}/join/{code}`. A phone camera opens it in the browser
  and the web console renders the landing page — works with zero apps
  installed, on any platform. The code alone remains valid for manual paste in
  native clients. Either base URL (`config.rs::ServerConfig`, both optional) is
  required for QR display; when both are unset, clients build the URL from the
  origin they're connected to.

  **Corrected 2026-09-18:** this originally read `server.base_url`, and the
  hosted deployment sets that to `https://api.own.audio` — an origin that
  serves no pages, so every generated join link 404'd. `app_base_url` is the
  console's origin and falls back to `base_url`, which keeps single-origin
  self-hosted installs working unchanged.
- **D2 — Three invite kinds** in one table: `email` (today's flow, unchanged),
  `link` (email-free, optionally multi-use — the fridge QR), `claim`
  (bound to a pre-provisioned account). A `kind` column, not new tables.
- **D3 — Multi-use and admin-role never combine.** `link` invites are always
  role `member` and may have `max_uses` 1–20 (default 1). `email` and `claim`
  invites stay single-use; only `email` invites may carry `family_admin`.
  Handing out admin via a QR taped to the fridge must be impossible.
- **D4 — Provisioned members get a login identifier, not a fake mailbox.**
  Keep `users.email NOT NULL` (relaxing it ripples everywhere). The admin
  picks an email-shaped identifier for the member — the UI suggests
  `firstname@familyname.family` — and it is stored in `users.email` and used
  to sign in. The claim screen shows it prominently ("you'll sign in as
  `lena@maraz.family`"). If the member later has a real email, the normal
  change-email path applies. No mail is ever sent to it — `crate::mail` only
  mails `email`-kind invites (see A6.1 below).
- **D5 — TTLs**: `email` 14 days (unchanged), `link` 7 days, `claim` 30 days
  (accounts are often set up ahead of gifting a device). Every kind gets a
  "regenerate" action rather than long-lived codes.
- **D6 — The landing page may reveal family name + inviter display name to
  anyone holding the code.** The code is a 96-bit bearer secret; possession
  is the auth. Invalid and expired codes return distinct states (expired
  should tell the scanner to ask for a fresh one) but never reveal anything
  about the family.
- **D7 — No new verification.** No address checks, no email confirmation
  loops, no approval queues. Scan → (register | set password) → in.

---

## Phase A — Backend (audio2/backend)

### A1. Migration `00XX_family_join.sql` (next free number)

```sql
ALTER TABLE family_invites
    ALTER COLUMN email DROP NOT NULL,
    ADD COLUMN kind          TEXT NOT NULL DEFAULT 'email'
        CHECK (kind IN ('email', 'link', 'claim')),
    ADD COLUMN max_uses      INTEGER NOT NULL DEFAULT 1 CHECK (max_uses BETWEEN 1 AND 20),
    ADD COLUMN use_count     INTEGER NOT NULL DEFAULT 0,
    ADD COLUMN label         TEXT,
    ADD COLUMN claim_user_id UUID REFERENCES users (id) ON DELETE CASCADE;

-- kind-shape guards
ALTER TABLE family_invites ADD CONSTRAINT family_invites_kind_shape CHECK (
    (kind = 'email' AND email IS NOT NULL AND claim_user_id IS NULL AND max_uses = 1)
 OR (kind = 'link'  AND email IS NULL     AND claim_user_id IS NULL AND role = 'member')
 OR (kind = 'claim' AND email IS NULL     AND claim_user_id IS NOT NULL AND max_uses = 1)
);
```

Existing rows are all `kind='email'` with email set, so the default satisfies
the constraint. `use_count` supersedes `accepted_at` as the "is it spent"
check for `link` invites; keep writing `accepted_at`/`accepted_by` on the
*last* acceptance for audit.

- [x] Write the migration; `cargo sqlx` / test DB migrate cleanly.

### A2. `db/families.rs`

- [x] Extend `FamilyInvite` struct with the new columns
      (`email` becomes `Option<String>`).
- [x] `create_invite` grows `kind`, `max_uses`, `label`, `claim_user_id`
      params (or a `NewInvite` struct — pick whichever is less churn).
- [x] Replace the spent-check in `mark_invite_accepted` with an atomic
      claim: `UPDATE family_invites SET use_count = use_count + 1,
      accepted_at = now(), accepted_by = $2 WHERE id = $1 AND use_count <
      max_uses AND expires_at > now() RETURNING id` — returns false when
      exhausted/expired. Single-use kinds keep working because
      `max_uses = 1`.

### A3. Public join endpoints (`families/mod.rs` + `http/router.rs`)

New public (unauthenticated) router nested at `/api/v1/join`:

- [x] `GET /api/v1/join/{code}` → the landing-page preview:
      `{ kind, family_name, inviter_name, role, member_count, expires_at,
      uses_left, status: "valid"|"expired"|"exhausted",
      claim: { display_name, login_email } | null }`.
      `inviter_name` = `created_by` user's display_name (nullable).
      404 for unknown codes; the JSON `status` covers known-but-dead ones.
- [x] `POST /api/v1/join/{code}/claim` body `{ password }` — `claim` kind
      only. Atomically: redeem invite (A2 update), create the `local`
      `auth_identities` row with the argon2 hash (the provisioned user has
      none — that is what makes the account unclaimable-by-login until now),
      move nothing (the user was provisioned already inside the family — see
      A4), then mint and return the standard login response
      `{ token, refresh_token, user }` (reuse the issue-tokens path from
      `auth::login`; accept optional `device_name`/`device_kind` like login
      does). Reject if the user already has a local identity (claimed twice).
- [x] Wire `.nest("/join", crate::families::join_router())` in
      `http/router.rs::api_routes`.

### A4. Provisioning endpoint (family_admin only, in the family router)

- [x] `POST /api/v1/family/members/provision`
      body `{ display_name, login_email, role?, display_label? }` →
      creates the user (`users` row, `is_active = true`, **no
      auth_identities row**, so no one can log in as them yet), inserts them
      into `family_members` immediately (they should appear in the members
      list as "hasn't joined yet"), creates a `claim` invite bound to them,
      returns `{ member, invite: { code, join_url, expires_at } }`.
      Validate `login_email` with the same validator registration uses;
      409 on duplicate email.
- [x] `POST /api/v1/family/invites/{id}/regenerate` — new code + fresh TTL
      for any pending invite (all kinds). Old code dies.
- [x] `DELETE /api/v1/family/members/{user_id}` — extend: if the target has
      **no auth identity and an unclaimed claim invite** (a provisioned
      account that never joined), hard-delete the user + invite instead of
      re-homing them into a personal family. Claimed members keep today's
      re-homing behavior.
- [x] `MemberResponse` gains `pending: bool` (true = provisioned, unclaimed)
      so clients can render the "hasn't joined yet" state. Derive it in the
      members query (`NOT EXISTS (SELECT 1 FROM auth_identities ...)`).

### A5. Link invites in the existing flows

- [x] `CreateInviteRequest` grows `kind?` (`email` default), `max_uses?`,
      `label?`; `email` becomes optional and is required iff `kind = email`.
      Enforce D3 (link ⇒ role member, max_uses ≤ 20) in the handler, not
      just the DB constraint, so the error messages are human.
- [x] `POST /auth/register` with `invite_code`: relax the email-match check —
      it applies only when the invite has an email. `link` invites accept
      any registrant. `claim` invites must be **rejected** here (they are
      not registration codes; point the error at the claim flow).
- [x] `POST /api/v1/family/invites/accept` (logged-in join): same
      relaxation; `claim` rejected likewise. A user who is already the sole
      member of a personal family moves in (existing `move_to_family`);
      guards for "already in a multi-member family" stay as they are.
- [x] `InviteResponse` gains `kind`, `join_url`, `label`, `max_uses`,
      `use_count`, `expires_at` (email becomes nullable). `join_url` =
      `{base_url}/join/{code}` when `server.base_url` is set, else null
      (clients fall back to their connected origin).

### A6. Tests

- [x] Extend the Rust smoke coverage the way Phase 1 did (see
      `backend-family-implementation-plan.md`): link invite → register
      without email match → member; link invite with `max_uses = 3` used by
      3 accounts, 4th rejected; expired link rejected; provision → claim
      sets password and returns working tokens → second claim rejected;
      claim code rejected by `/auth/register`; provisioned-but-unclaimed
      member deleted cleanly; regenerate kills the old code.
- [x] New `scripts/family_join_test.py` (mirror the structure of
      `scripts/family_sharing_test.py`): full HTTP walk of both flows
      including `GET /join/{code}` previews and the D3 guard
      (creating a `family_admin` link invite must 400).
- [x] `cargo clippy --lib` clean on every touched file; existing
      `scripts/backend_smoke_test.py` and `scripts/family_sharing_test.py`
      still green. `cargo fmt --check` was **not** run — the checked-in tree
      already disagrees with this machine's rustfmt on ~every pre-existing
      file (not something introduced here), so running it would have
      produced a repo-wide reformat unrelated to this change. New code was
      hand-formatted to match the surrounding style instead.

### A6.1 Implementation notes (read before touching Phase A code)

- **`crate::mail`** (`backend/src/mail/mod.rs` + `mail/invite.rs`) is a
  Rust port of `audio2-www/functions/lib/mailer.ts`'s JMAP client — same
  protocol, same server, no new infrastructure. Config lives at
  `AppConfig.mail: Option<MailConfig>` (`app/config.rs`,
  `MAIL__JMAP_BASE_URL` / `MAIL__JMAP_USER` / `MAIL__JMAP_PASSWORD` /
  `MAIL__FROM_ADDRESS` / `MAIL__FROM_NAME`); unset ⇒ silent no-op with a
  `tracing::warn!`, same contract as the TS mailer's `null` config path.
  **The hosted own.audio instance reuses `hello@own.audio`** (the address
  the waitlist already sends from) rather than a dedicated `invites@`
  alias — decided 2026-08-23, no new Stalwart alias needed.
  `mail` is declared in **both** `lib.rs` and `main.rs` (this crate has two
  separate module trees — the binary doesn't go through `lib.rs`); add new
  top-level modules to both or the binary fails to build even though
  `cargo check --lib` passes.
- **`AuthError::NotFound` is not a generic 404.** It renders as **401**
  (`auth/error.rs`) — reserved for account-lookup paths (login-style) so a
  missing account can't be distinguished from a wrong password. The public
  `GET /join/{code}` unknown-code case must use **`AuthError::ItemNotFound`**
  (404) instead; this was caught by the manual test run, not by the
  compiler — nothing type-checks the distinction.
- `server.base_url` is required for `join_url`/emailed links to work.
  It's already set in the local dev compose stack
  (`SERVER__BASE_URL=http://localhost:8080`), so `docker compose up backend`
  exercises the real URL-building path, not just the `None` fallback.

## Phase B — Web console (audio2/frontend)

The QR's landing target. Keep it one page, no navigation required.

- [ ] Route `/join/:code` in the SPA router, standalone page (no auth
      required, works logged-out).
- [ ] On load, `GET /api/v1/join/{code}`; render by state:
      - valid + `kind email|link`, logged out → "**{inviter}** invited you to
        the **{family}** family" + register form (name, email — prefilled
        and locked for `email` kind — password) posting to `/auth/register`
        with the code; below it a "have an account? sign in" toggle that
        signs in then calls `/invites/accept`.
      - valid, logged in → one confirm button ("Join as {current user}") →
        `/invites/accept`.
      - valid + `kind claim` → "{admin} set up an account for **{name}**.
        You'll sign in as `{login_email}`." + one password field (+ confirm)
        → `/join/{code}/claim`, store the returned tokens exactly like
        login does, land in the library.
      - expired/exhausted/404 → plain explanation + "ask for a new code".
- [ ] Family settings area: this plan does **not** build the full web family
      management UI (separate effort); do add the pieces the flows need if a
      family page already exists by then — otherwise skip, Mac is the admin
      surface (see repo CLAUDE.md: Mac is the lead client).
- [ ] Success path ends inside the app with a small "welcome to {family}"
      toast — not on a dead-end confirmation page.

## Phase C — Mac client (audio2-mac, lead client — API shapes get proven here)

**Status: mostly done, 2026-08-23** (C1/C2/leave-family/join-preview shipped;
the URL-scheme deep link and pre-auth claim entry are deliberately deferred —
see the note at the end of C3).

### C1. QR rendering (no dependencies) ✅

- [x] New `QRCodeView.swift` in `Audio2DesignSystem`: `CIFilter.qrCodeGenerator()`
      (`correctionLevel = "M"`), upscaled 12x in CI space (not just via
      `.interpolation(.none)` — a naive 1x-per-module image stretched by
      SwiftUI alone came out visibly soft), rendered via `Image(decorative:)`.
      `nonisolated static func render(_:) -> CGImage?` is the testable core;
      `QRCodeViewTests.swift` covers a join URL, a bare code, and the
      degenerate empty-string case, no snapshotting needed.

### C2. Invite creation & share ✅

- [x] Reworked "Invite someone" into `InviteCreationSheet` (`FamilyView.swift`),
      a proper `Form` with a segmented `Picker` — **Share a Link** (default;
      `Stepper` 1–20 uses, optional label), **Email** (today's flow, now also
      mailed server-side when configured), **Create Account** (name + login
      identifier field, suggesting the `name@familyname.family` shape).
      Link mode never exposes a role picker (D3 — server-enforced anyway).
- [x] `InviteShareView` v2: `QRCodeView(content: invite.joinUrl ?? invite.code)`,
      the link as selectable text, the code as a monospaced fallback,
      `ShareLink(item: shareContent)`, and a kind-aware caption (claim/link/
      email each read differently — see the view's own `caption` computed
      property).
- [x] Members list (`memberLabel`): a "Hasn't joined yet" badge for
      `pending` members; tapping one opens a `Menu` ("Show Join Code",
      "Regenerate Code", "Remove") instead of `MemberAccessView` — role/
      policy editing is moot for an account that can't sign in yet.
- [x] `FamilyAPI`: `createInvite` grew `kind/maxUses/label` (default-valued,
      old call sites unchanged), plus `provisionMember`, `regenerateInvite`,
      and `joinPreview` (`requiresAuth: false` — the one endpoint here that
      must work signed-out, see `APIRequest.requiresAuth`). DTOs extended in
      `Family.swift`; `FamilyAPITests`/`FamilyViewModelTests` cover all of it
      (18 + 19 tests respectively, including the D3 link+admin 400 and the
      "public preview sends no Authorization header" check).
- [x] **Backend addendum found necessary here**: `GET /family` gained
      `my_user_id` (the caller's own id) and `InviteResponse` gained
      `member_user_id` (a `claim` invite's target user) — neither was in the
      original A-phase design. `my_user_id` is what makes "Leave family"
      possible without threading `AppContainer.currentUser.id` through
      `SettingsView`/`FamilyView`'s init (both mid-edit for billing work at
      the time); `member_user_id` is what lets a pending member's row find
      its own claim invite in `GET /family/invites` to power "Show Join
      Code"/"Regenerate". Small, additive, already covered by
      `backend_smoke_test.py`/`family_join_test.py`.

### C3. Joining and leaving on the Mac — partly done

- [x] The paste-a-code join alert (`FamilyView.extractCode(from:)`) accepts
      either a bare code or a full join URL — only treats input as a URL
      when it has a `scheme` (a bare hex code parses as a scheme-less
      relative URL otherwise, and would wrongly fall through
      `pathComponents`). Calls `GET /join/{code}` first
      (`FamilyViewModel.previewJoin`) and shows a `JoinConfirmSheet` with the
      family name before spending the code; a code the preview can't
      validate falls back to the old direct-`acceptInvite` behavior so its
      existing error message still covers that case.
- [x] **Leave family**: destructive button + `.confirmationDialog` in
      `FamilyView`, exact copy from this plan's own wording. Targets
      `family.myUserId` (the new backend field above) — no client-side
      last-admin gate; the server's rejection surfaces via `errorMessage`,
      same pattern `MemberAccessViewModel.setRole` already uses.
- [ ] **Deferred**: the `audio2://` URL scheme + `onOpenURL` handling, and
      any pre-auth "claim your account" entry point on the Mac. Both need
      `AppContainer.swift`/the sign-in flow, which (a) this session
      deliberately stayed out of — it was mid-edit for the concurrent
      billing/storage/home-screen work at the time, a 2750-line composition
      root not worth the collision risk for a deep link — and (b) has
      nowhere to send a *signed-out* claimant until Phase B's web landing
      page exists anyway (D1: the QR's primary target is the browser, not
      this app). Revisit once Phase B ships and AppContainer settles;
      `FamilyAPI.joinPreview`/the claim endpoint plumbing is already in
      place and ready to reuse.

### C4. Verify

- [x] `swift test` green: `Audio2DesignSystemTests` (QRCodeView, 3 tests),
      `Audio2NetworkingTests/FamilyAPITests` (18 tests), `FeatureFamilyTests`
      (19 tests) — all via `swift test` in each package plus a full
      `xcodebuild build` of the `Audio2Mac` scheme (BUILD SUCCEEDED,
      including the concurrent billing/`FeatureBilling`/`FeatureHome` work).
- [ ] Manual run against the dev backend: create each invite kind on the
      Mac, scan the QR with a real phone, complete both web flows, watch the
      member appear (and the pending badge clear) after refresh. **Blocked
      on Phase B** — there is no landing page yet for a phone scan to open.

## Phase D — Later (explicitly out of scope now)

- **Universal links** (`https://…/join/…` opening native apps directly)
  need an AASA file per server domain — feasible for hosted instances,
  impossible for arbitrary self-hosted domains; revisit when hosting story
  firms up.
- **Quick-Connect-style device login**: scan a QR shown on a new device from
  an already-signed-in phone to log the device in (Jellyfin's pattern).
  Reuses this plan's short-lived-code plumbing; design when the iOS app
  exists.
- iOS / Android clients port C1–C3 (the API is already client-agnostic).
- Push/email notification of "X joined your family" — backend sends no mail
  today; fold into whatever notification story lands later.

## Security notes for the implementer

- Codes stay 24 hex chars from `OsRng` (96 bits) — do not shorten them for
  typeability; the QR/link is the typing story.
- Never issue or accept `family_admin` on `link` invites (D3) — check in the
  handler and keep the DB CHECK as backstop.
- The claim endpoint must be atomic (redeem + identity-create in one
  transaction) and must refuse users who already have any auth identity.
- `GET /join/{code}` leaks family name + inviter to code holders only (D6);
  unknown codes are a plain 404 with no body variation.
- Revocation (`DELETE /invites/{id}`) and regeneration must kill old codes
  immediately — no caching layer in front of the lookup.
