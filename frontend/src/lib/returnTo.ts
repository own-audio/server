// SPDX-License-Identifier: AGPL-3.0-or-later
/* Where to land after signing in. Only same-origin paths are honoured so a
   crafted link can't bounce a fresh sign-in to another site. */
export function safeReturnTo(raw: string | null | undefined): string {
  if (!raw || !raw.startsWith("/") || raw.startsWith("//")) return "/";
  if (raw.startsWith("/auth") || raw.startsWith("/setup")) return "/";
  return raw;
}

export function loginPathFor(location: { pathname: string; search: string }): string {
  const target = location.pathname + location.search;
  return target === "/" ? "/auth/login" : `/auth/login?next=${encodeURIComponent(target)}`;
}
