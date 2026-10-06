// SPDX-License-Identifier: AGPL-3.0-or-later
/* Figures for the Wrapped story and the stats page, all derived on the client
   from what /stats already returns — by_day, by_kind, top_items, the history
   and the family totals. Nothing here asks the server for a new statistic. */

import type { DayKindTotal, DayTotal, FamilyStatsEntry, HistoryEntry, TopItem } from "../../api/types";
import { intlLocale, t, type PlainKey } from "../../i18n";

/** Catalog keys — translate with t() where they are shown. */
export const KIND_LABEL: Record<string, PlainKey> = { audiobook: "common.kind.audiobooks", podcast: "common.kind.podcasts", music: "common.kind.music" };

const DAY_MS = 86_400_000;

/** Weekday names in the app's language, Monday first (1 Jan 2024 was a Monday). */
export function weekdayNames(width: "long" | "short" | "narrow" = "long"): string[] {
  const fmt = new Intl.DateTimeFormat(intlLocale(), { weekday: width, timeZone: "UTC" });
  return Array.from({ length: 7 }, (_, i) => fmt.format(Date.UTC(2024, 0, 1 + i)));
}

/** `YYYY-MM-DD` as a UTC midnight timestamp: day arithmetic without DST. */
const dayNumber = (day: string) => Date.parse(`${day}T00:00:00Z`) / DAY_MS;

export function busiestDay(days: DayTotal[]): DayTotal | null {
  return days.reduce<DayTotal | null>((best, d) => (d.seconds > (best?.seconds ?? 0) ? d : best), null);
}

/** Longest run of consecutive days with at least a minute of listening. */
export function longestStreak(days: DayTotal[]): { length: number; from: string | null; to: string | null } {
  const active = days.filter((d) => d.seconds >= 60).map((d) => d.day).sort();
  let best = { length: 0, from: null as string | null, to: null as string | null };
  let runStart = 0;
  for (let i = 0; i < active.length; i++) {
    if (i > 0 && dayNumber(active[i]) - dayNumber(active[i - 1]) !== 1) runStart = i;
    const length = i - runStart + 1;
    if (length > best.length) best = { length, from: active[runStart], to: active[i] };
  }
  return best;
}

/** The calendar month with the most listening, as `YYYY-MM`. */
export function bestMonth(days: DayTotal[]): { month: string; seconds: number } | null {
  const byMonth = new Map<string, number>();
  for (const d of days) byMonth.set(d.day.slice(0, 7), (byMonth.get(d.day.slice(0, 7)) ?? 0) + d.seconds);
  let best: { month: string; seconds: number } | null = null;
  for (const [month, seconds] of byMonth) if (seconds > (best?.seconds ?? 0)) best = { month, seconds };
  return best;
}

/** Total per weekday, Monday first, and the favourite — its name in the app's
 *  language, and its index (0 = Monday) for messages that inflect it. */
export function weekdays(days: DayTotal[]): { totals: number[]; favourite: string | null; favouriteIndex: number | null } {
  const totals = Array(7).fill(0) as number[];
  for (const d of days) totals[(new Date(`${d.day}T12:00:00Z`).getUTCDay() + 6) % 7] += d.seconds;
  const max = Math.max(...totals);
  const index = max > 0 ? totals.indexOf(max) : null;
  return { totals, favourite: index == null ? null : weekdayNames()[index], favouriteIndex: index };
}

/** An id, not a label: messages pick their wording with a select on it. */
export type DayPart = "Morning" | "Afternoon" | "Evening" | "Night";

/** Listening by part of the day, in the viewer's local time, from sessions. */
export function dayParts(history: HistoryEntry[]): { totals: Record<DayPart, number>; favourite: DayPart | null } {
  const totals: Record<DayPart, number> = { Morning: 0, Afternoon: 0, Evening: 0, Night: 0 };
  for (const h of history) {
    const hour = new Date(h.started_at).getHours();
    const part: DayPart = hour >= 5 && hour < 12 ? "Morning" : hour >= 12 && hour < 17 ? "Afternoon" : hour >= 17 && hour < 22 ? "Evening" : "Night";
    totals[part] += h.seconds;
  }
  const entries = Object.entries(totals) as [DayPart, number][];
  const top = entries.reduce((a, b) => (b[1] > a[1] ? b : a));
  return { totals, favourite: top[1] > 0 ? top[0] : null };
}

const DEVICE_LABEL: Record<string, PlainKey> = {
  web: "stats.device.web",
  ios: "stats.device.ios",
  android: "stats.device.android",
  macos: "stats.device.macos",
  windows: "stats.device.windows",
  tvos: "stats.device.tvos",
  subsonic: "stats.device.subsonic",
  other: "stats.device.other",
};

/** A device kind's name in the app's language; an unknown kind shows as it came. */
export const deviceLabel = (kind: string): string => (DEVICE_LABEL[kind] ? t(DEVICE_LABEL[kind]) : kind);

/** Share of listening time per device kind, largest first, labelled in the app's language. */
export function devices(history: HistoryEntry[]): { kind: string; label: string; share: number }[] {
  const byKind = new Map<string, number>();
  let total = 0;
  for (const h of history) {
    byKind.set(h.device_kind, (byKind.get(h.device_kind) ?? 0) + h.seconds);
    total += h.seconds;
  }
  if (total === 0) return [];
  return [...byKind]
    .sort((a, b) => b[1] - a[1])
    .map(([kind, seconds]) => ({ kind, label: deviceLabel(kind), share: seconds / total }));
}

/** The most-listened item of each kind, from the server's top list. */
export function topPerKind(items: TopItem[]): Partial<Record<"audiobook" | "podcast" | "music", TopItem>> {
  const out: Partial<Record<"audiobook" | "podcast" | "music", TopItem>> = {};
  for (const t of items) {
    const k = t.media_kind as "audiobook" | "podcast" | "music";
    if (k in KIND_LABEL && !out[k]) out[k] = t;
  }
  return out;
}

export interface FamilySummary {
  totalSeconds: number;
  /** Members who share their totals, busiest first. */
  members: { name: string; seconds: number; share: number }[];
  /** Who listened most to each kind. */
  leaders: { kind: string; name: string; seconds: number }[];
  privateCount: number;
}

/** Family totals only — admins never see what anyone played, and a member who
 *  keeps their listening private is counted, never figured. */
export function familySummary(entries: FamilyStatsEntry[]): FamilySummary {
  const shared = entries.filter((e) => !e.hidden && (e.total_seconds ?? 0) > 0);
  const totalSeconds = shared.reduce((a, e) => a + (e.total_seconds ?? 0), 0);
  const name = (e: FamilyStatsEntry) => e.display_label ?? e.display_name;
  const members = shared
    .map((e) => ({ name: name(e), seconds: e.total_seconds ?? 0, share: totalSeconds ? (e.total_seconds ?? 0) / totalSeconds : 0 }))
    .sort((a, b) => b.seconds - a.seconds);
  const leaders: FamilySummary["leaders"] = [];
  for (const kind of Object.keys(KIND_LABEL)) {
    let best: { name: string; seconds: number } | null = null;
    for (const e of shared) {
      const s = e.by_kind?.find((k) => k.media_kind === kind)?.seconds ?? 0;
      if (s > (best?.seconds ?? 0)) best = { name: name(e), seconds: s };
    }
    if (best) leaders.push({ kind, ...best });
  }
  return { totalSeconds, members, leaders, privateCount: entries.filter((e) => e.hidden).length };
}

/** "1 h 20 min", rounded to the minute, in the app's language. */
export function hoursText(secs: number): string {
  if (secs < 60) return t("common.duration.underMinute");
  const h = Math.floor(secs / 3600);
  const m = Math.round((secs % 3600) / 60);
  return h ? (m ? t("common.duration.hoursMinutes", { h, m }) : t("common.duration.hours", { h })) : t("common.duration.minutes", { m });
}

export type Horizon = "week" | "month" | "quarter" | "year" | "all";

export interface Bucket {
  /** First day of the bucket, `YYYY-MM-DD`. */
  start: string;
  /** Last day, inclusive — the same as `start` for a day bucket. */
  end: string;
  unit: "day" | "week" | "month";
  /** Seconds per media kind; `all` when the server sent no split. */
  values: Record<string, number>;
  total: number;
}

const iso = (d: Date) => `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, "0")}-${String(d.getUTCDate()).padStart(2, "0")}`;
const addDays = (d: Date, n: number) => new Date(d.getTime() + n * DAY_MS);

/**
 * Days, weeks or months ending with `today`, each holding the listening that
 * fell in it: a week or a month by the day, a quarter by the (Monday) week, a
 * year or all time by the month. Empty buckets stay, so gaps read as gaps.
 * `today` is the viewer's local date as `YYYY-MM-DD`.
 */
export function bucketize(horizon: Horizon, today: string, byDayKind: DayKindTotal[] | null, byDay: DayTotal[]): Bucket[] {
  const rows: DayKindTotal[] = byDayKind ?? byDay.map((d) => ({ day: d.day, media_kind: "all", seconds: d.seconds }));
  const end = new Date(`${today}T00:00:00Z`);
  const out: Bucket[] = [];

  if (horizon === "week" || horizon === "month") {
    const n = horizon === "week" ? 7 : 30;
    for (let i = n - 1; i >= 0; i--) {
      const day = iso(addDays(end, -i));
      out.push({ start: day, end: day, unit: "day", values: {}, total: 0 });
    }
  } else if (horizon === "quarter") {
    const monday = addDays(end, -((end.getUTCDay() + 6) % 7));
    for (let i = 12; i >= 0; i--) {
      const start = addDays(monday, -7 * i);
      out.push({ start: iso(start), end: iso(addDays(start, 6)), unit: "week", values: {}, total: 0 });
    }
  } else {
    let months = 12;
    if (horizon === "all" && rows.length > 0) {
      const first = rows.reduce((a, r) => (r.day < a ? r.day : a), rows[0].day);
      const [fy, fm] = first.split("-").map(Number);
      months = Math.max(12, (end.getUTCFullYear() - fy) * 12 + (end.getUTCMonth() + 1 - fm) + 1);
    }
    for (let i = months - 1; i >= 0; i--) {
      const start = new Date(Date.UTC(end.getUTCFullYear(), end.getUTCMonth() - i, 1));
      const last = new Date(Date.UTC(start.getUTCFullYear(), start.getUTCMonth() + 1, 0));
      out.push({ start: iso(start), end: iso(last), unit: "month", values: {}, total: 0 });
    }
  }

  for (const r of rows) {
    // Buckets are sorted and contiguous: the first whose end is on or after the day.
    const b = out.find((x) => r.day >= x.start && r.day <= x.end);
    if (!b) continue;
    b.values[r.media_kind] = (b.values[r.media_kind] ?? 0) + r.seconds;
    b.total += r.seconds;
  }
  return out;
}
