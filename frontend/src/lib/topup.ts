// SPDX-License-Identifier: AGPL-3.0-or-later
import type { LedgerEntry } from "../api/billing";

/** How long before the return from Stripe a top-up in the ledger still counts as "this one". */
export const THANKS_WINDOW_MS = 30 * 60 * 1000;

/**
 * The top-up the payer just came back from: the newest `topup` entry made within the window
 * before `returnedAt`. Undefined while Stripe's webhook has not put it in yet.
 */
export function recentTopup(entries: LedgerEntry[], returnedAt: number): LedgerEntry | undefined {
  return entries
    .filter((e) => e.entry_type === "topup")
    .filter((e) => returnedAt - new Date(e.created_at).getTime() < THANKS_WINDOW_MS)
    .sort((a, b) => b.created_at.localeCompare(a.created_at))[0];
}
