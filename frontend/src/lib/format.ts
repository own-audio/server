// SPDX-License-Identifier: AGPL-3.0-or-later
import { intlLocale, t } from "../i18n";

export function formatDuration(secs: number): string {
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  if (h > 0) return m > 0 ? t("common.duration.hoursMinutes", { h, m }) : t("common.duration.hours", { h });
  if (m > 0) return s > 0 ? t("common.duration.minutesSeconds", { m, s }) : t("common.duration.minutes", { m });
  return t("common.duration.seconds", { s });
}

export function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString(intlLocale(), {
    year: "numeric",
    month: "short",
    day: "numeric",
  });
}

export function formatCents(cents: number, currency: string): string {
  return new Intl.NumberFormat(intlLocale(), { style: "currency", currency }).format(cents / 100);
}

/** "5 min ago" / "před 5 min", worded by the browser's own relative-time rules. */
export function timeAgo(iso: string): string {
  const rtf = new Intl.RelativeTimeFormat(intlLocale(), { numeric: "auto", style: "short" });
  const mins = Math.floor((Date.now() - new Date(iso).getTime()) / 60_000);
  if (mins < 60) return rtf.format(-mins, "minute");
  const hrs = Math.floor(mins / 60);
  if (hrs < 24) return rtf.format(-hrs, "hour");
  return rtf.format(-Math.floor(hrs / 24), "day");
}

/** Feed descriptions are HTML; render them as plain text rather than trusting markup. */
export function stripHtml(html: string): string {
  return html
    .replace(/<[^>]+>/g, " ")
    .replace(/&amp;/g, "&").replace(/&lt;/g, "<").replace(/&gt;/g, ">")
    .replace(/&quot;/g, '"').replace(/&#39;/g, "'").replace(/&nbsp;/g, " ")
    .replace(/\s{2,}/g, " ")
    .trim();
}
