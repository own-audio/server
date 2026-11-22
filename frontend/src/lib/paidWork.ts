// SPDX-License-Identifier: AGPL-3.0-or-later
import { formatAmount } from "../api/billing";
import { t } from "../i18n";

/**
 * Why the server refused paid work (a narration, a translation, a quote for one), read from the
 * body's `code` — never from the status alone or the English text
 * (`audio2/docs/creation-features-clients.md` §5.4).
 */
export type PaidWorkRefusal =
  | { kind: "insufficient_credit" }
  | { kind: "daily_cap"; limitMicro: number | null }
  | { kind: "too_many_jobs"; maxRunning: number | null }
  | { kind: "email_unverified" }
  | { kind: "not_allowed" }
  | { kind: "tier_withdrawn" }
  | { kind: "feature_unavailable" }
  /** Any other 400/422: the server's own sentence (file too large, the same language twice). */
  | { kind: "bad_request"; message: string | null }
  /** Network, 5xx, anything unrecognised. */
  | { kind: "other" };

interface RefusalBody {
  code?: string;
  error?: string;
  detail?: string;
  daily_cap_micro?: number | string;
  max_running?: number | string;
}

/** A refusal's number, as v1 sends it and as v2 keeps it beside `code` (core 7853a2c). */
function count(value: unknown): number | null {
  const n = typeof value === "string" && value.trim() !== "" ? Number(value) : value;
  return typeof n === "number" && Number.isFinite(n) && n >= 0 ? n : null;
}

export function readRefusal(err: unknown): PaidWorkRefusal {
  const response = (err as { response?: { status?: number; data?: unknown } })?.response;
  const status = response?.status;
  const body = (response?.data && typeof response.data === "object" ? response.data : {}) as RefusalBody;
  const sentence = body.detail ?? body.error ?? null;
  switch (body.code) {
    case "insufficient_credit":
      return { kind: "insufficient_credit" };
    case "daily_cap_reached":
      return { kind: "daily_cap", limitMicro: count(body.daily_cap_micro) };
    case "too_many_jobs":
      return { kind: "too_many_jobs", maxRunning: count(body.max_running) };
    case "email_unverified":
      return { kind: "email_unverified" };
    case "feature_unavailable":
      return { kind: "feature_unavailable" };
  }
  if (status === 402) return { kind: "insufficient_credit" };
  if (status === 501) return { kind: "feature_unavailable" };
  if (status === 403) return { kind: "not_allowed" };
  // "unknown voice_tier: …" or "voice_tier … is not available on this server".
  if (status === 400 && sentence && /\bvoice_tier\b/.test(sentence)) return { kind: "tier_withdrawn" };
  if (status === 400 || status === 422) return { kind: "bad_request", message: sentence };
  return { kind: "other" };
}

/** The catalog sentence for a refusal; `fallback` for a network error or a 5xx. */
export function refusalMessage(refusal: PaidWorkRefusal, fallback: string, currency = "USD"): string {
  switch (refusal.kind) {
    case "insufficient_credit":
      return t("billing.paidWork.insufficientCreditWeb");
    case "daily_cap":
      // The limit is the server's, formatted; an older server without it gets the sentence without a figure.
      return refusal.limitMicro != null
        ? t("billing.paidWork.dailyCap", { limit: formatAmount(refusal.limitMicro, currency) })
        : t("billing.paidWork.dailyCapNoFigure");
    case "too_many_jobs":
      return t("billing.paidWork.tooManyJobs", { count: refusal.maxRunning ?? 2 });
    case "email_unverified":
      return t("billing.paidWork.emailUnverified");
    case "not_allowed":
      return t("billing.paidWork.notAllowed");
    case "tier_withdrawn":
      return t("generate.tier.withdrawn");
    case "bad_request":
      return refusal.message ?? fallback;
    default:
      return fallback;
  }
}

/** What a cover change refused or failed with (`POST …/jobs/{id}/cover`), as the catalog says it. */
export function coverChangeMessage(err: unknown): string {
  const response = (err as { response?: { status?: number; data?: unknown } })?.response;
  const body = (response?.data && typeof response.data === "object" ? response.data : {}) as RefusalBody;
  switch (body.code) {
    case "not_finished":
      return t("generate.cover.error.notFinished");
    case "no_book_cover":
      return t("generate.cover.error.noBookCover");
    case "cover_limit_reached":
      return t("generate.cover.error.limitReached");
    case "cover_failed":
      return t("generate.cover.error.failed");
    case "cover_unavailable":
      return t("generate.cover.error.unavailable");
  }
  const status = response?.status;
  if (status === 404) return t("generate.cover.error.notFound");
  if (status === 400) return body.detail ?? body.error ?? t("generate.cover.error.generic");
  // A generic 403 from the paid-work limiter or a lost permission reads the same as a refusal.
  if (status === 403 || status === 402 || status === 409) return refusalMessage(readRefusal(err), t("generate.cover.error.generic"));
  return t("generate.cover.error.generic");
}
