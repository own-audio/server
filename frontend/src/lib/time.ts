// SPDX-License-Identifier: AGPL-3.0-or-later
import { intlLocale, t } from "../i18n";
/** h:mm:ss (or m:ss under an hour) — for positions and durations on the player. */
export function formatClock(secs: number): string {
  if (!isFinite(secs) || secs < 0) return "0:00";
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  const s = Math.floor(secs % 60);
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m);
  return `${h > 0 ? `${h}:` : ""}${mm}:${String(s).padStart(2, "0")}`;
}

/** "1h 20m left" — for remaining time, where seconds are noise. */
export function formatRemaining(secs: number): string {
  if (!isFinite(secs) || secs <= 0) return "";
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  if (h > 0) return `${h}h ${m}m left`;
  if (m > 0) return `${m}m left`;
  return "under a minute left";
}

/** "in 6 days", "in 3 hours" — for a future timestamp. `timeAgo` measures the
 *  other direction and renders an expiry as a negative if you reach for it. */
/** "in 6 days" / "za 6 dní", worded by the browser's relative-time rules. */
export function formatUntil(iso: string): string {
  const ms = new Date(iso).getTime() - Date.now();
  if (!isFinite(ms)) return "";
  if (ms <= 0) return t("common.time.expired");
  const rtf = new Intl.RelativeTimeFormat(intlLocale(), { numeric: "always" });
  const mins = Math.round(ms / 60_000);
  if (mins < 60) return rtf.format(mins, "minute");
  const hours = Math.round(mins / 60);
  if (hours < 48) return rtf.format(hours, "hour");
  return rtf.format(Math.round(hours / 24), "day");
}
