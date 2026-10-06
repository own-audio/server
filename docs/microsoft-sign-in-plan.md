# Microsoft sign-in (B-W3)

Adding "Sign in with Microsoft" alongside Google. Written 2026-09-13, against the code as it
stands, for the Windows client — but the endpoint is platform-neutral and iOS/Android/web can
use it unchanged once it exists.

**Status: backend and Windows client done 2026-09-13.** `POST /auth/microsoft` exists,
`/auth/providers` reports it, and the sign-in card grows a button when a server offers it.
What is left is §6 — signing in with a real account, which needs a person and a browser.
(The `/auth/microsoft/redirect|callback` stubs are a different, unused thing and stay.)

---

## 1. What exists

Verified by reading the code, not assumed. The ❌ rows below were the work; they are done.

| Piece | State |
|---|---|
| `auth_identities.provider` accepts `'microsoft'` | ✅ **already** — migration `0002`, widened in `0045`. Confirmed against the live local database. **No migration is needed.** |
| Loopback + PKCE flow on Windows | ✅ `LoopbackAuthorizationFlow`, provider-agnostic |
| `OAuthProviderConfig.Microsoft(clientId)` | ✅ already written — authorize endpoint and scopes |
| `sso_sign_in` (account linking, invites, token issue) | ✅ provider-agnostic already; `google_sign_in` just passes `"google"` |
| `POST /auth/microsoft` | ✅ done 2026-09-13 |
| `AUTH__MICROSOFT__*` config | ✅ done 2026-09-13 |
| `microsoft` in `GET /auth/providers` | ✅ done 2026-09-13 — **additive**; older servers omit the key |
| `MicrosoftSignInAsync` on the client | ✅ done 2026-09-13, with the button |
| Entra app registration | ✅ created 2026-09-13; credentials live in the gitignored `.env` only |

The work is genuinely small because `sso_sign_in` is already the shared half. What is new is
one config struct, one handler, one verifier, and one button.

---

## 2. The Entra app registration — the part only you can do

In the [Azure portal](https://portal.azure.com) → **Microsoft Entra ID** → **App registrations**
→ **New registration**.

| Field | Value | Why |
|---|---|---|
| **Name** | `own.audio` | Shown on the consent screen. Users read this. |
| **Supported account types** | **Accounts in any organizational directory and personal Microsoft accounts** | Anything narrower excludes `@outlook.com` / `@hotmail.com`, which for a family audio service is most of the audience. This is what makes the authority `common`. |
| **Redirect URI** | platform **Mobile and desktop applications**, and **five explicit URIs**: `http://localhost:51789/` through `http://localhost:51793/` | See the box below — this was wrong twice before it was right. |

Then:

1. **Overview** → copy the **Application (client) ID**.
2. **No client secret.** An app registered under *Mobile and desktop applications* is a
   **public client**, and Entra refuses one outright: `AADSTS90023: Public clients can't send a
   client secret`. PKCE is what secures the exchange. If a secret was created, delete it — it is
   not merely unused, it is a credential lying around for no reason.
3. **Token configuration** → **Add optional claim** → **ID** → add **`email`**. Without it a
   personal Microsoft account can return an ID token with no `email` claim at all.
4. **API permissions** → confirm `openid`, `profile`, `email`, `offline_access` are present
   (delegated, Microsoft Graph). They are the default set; add `email` if it is missing.

> ### The redirect URI, which took two refusals to get right
>
> **Entra matches the host as a string.** `http://localhost` does not match `http://127.0.0.1`.
> The loopback flow sent `127.0.0.1` while the registration said `localhost`, and the authorize
> request was refused with *"The provided value for the input parameter 'redirect_uri' is not
> valid"* before a password could be typed.
>
> **And a personal Microsoft account matches the port too.** Entra ID (work/school) ignores the
> port on a registered loopback redirect, which is what every "just register `http://localhost`"
> answer assumes. A personal account is served by `login.live.com`, which matches the whole URI.
> An ephemeral port therefore works for work accounts and fails for everyone with an
> @outlook.com address — the larger half of a family audio service's users.
>
> Hence: a fixed set of five ports, all registered, tried in order. Five rather than one because
> a single hard-coded port is one conflicting application away from being unusable.
>
> **Google is the opposite case on both counts** — any port on `127.0.0.1`, and a desktop client
> that *requires* a secret. That is why `OAuthProviderConfig` carries the host and ports per
> provider instead of sharing one rule.

---

## 3. Configuration

Mirrors `GoogleAuthConfig` exactly, including the `enabled` master switch, so credentials can be
set and tested before going live.

```
AUTH__MICROSOFT__ENABLED=false
AUTH__MICROSOFT__CLIENT_IDS=<application-client-id>
AUTH__MICROSOFT__DESKTOP_CLIENT_ID=<application-client-id>
# Left empty on purpose: a public client must not send one (AADSTS90023).
AUTH__MICROSOFT__DESKTOP_CLIENT_SECRET=
```

`CLIENT_IDS` is comma-separated and feeds `allowed_audiences`, the same as Google's — one id
today, more as other clients come online.

Remember §5 of `CLAUDE.md`: a new backend setting means **editing `docker-compose.yml` too**,
not only `.env`. Both `.env.example` and `backend/.env.example` want the block as well.

---

## 4. Backend

### 4.1 `POST /auth/microsoft`

Mirror `google_sign_in` (`backend/src/auth/mod.rs:595`) verbatim, with `"microsoft"` as the
provider string passed to `sso_sign_in`. The request body is the same shape:
`{ code, code_verifier, redirect_uri, device_name, device_kind, invite_code }`, the loopback
check on `redirect_uri` included.

### 4.2 The one part that is **not** a copy of Google

**Microsoft's issuer is tenant-specific, so a fixed issuer list cannot work.**

`verify_claims` (`backend/src/auth/oidc.rs:260`) calls `validation.set_issuer(&[...])` with
constants. For Google that is right — the issuer is always `accounts.google.com`. For the
`common` authority Microsoft issues:

```
https://login.microsoftonline.com/{tid}/v2.0
```

where `{tid}` is **the signing-in user's own tenant**, different for every organisation, and
`9188040d-6c67-4c5b-b112-36a304b66dad` for personal accounts. Passing a fixed list rejects
every real token; passing none accepts tokens from anywhere.

The correct check, and Microsoft's own documented rule: decode the `tid` claim, then require
`iss == "https://login.microsoftonline.com/" + tid + "/v2.0"`. So `verify_microsoft_id_token`
needs issuer validation **disabled in `Validation`** and asserted by hand against the token's
own `tid` afterwards — which also means `IdTokenClaims` gains a `tid` field.

**This is the single thing most likely to cost a day if it is discovered at debugging time
rather than read here first.**

Other differences, smaller:

- JWKS: `https://login.microsoftonline.com/common/discovery/v2.0/keys`, with its own
  `CachedJwks` static alongside the Google and Apple ones.
- Token endpoint: `https://login.microsoftonline.com/common/oauth2/v2.0/token`, same form
  fields as Google's exchange.
- `email` may still be absent even with the optional claim configured; fall back to
  `preferred_username` when it looks like an address, and treat a missing one as the error it
  is rather than storing an empty string.
- `email_verified` is **not** issued by Microsoft. Do not default it to `true` silently —
  decide explicitly and write down which, because `sso_sign_in`'s linking rules read it.

### 4.3 `GET /auth/providers`

Add a `microsoft` object shaped like `google`'s (`enabled`, `desktop_client_id`), gated the
same way: `auth.microsoft.as_ref().filter(|m| m.enabled)`, and `non_empty` on the id so an
unset value arrives as `null` rather than `""`.

**This is an additive contract change**, so per `CLAUDE.md` §3 the doc update lands in the
**same commit**: `docs/android-client-guide.md` §3/§12 and `docs/mobile-backend-api-spec.md`.
§12's OIDC row is stale anyway (item B-W2 in the Windows plan).

### 4.4 Tests

`oidc.rs` already has a throwaway RSA keypair and a `verify_claims` test to copy. The one worth
adding beyond the Google equivalents: **a token whose `iss` does not match its own `tid` is
rejected** — that is the whole point of §4.2, and nothing else would catch a regression in it.

---

## 5. Windows client

Small, because the flow is already provider-agnostic.

1. `Providers` record gains `MicrosoftProvider Microsoft` (`AuthDtos.cs:43`).
2. `AuthApi.MicrosoftSignInAsync(...)` — a copy of `GoogleSignInAsync` posting to
   `auth/microsoft`. The request record is shape-identical; reuse it rather than adding a second
   one that means the same thing.
3. `SignInViewModel`: `ShowMicrosoft`, and `SignInWithMicrosoftAsync` calling
   `OAuthProviderConfig.Microsoft(Providers.Microsoft.DesktopClientId!)` — already written.
4. A button in `SignInPanel.xaml`, `x:Uid` + both `.resw` files, an `AutomationId`, shown only
   when `ShowMicrosoft`.

---

## 6. Verifying it

Not "it compiles":

1. `GET /auth/providers` on the local stack reports `microsoft.enabled: false` while
   `AUTH__MICROSOFT__ENABLED=false` — the switch works before the credentials matter.
2. Flip it on; the Windows sign-in card grows a Microsoft button.
3. Sign in with a **personal** Microsoft account and with a **work/school** account. These take
   different paths through §4.2 and only the second proves the tenant-specific issuer check.
4. The user lands in `auth_identities` with `provider = 'microsoft'`, and the device shows as
   **windows** with this PC's name in the web console's device list.
5. Sign in again with the same Microsoft account: it **links to the existing user** rather than
   creating a second one.
6. Sign in with a Microsoft account whose email matches an existing local account, and confirm
   the linking rule that applies is the one `sso_sign_in` already implements for Google — the
   behaviour must not differ by provider.

---

## 6a. Still to do: the web console

`frontend/` has its own sign-in and registration pages and knows nothing about Microsoft yet.
The endpoint is platform-neutral, so nothing server-side is needed — but the browser cannot use
the loopback flow, so the console needs the **redirect** flow rather than this one, and
`/auth/microsoft/redirect|callback` are still the `"TODO"` stubs they always were. That is a
separate piece of backend work, not a port of this one. iOS and Android can use `POST
/auth/microsoft` unchanged.

---

## 7. Order of work

1. You: the Entra registration (§2), and hand over the client id and secret **out of band** —
   never in a commit, an issue, or a chat log.
2. Backend: config, handler, verifier, providers, tests, guide — one commit, local stack only.
3. Verify against local (§6 steps 1–2, 4–6).
4. Push to `main` → canary; set the canary variables; verify §6 step 3 against canary with a
   real Microsoft account.
5. Windows client: the button.
6. Production variables and `AUTH__MICROSOFT__ENABLED=true` only once canary has been used in
   anger.
