// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo } from "react";
import { useQuery } from "@tanstack/react-query";
import { getStats } from "../../api/stats";
import { Skeleton } from "../../components/ui";
import { useT, type PlainKey } from "../../i18n";
import { hoursText } from "../stats/insights";

/* A GitHub-style year of listening: one row of day cells per cloud, weeks as
   columns, Monday on top. Each row is shaded against its own busiest day, so
   an hour of music a day does not wash out ten minutes of podcasts. */

const WEEKS = 53;
const LEVELS = [0.3, 0.55, 0.8, 1];

interface Row {
  key: string;
  label: PlainKey;
  colour: string;
  byDay: Map<string, number>;
}

const dayKey = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;


/** Monday-first columns ending with the current week. */
function buildDays(): Date[] {
  const today = new Date();
  today.setHours(12, 0, 0, 0);
  const mondayOffset = (today.getDay() + 6) % 7;
  const start = new Date(today);
  start.setDate(today.getDate() - mondayOffset - (WEEKS - 1) * 7);
  return Array.from({ length: WEEKS * 7 }, (_, i) => {
    const d = new Date(start);
    d.setDate(start.getDate() + i);
    return d;
  });
}

function ActivityRow({ row, days, today }: { row: Row; days: Date[]; today: string }) {
  const { t, locale } = useT();
  const dateFmt = useMemo(() => new Intl.DateTimeFormat(locale, { weekday: "short", day: "numeric", month: "short", year: "numeric" }), [locale]);
  const hoursLabel = (secs: number) => (secs < 60 ? t("home.activity.none") : hoursText(secs));
  const max = Math.max(0, ...row.byDay.values());
  const total = [...row.byDay.values()].reduce((a, b) => a + b, 0);
  const activeDays = [...row.byDay.values()].filter((s) => s >= 60).length;

  return (
    <div>
      <div className="mb-1.5 flex items-baseline justify-between gap-3">
        <span className="text-sm font-medium">{t(row.label)}</span>
        <span className="text-xs tabular-nums text-muted">{t("home.activity.rowSummary", { time: hoursLabel(total), days: activeDays })}</span>
      </div>
      <div
        role="img"
        aria-label={t("home.activity.rowLabel", { kind: t(row.label), time: hoursLabel(total), days: activeDays })}
        className="grid grid-flow-col gap-[2px]"
        style={{ gridTemplateRows: "repeat(7, minmax(0, 1fr))", gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))` }}
      >
        {days.map((d) => {
          const key = dayKey(d);
          const future = key > today;
          const secs = row.byDay.get(key) ?? 0;
          const level = secs >= 60 && max > 0 ? LEVELS.findIndex((l) => Math.sqrt(secs / max) <= l) : -1;
          return (
            <span
              key={key}
              title={future ? undefined : t("home.activity.cell", { date: dateFmt.format(d), time: hoursLabel(secs) })}
              className="aspect-square rounded-[1.5px]"
              style={{
                background: future
                  ? "transparent"
                  : level < 0
                    ? "var(--border)"
                    : `color-mix(in srgb, ${row.colour} ${Math.round(LEVELS[level] * 100)}%, var(--bg-alt))`,
              }}
            />
          );
        })}
      </div>
    </div>
  );
}

/** `withTotal` adds an "All listening" row above the three (the stats page). */
export function ActivityWidget({ withTotal = false }: { withTotal?: boolean }) {
  const { t, locale } = useT();
  const { data, isLoading } = useQuery({ queryKey: ["stats", "365d"], queryFn: () => getStats("365d") });
  const days = useMemo(() => buildDays(), []);
  const today = dayKey(new Date());

  const rows = useMemo<Row[]>(() => {
    if (!data) return [];
    // An older server sends only the combined total: one row rather than none.
    if (!data.by_day_kind) {
      return [{ key: "all", label: "home.activity.allListening", colour: "var(--accent)", byDay: new Map(data.by_day.map((d) => [d.day, d.seconds])) }];
    }
    const kinds: Omit<Row, "byDay">[] = [
      { key: "audiobook", label: "common.kind.audiobooks", colour: "var(--book)" },
      { key: "podcast", label: "common.kind.podcasts", colour: "var(--podcast)" },
      { key: "music", label: "common.kind.music", colour: "var(--music)" },
    ];
    const perKind = kinds.map((k) => ({
      ...k,
      byDay: new Map(data.by_day_kind!.filter((d) => d.media_kind === k.key).map((d) => [d.day, d.seconds])),
    }));
    const total: Row = { key: "all", label: "home.activity.allListening", colour: "var(--accent)", byDay: new Map(data.by_day.map((d) => [d.day, d.seconds])) };
    return withTotal ? [total, ...perKind] : perKind;
  }, [data, withTotal]);

  if (isLoading) return <Skeleton className="h-48" />;
  if (!data) return <p className="text-sm text-muted">{t("home.activity.unavailable")}</p>;

  // Month names under the columns where a month begins. Walked from the right
  // so a partial first month gives way to the full one after it rather than
  // printing over it. A name needs about three columns, so none starts in the
  // last two.
  const monthFmt = new Intl.DateTimeFormat(locale, { month: "short" });
  const months: string[] = Array(WEEKS).fill("");
  let nextKept = WEEKS + 1;
  for (let w = WEEKS - 3; w >= 0; w--) {
    const first = days[w * 7];
    const prev = w > 0 ? days[(w - 1) * 7] : null;
    const starts = !prev || first.getMonth() !== prev.getMonth();
    if (starts && nextKept - w >= 3) {
      months[w] = monthFmt.format(first);
      nextKept = w;
    }
  }

  return (
    <div className="rounded-card border border-border p-4">
      <div className="space-y-4">
        {rows.map((row) => (
          <ActivityRow key={row.key} row={row} days={days} today={today} />
        ))}
      </div>
      <div
        aria-hidden="true"
        className="mt-1.5 grid text-[10px] text-muted"
        style={{ gridTemplateColumns: `repeat(${WEEKS}, minmax(0, 1fr))` }}
      >
        {months.map((m, i) => (
          <span key={i} className="overflow-visible whitespace-nowrap">
            {m}
          </span>
        ))}
      </div>
      <div aria-hidden="true" className="mt-3 flex items-center justify-end gap-1 text-[10px] text-muted">
        {t("home.activity.less")}
        <span className="h-2.5 w-2.5 rounded-[2px] bg-border" />
        {LEVELS.map((l) => (
          <span key={l} className="h-2.5 w-2.5 rounded-[2px]" style={{ background: `color-mix(in srgb, var(--fg) ${Math.round(l * 70)}%, var(--bg-alt))` }} />
        ))}
        {t("home.activity.more")}
      </div>
    </div>
  );
}
