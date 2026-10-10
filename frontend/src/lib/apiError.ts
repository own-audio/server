// SPDX-License-Identifier: AGPL-3.0-or-later
export function apiErrorMessage(err: unknown, fallback: string): string {
  return (err as { response?: { data?: { error?: string } } })?.response?.data?.error ?? fallback;
}

/** Sign-in refused for a while: the per-IP limit or too many wrong passwords. */
export function isRateLimited(err: unknown): boolean {
  return (err as { response?: { status?: number } })?.response?.status === 429;
}
