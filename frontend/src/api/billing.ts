// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import { intlLocale } from "../i18n";

/* Storage billing. Amounts are in micro-units of the currency (1e-6), because
   a day's storage cost for a small library is far below a cent. */

export interface BillingStorage {
  total_bytes: number;
  audiobooks_bytes: number;
  podcasts_bytes: number;
  music_bytes: number;
  other_bytes: number;
  /** Referenced objects with no recorded size, counted as zero — a non-zero
   *  value here means `total_bytes` is an undercount. */
  unsized_objects: number;
}

export interface BillingPricing {
  currency: string;
  price_per_gb_month_micro: number;
  gb_bytes: number;
}

export interface LedgerEntry {
  id: string;
  entry_type: string;
  amount_micro: number;
  storage_bytes: number | null;
  charge_date: string | null;
  note: string | null;
  created_at: string;
  external_ref: string | null;
}

export interface BillingAlerts {
  min_balance_micro: number | null;
  min_days_remaining: number | null;
  balance_alert_active: boolean;
  days_alert_active: boolean;
}

/** Top-ups are dormant until Stripe is configured — `enabled` mirrors that,
 *  the same pattern `/auth/providers` uses for SSO. */
export interface PaymentsInfo {
  enabled: boolean;
  currency: string;
  presets_micro: number[];
  min_micro: number;
  max_micro: number;
}

export interface Billing {
  storage: BillingStorage;
  pricing: BillingPricing;
  balance_micro: number;
  estimated_daily_cost_micro: number;
  estimated_monthly_cost_micro: number;
  days_remaining: number | null;
  runs_out_on: string | null;
  depleted: boolean;
  last_charge: { charge_date: string; storage_bytes: number; amount_micro: number } | null;
  entries: LedgerEntry[];
  /** Null for a non-admin: thresholds are a management setting. */
  alerts: BillingAlerts | null;
  payments: PaymentsInfo;
}

/** The family's bytes per kind, from the core (`GET /family/storage`) — the
 *  half of the billing page every edition has. */
export async function getFamilyStorage(): Promise<BillingStorage> {
  const { data } = await api.get<BillingStorage>("/family/storage");
  return data;
}

export async function getBilling(): Promise<Billing> {
  const { data } = await api.get<Billing>("/family/billing");
  return data;
}

/** Full replace of both thresholds; `null` turns a rule off. */
export async function setBillingAlerts(req: {
  min_balance_micro: number | null;
  min_days_remaining: number | null;
}): Promise<void> {
  await api.put("/family/billing/alerts", req);
}

export async function createTopup(amount_micro: number): Promise<{ checkout_url: string }> {
  const { data } = await api.post<{ checkout_url: string }>("/family/billing/topup", { amount_micro });
  return data;
}

export function formatMicro(micro: number, currency: string): string {
  return new Intl.NumberFormat(intlLocale(), { style: "currency", currency }).format(micro / 1_000_000);
}

/** Binary sizes with the app language's decimal mark — "1.5 GB", "1,5 GB". */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let i = 0;
  while (value >= 1024 && i < units.length - 1) {
    value /= 1024;
    i++;
  }
  const digits = value >= 100 || i === 0 ? 0 : 1;
  const n = new Intl.NumberFormat(intlLocale(), { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(value);
  return `${n} ${units[i]}`;
}
