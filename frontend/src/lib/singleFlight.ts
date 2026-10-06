// SPDX-License-Identifier: AGPL-3.0-or-later
/**
 * Collapse concurrent calls into one.
 *
 * While a call is in flight every other caller gets that same promise rather
 * than starting its own; once it settles the slot is cleared, so the next call
 * runs fresh. Failures are shared too — everyone waiting sees the same error.
 *
 * Used for token refresh, where this is a correctness requirement and not a
 * saving: refresh tokens are single-use, and the server treats a replayed one
 * as a leak and revokes every session on the device chain. Two parallel
 * refreshes would sign the user out of everything.
 */
export function singleFlight<T>(fn: () => Promise<T>): () => Promise<T> {
  let inFlight: Promise<T> | null = null;
  return () => {
    if (!inFlight) {
      inFlight = fn().finally(() => {
        inFlight = null;
      });
    }
    return inFlight;
  };
}
