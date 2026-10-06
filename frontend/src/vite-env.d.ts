// SPDX-License-Identifier: AGPL-3.0-or-later
/// <reference types="vite/client" />

interface ImportMetaEnv {
  /** Absolute API origin (e.g. https://api.own.audio/api/v1) for builds that
   *  are not served by the backend. Unset ⇒ same-origin "/api/v1". */
  readonly VITE_API_BASE_URL?: string;
}

interface ImportMeta {
  readonly env: ImportMetaEnv;
}
