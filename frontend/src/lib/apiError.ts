// SPDX-License-Identifier: AGPL-3.0-or-later
export function apiErrorMessage(err: unknown, fallback: string): string {
  // v1 answers `{ error }`; v2 answers problem details, whose sentence is `detail`.
  const data = (err as { response?: { data?: { error?: string; detail?: string } } })?.response?.data;
  return data?.error ?? data?.detail ?? fallback;
}

/** Sign-in refused for a while: the per-IP limit or too many wrong passwords. */
export function isRateLimited(err: unknown): boolean {
  return (err as { response?: { status?: number } })?.response?.status === 429;
}
