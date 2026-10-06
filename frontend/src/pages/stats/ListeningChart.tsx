// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { getStats } from "../../api/stats";
import type { StatsRange } from "../../api/types";
import { Skeleton } from "../../components/ui";
import { cn } from "../../lib/cn";
import { bucketize, hoursText, type Bucket, type Horizon } from "./insights";
import { t, useT, type Locale, type PlainKey } from "../../i18n";

/* Listening time as stacked bars — audiobooks at the base, then podcasts, then
   music — so a bar's height is the whole period and its bands the split. A
   week or month is drawn by the day, a quarter by the week, a year or all
   time by the month (see bucketize). Hover, tap or focus a bar for its figures;
   each bar is a button whose label carries them for screen readers. */

const SERIES: { key: string; label: PlainKey; colour: string }[] = [
  { key: "audiobook", label: "common.kind.audiobooks", colour: "var(--chart-book)" },
  { key: "podcast", label: "common.kind.podcasts", colour: "var(--chart-podcast)" },
  { key: "music", label: "common.kind.music", colour: "var(--chart-music)" },
];
const FALLBACK: typeof SERIES = [{ key: "all", label: "stats.chart.listening", colour: "var(--accent)" }];

/** The query that covers a horizon's buckets. A quarter's first week can start
 *  before "90 days ago", so it reads from the year. */
const QUERY_RANGE: Record<Horizon, StatsRange> = { week: "7d", month: "30d", quarter: "365d", year: "365d", all: "all" };

const amount = (secs: number) => (secs > 0 ? hoursText(secs) : "—");

const localToday = () => {
  const d = new Date();
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
};

const at = (day: string) => new Date(`${day}T12:00:00`);
const formatters = (locale: Locale) => ({
  weekday: new Intl.DateTimeFormat(locale, { weekday: "short" }),
  dayMonth: new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }),
  month: new Intl.DateTimeFormat(locale, { month: "short" }),
  monthYear: new Intl.DateTimeFormat(locale, { month: "long", year: "numeric" }),
  long: new Intl.DateTimeFormat(locale, { weekday: "long", day: "numeric", month: "long" }),
});
type Formatters = ReturnType<typeof formatters>;

function tickLabel(F: Formatters, b: Bucket, horizon: Horizon): string {
  if (b.unit === "day") return horizon === "week" ? F.weekday.format(at(b.start)) : F.dayMonth.format(at(b.start));
  if (b.unit === "week") return F.dayMonth.format(at(b.start));
  return F.month.format(at(b.start));
}

function bucketTitle(F: Formatters, b: Bucket): string {
  if (b.unit === "day") return F.long.format(at(b.start));
  if (b.unit === "week") return t("stats.chart.weekOf", { date: F.dayMonth.format(at(b.start)) });
  return F.monthYear.format(at(b.start));
}

/** A round top for the axis and its step, in minutes below two hours, else hours. */
function scale(maxSecs: number): { top: number; ticks: number[]; fmt: (s: number) => string } {
  const minutes = maxSecs < 2 * 3600;
  const unit = minutes ? 60 : 3600;
  const raw = Math.max(maxSecs / unit, 1);
  const step = [1, 2, 5, 10, 15, 20, 30, 50, 60, 100, 200, 500].find((s) => raw / s <= 4) ?? 1000;
  const top = Math.ceil(raw / step) * step;
  const ticks = Array.from({ length: top / step + 1 }, (_, i) => i * step * unit);
  return { top: top * unit, ticks, fmt: (s) => (minutes ? t("stats.chart.axisMinutes", { n: Math.round(s / 60) }) : t("stats.chart.axisHours", { n: Math.round(s / 3600) })) };
}

export function ListeningChart({ horizon, compact = false }: { horizon: Horizon; compact?: boolean }) {
  const { data, isLoading } = useQuery({ queryKey: ["stats", QUERY_RANGE[horizon]], queryFn: () => getStats(QUERY_RANGE[horizon]) });
  const [active, setActive] = useState<number | null>(null);
  const { t, locale } = useT();
  const F = useMemo(() => formatters(locale), [locale]);

  const buckets = useMemo(
    () => (data ? bucketize(horizon, localToday(), data.by_day_kind ?? null, data.by_day) : []),
    [data, horizon]
  );

  const height = compact ? 120 : 200;
  if (isLoading) return <Skeleton className={compact ? "h-44" : "h-64"} />;
  if (!data) return <p className="text-sm text-muted">{t("stats.chart.unavailable")}</p>;

  const series = data.by_day_kind ? SERIES : FALLBACK;
  const total = buckets.reduce((a, b) => a + b.total, 0);
  const days = buckets.length === 0 ? 1 : Math.max(1, Math.round((at(buckets[buckets.length - 1].end).getTime() - at(buckets[0].start).getTime()) / 86_400_000) + 1);
  const { top, ticks, fmt } = scale(Math.max(0, ...buckets.map((b) => b.total)));
  const labelEvery = Math.ceil(buckets.length / (compact ? 7 : 8));
  const shown = active != null ? buckets[active] : null;

  return (
    <div className="rounded-card border border-border p-4">
      <div className="mb-3 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <p className="text-sm">
          <span className="font-semibold tabular-nums">{hoursText(total)}</span>
          <span className="text-muted"> · {t("stats.chart.perDay", { time: hoursText(total / days) })}</span>
        </p>
        {series.length > 1 && (
          <ul className="flex flex-wrap gap-x-3 gap-y-1 text-xs text-muted" aria-label={t("stats.chart.legend")}>
            {series.map((s) => (
              <li key={s.key} className="flex items-center gap-1.5">
                <span className="h-2.5 w-2.5 rounded-[3px]" style={{ background: s.colour }} />
                {t(s.label)}
              </li>
            ))}
          </ul>
        )}
      </div>

      <div className="relative flex gap-2">
        {/* Y axis: recessive gridlines with round values. */}
        <div className="relative w-7 shrink-0 text-right text-[10px] tabular-nums text-muted" style={{ height }} aria-hidden="true">
          {ticks.map((t) => (
            <span key={t} className="absolute right-0" style={{ bottom: `${(t / top) * 100}%`, transform: "translateY(50%)" }}>
              {fmt(t)}
            </span>
          ))}
        </div>
        <div className="relative min-w-0 flex-1">
          <div className="pointer-events-none absolute inset-x-0 top-0" style={{ height }} aria-hidden="true">
            {ticks.map((t) => (
              <span key={t} className="absolute inset-x-0 border-t border-border/70" style={{ bottom: `${(t / top) * 100}%` }} />
            ))}
          </div>

          <div className="relative flex items-end gap-[2px]" style={{ height }} onMouseLeave={() => setActive(null)}>
            {buckets.map((b, i) => {
              const none = t("stats.chart.none");
              const label = t("stats.chart.barLabel", {
                title: bucketTitle(F, b),
                parts: series.map((s) => t("stats.chart.barPart", { kind: t(s.label), time: b.values[s.key] ? hoursText(b.values[s.key]) : none })).join(", "),
                total: b.total ? hoursText(b.total) : none,
              });
              return (
                <button
                  key={b.start}
                  type="button"
                  aria-label={label}
                  onMouseEnter={() => setActive(i)}
                  onFocus={() => setActive(i)}
                  onBlur={() => setActive(null)}
                  onClick={() => setActive((a) => (a === i ? null : i))}
                  className={cn(
                    "group flex h-full min-w-0 flex-1 flex-col items-center justify-end rounded-t-[4px] outline-none focus-visible:ring-2 focus-visible:ring-accent",
                    active === i && "bg-fg/5"
                  )}
                >
                  {b.total > 0 && (
                    <span
                      className="flex w-full max-w-7 flex-col-reverse gap-[2px] overflow-hidden rounded-t-[4px]"
                      style={{ height: Math.max(2, (b.total / top) * height) }}
                    >
                      {series.map((s) =>
                        b.values[s.key] ? (
                          <span key={s.key} className="w-full shrink-0" style={{ flexGrow: b.values[s.key], flexBasis: 0, minHeight: 2, background: s.colour }} />
                        ) : null
                      )}
                    </span>
                  )}
                </button>
              );
            })}
          </div>

          <div className="mt-1.5 flex gap-[2px] text-[10px] text-muted" aria-hidden="true">
            {buckets.map((b, i) => (
              // Flex, not text-align: a label wider than its column overflows
              // on the side flex alignment says, where text-align would give up.
              // The end labels grow inwards, so neither runs off the card.
              <span
                key={b.start}
                className={cn(
                  "flex min-w-0 flex-1 whitespace-nowrap",
                  i === buckets.length - 1 ? "justify-end" : i === 0 ? "justify-start" : "justify-center"
                )}
              >
                {(buckets.length - 1 - i) % labelEvery === 0 ? tickLabel(F, b, horizon) : ""}
              </span>
            ))}
          </div>

          {shown && (
            <div
              role="status"
              className="pointer-events-none absolute top-0 z-10 min-w-40 rounded-[10px] border border-border bg-card px-3 py-2 text-xs shadow-pop"
              style={
                active! < buckets.length / 2
                  ? { left: `${((active! + 1) / buckets.length) * 100}%`, marginLeft: 6 }
                  : { right: `${((buckets.length - active!) / buckets.length) * 100}%`, marginRight: 6 }
              }
            >
              <p className="mb-1 font-medium">{bucketTitle(F, shown)}</p>
              {series.map((s) => (
                <p key={s.key} className="flex items-center gap-2">
                  <span className="h-2 w-2 rounded-[2px]" style={{ background: s.colour }} />
                  <span className="text-muted">{t(s.label)}</span>
                  <span className="ml-auto tabular-nums">{amount(shown.values[s.key] ?? 0)}</span>
                </p>
              ))}
              {series.length > 1 && (
                <p className="mt-1 flex border-t border-border pt-1 font-medium">
                  {t("stats.chart.total")}
                  <span className="ml-auto tabular-nums">{amount(shown.total)}</span>
                </p>
              )}
            </div>
          )}
        </div>
      </div>
    </div>
  );
}
