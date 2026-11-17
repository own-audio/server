// SPDX-License-Identifier: AGPL-3.0-or-later
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Absolute API origin (e.g. https://api.own.audio/api/v1) for builds that
   *  are not served by the backend. Unset ⇒ same-origin "/api/v1". */
  readonly VITE_API_BASE_URL?: string;
  /** Cloudflare Turnstile site key for the email confirmation page's human check. Unset ⇒ no
   *  check (a self-hosted server has no use for one; the hosted edition grants its welcome
   *  credit only to a confirmation that passed it). */
  readonly VITE_TURNSTILE_SITEKEY?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
