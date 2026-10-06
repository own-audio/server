// SPDX-License-Identifier: AGPL-3.0-or-later
import { useQuery } from "@tanstack/react-query";
import { getStats } from "../../api/stats";
import { Skeleton } from "../../components/ui";
import { hoursText } from "../stats/insights";
import { useT, type PlainKey } from "../../i18n";

/* The last seven days in one card: the average day for each cloud, and a
   seven-bar strip of when it happened. From /stats?range=7d — nothing new. */

const KINDS: { key: string; label: PlainKey; colour: string }[] = [
  { key: "audiobook", label: "common.kind.audiobooks", colour: "var(--book)" },
  { key: "podcast", label: "common.kind.podcasts", colour: "var(--podcast)" },
  { key: "music", label: "common.kind.music", colour: "var(--music)" },
];

const dayKey = (d: Date) =>
  `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;

export function WeekSummaryWidget() {
  const { t } = useT();
  const { data, isLoading } = useQuery({ queryKey: ["stats", "7d"], queryFn: () => getStats("7d") });
  if (isLoading) return <Skeleton className="h-28" />;
  if (!data) return <p className="text-sm text-muted">{t("home.week.unavailable")}</p>;

  const week = Array.from({ length: 7 }, (_, i) => {
    const d = new Date();
    d.setDate(d.getDate() - 6 + i);
    return dayKey(d);
  });

  return (
    <div>
      <div className="grid grid-cols-3 gap-2">
        {KINDS.map((k) => {
          const total = data.by_kind.find((b) => b.media_kind === k.key)?.seconds ?? 0;
          const perDay = new Map((data.by_day_kind ?? []).filter((d) => d.media_kind === k.key).map((d) => [d.day, d.seconds]));
          const max = Math.max(1, ...perDay.values());
          return (
            <div key={k.key} className="min-w-0 rounded-card border border-border px-3 py-3">
              <p className="truncate text-xs font-medium" style={{ color: k.colour }}>
                {t(k.label)}
              </p>
              <p className="mt-1 truncate text-lg font-semibold tabular-nums leading-tight">{total <= 0 ? "—" : total / 7 < 60 ? t("common.duration.underMinuteShort") : hoursText(total / 7)}</p>
              <p className="text-[11px] text-muted">{t("home.week.perDay")}</p>
              {data.by_day_kind && (
                <div className="mt-2 flex h-6 items-end gap-[3px]" aria-hidden="true">
                  {week.map((day) => {
                    const s = perDay.get(day) ?? 0;
                    return (
                      <span
                        key={day}
                        className="flex-1 rounded-[2px]"
                        style={{ height: s > 0 ? `${Math.max(15, (s / max) * 100)}%` : "2px", background: s > 0 ? k.colour : "var(--border)" }}
                      />
                    );
                  })}
                </div>
              )}
            </div>
          );
        })}
      </div>
      <p className="mt-2 text-xs text-muted">
        {data.streak_days > 1
          ? t("home.week.totalStreak", { time: hoursText(data.total_seconds), days: data.streak_days })
          : t("home.week.total", { time: hoursText(data.total_seconds) })}
      </p>
    </div>
  );
}
