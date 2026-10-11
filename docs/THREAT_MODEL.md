# Threat model

What the server protects, from whom, where attacks arrive, and what stands in
the way. Written for the open-source server; the hosted service at own.audio
runs the same code behind the same boundaries plus billing. Kept short on
purpose — a page people read beats a binder nobody opens. Updated when a
boundary moves.

## Assets

- **The library.** Audio files, covers, books and podcasts a family uploaded
  or indexed; who may see each item (private, family). Losing them is the
  worst outcome for the people who run a server; leaking a private item to
  another member of the family or to another family is the worst outcome for
  the project.
- **Accounts.** Passwords (argon2 hashes), refresh tokens (hashes), Subsonic
  API keys (encrypted at rest), two-factor secrets (encrypted at rest),
  recovery codes (hashes), the sign-in providers' identity links.
- **Listening history** and the derived recommendations, opt-in per user.
- **The server itself**: its secrets (`AUTH__SESSION_SECRET`, storage and
  mail credentials), its database, and the network it stands in.
- **Hosted only:** the credit ledger and payments.

## Actors

- **Anyone on the internet** reaching a server that is exposed on purpose
  (the common case: a reverse proxy in front, or Cloudflare).
- **A member of a family** who should see only what is shared with them —
  including a child account with `can_upload` and `can_generate` off.
- **A family admin** of one family, who must not reach another family.
- **An instance admin**, trusted with the install but still not with other
  people's private items beyond what the admin routes show.
- **A user's own device** that was stolen or shared, holding a session.
- **The feeds and services the server fetches from** (podcast feeds,
  MusicBrainz, Wikidata, Apple's and Google's sign-in, Podcast Index), which
  may be slow, malicious or hijacked.
- **Someone with a copy of the database or a backup.**

## Entry points

| Entry point | Who reaches it | Notes |
|---|---|---|
| `/api/v1` — the REST API | apps, the console | bearer tokens (1 h) + refresh tokens (rotating, 90 d) |
| `/rest` — the Subsonic API | third-party music apps | per-user API key, `md5(key+salt)` token or legacy `p=` |
| `/api/v1/media/*` — signed media links | players, browsers | the signature is the authorisation; short-lived |
| The console (static files) | browsers | same origin as the API |
| Sign-in providers' callbacks | Google, Apple, Microsoft | id tokens verified against the providers' keys |
| Podcast feeds, enclosures, artwork, transcripts | the server fetching out | user-supplied URLs |
| Library folders and file sync | the admin's disk, the Mac/Windows app | paths under configured roots only |
| Mail (SMTP submission) | the server sending out | invites, resets, confirmations, notices |
| The database and object store | the server only | not exposed; the compose file keeps them on the internal network |

## Trust boundaries

1. **Internet → server.** Everything past the TLS terminator is untrusted
   until a token or signature says otherwise. Rate limits sit here.
2. **Signed-in user → their family.** Every item has an owner and a
   visibility; every `{id}` in a path is checked against the caller's family
   and the item's visibility. "Not yours" and "does not exist" answer alike.
3. **Family → instance.** Family admins reach only their family; instance
   admin routes take a separate extractor that refuses before reading a body.
4. **Server → the outside.** Outbound fetches go through one guard: public
   addresses only (after DNS, on every redirect), http(s) only, size and
   stall limits.
5. **Server → its secrets at rest.** What the database must hold readable
   (Subsonic keys, TOTP secrets) is encrypted under keys derived from the
   session secret; what can be hashed is hashed.
6. **Hosted only:** the edition seam (`Hooks`) is the only place money or
   quota decisions enter the core.

## Controls, by threat

| Threat | Control | Where |
|---|---|---|
| Password guessing | argon2id; 12-character minimum; per-email lock after five failures (30 s doubling to 16 min, for unknown emails too); per-address limit on sign-in routes; the owner is mailed at the first lock | `auth::password`, `db::login_failures`, `http::rate_limit` |
| Stolen access token | one-hour lifetime; `403` never triggers a refresh; sessions revocable | `AuthConfig::access_ttl`, `db::sessions` |
| Stolen refresh token | stored as a hash; rotation; reuse of a rotated token revokes the whole chain; 90-day life | `db::refresh_tokens` |
| Stolen device, shared computer | sessions page with "sign out everywhere"; password change signs other devices out; mail on a sign-in from an unfamiliar device; optional two-factor sign-in | `/auth/sessions`, `auth::totp`, `mail::new_device` |
| Account takeover through "forgot password" | single-use, 30-minute link stored as a hash; a reset signs every session out; the answer never says whether the email exists | `auth::forgot_password`, `db::password_resets` |
| Account takeover through a sign-in provider | identities link only on a verified email; an unverified provider email never links to an existing account | `auth::sso_sign_in` |
| Fake accounts for free credit (hosted) | the welcome credit waits for a confirmed email and is granted once per family; paid work needs a confirmed email | `auth::verification`, `Hooks::email_verified` |
| One member reading another's private item; one family reading another's | owner and visibility checks on every id; conformance suites `families`, `isolation` and `access` walk the whole API | `db::access::Viewer`, `conformance/` |
| A member spending the family's storage or money without the right | `can_upload`, `can_generate`; the storage hook before anything is kept | `FamilyContext`, `storage::quota` |
| SSRF through a feed URL | the outbound guard | `http::outbound` |
| Oversized uploads and episodes, zip bombs | size caps at presign and complete, streaming to disk under a cap, EPUB text cap (hosted) | `uploads`, `podcasts`, `audiobook_gen::extract` |
| Scraping and abuse | per-device/address limit on the whole API; stricter buckets for search and outbound lookups | `http::rate_limit` |
| Cross-site attacks on the console | CSP without inline scripts, no framing, nosniff, strict referrer, CORS limited to the console's origins; tokens scoped to the console's origin | `http::security_headers`, `http::router::api_cors` |
| A database dump | passwords, refresh tokens, reset and confirmation links, recovery codes as hashes; Subsonic keys and TOTP secrets encrypted | `auth::at_rest` |
| Not knowing what happened | the security trail per account, kept a year; application logs checked in CI for secrets | `db::security_events`, CI |
| Vulnerable dependencies | `cargo deny` (advisories, licences), `npm audit`, image scan, gitleaks in CI | `.github/workflows/ci.yml` |

## Known gaps

Listed so nobody mistakes silence for coverage. They are tracked in the
hardening plan.

- Two-factor sign-in is optional; nothing yet requires it of admins, and
  there are no passkeys.
- No breached-password list is consulted when a password is set.
- The console keeps the refresh token in `localStorage`; moving it into an
  `HttpOnly` cookie is planned.
- The Subsonic API still accepts the legacy plaintext `p=` parameter (the
  protocol does), which a proxy log could capture; prefer the token form.
- Uploaded files are typed by their declared content type, not by content.
- Family role and permission changes and invites are not yet on the
  security trail.
- Alerts (failed sign-in spikes, `5xx`) and an incident procedure exist only
  for the hosted service, outside this repository.
