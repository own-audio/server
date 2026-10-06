// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Flame, Sparkles, Users } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../../components/ui/CloudIcon";
import { getFamilyStats, getStats, listHistory, setStatsVisibility } from "../../api/stats";
import { getFamily, isFamilyAdmin } from "../../api/family";
import { Page } from "../../components/shell/SplitView";
import { Button, EmptyState, SegmentedControl, Skeleton, Pill, toast } from "../../components/ui";
import { formatDuration } from "../../lib/format";
import { cn } from "../../lib/cn";
import type { KindTotal, StatsRange, StatsVisibility } from "../../api/types";
import { ActivityWidget } from "../home/ActivityWidget";
import { WrappedDialog } from "./Wrapped";
import { ListeningChart } from "./ListeningChart";
import { deviceLabel, type Horizon } from "./insights";
import { t, useT, type PlainKey } from "../../i18n";

const RANGES: { value: StatsRange; label: PlainKey; horizon: Horizon }[] = [
  { value: "7d", label: "stats.range.week", horizon: "week" },
  { value: "30d", label: "stats.range.month", horizon: "month" },
  { value: "90d", label: "stats.range.quarter", horizon: "quarter" },
  { value: "365d", label: "stats.range.year", horizon: "year" },
  { value: "all", label: "stats.range.all", horizon: "all" },
];

const KIND_STYLE: Record<string, { label: PlainKey; bar: string; text: string; icon: React.ReactNode }> = {
  audiobook: { label: "common.kind.audiobooks", bar: "bg-book", text: "text-book", icon: <BookIcon className="h-4 w-4" /> },
  podcast: { label: "common.kind.podcasts", bar: "bg-podcast", text: "text-podcast", icon: <PodcastIcon className="h-4 w-4" /> },
  music: { label: "common.kind.music", bar: "bg-music", text: "text-music", icon: <MusicIcon className="h-4 w-4" /> },
};

function Stat({ label, value, hint }: { label: string; value: string; hint?: string }) {
  return (
    <div className="rounded-card border border-border px-4 py-3">
      <p className="text-xs uppercase tracking-wide text-muted">{label}</p>
      <p className="mt-1 text-2xl font-semibold tracking-tight tabular-nums">{value}</p>
      {hint && <p className="mt-0.5 text-xs text-muted">{hint}</p>}
    </div>
  );
}

/** A kind's name in the current language; an unknown kind shows as it came. */
const kindName = (kind: string) => (KIND_STYLE[kind] ? t(KIND_STYLE[kind].label) : kind);

function KindBars({ byKind, total }: { byKind: KindTotal[]; total: number }) {
  const { t } = useT();
  if (byKind.length === 0) return null;
  return (
    <div className="rounded-card border border-border p-4">
      <p className="mb-3 text-xs uppercase tracking-wide text-muted">{t("stats.kinds.title")}</p>
      <div className="space-y-2.5">
        {byKind.map((k) => {
          const style = KIND_STYLE[k.media_kind] ?? { bar: "bg-accent", text: "text-accent", icon: null };
          const pct = total > 0 ? Math.round((k.seconds / total) * 100) : 0;
          return (
            <div key={k.media_kind}>
              <div className="mb-1 flex items-center gap-2 text-sm">
                <span className={style.text}>{style.icon}</span>
                <span className="font-medium">{kindName(k.media_kind)}</span>
                <span className="ml-auto tabular-nums text-muted">{formatDuration(k.seconds)}</span>
                <span className="w-9 text-right tabular-nums text-xs text-muted">{t("stats.wrapped.share", { share: pct / 100 })}</span>
              </div>
              <div className="h-1.5 rounded-pill bg-bg-alt">
                <div className={cn("h-full rounded-pill", style.bar)} style={{ width: `${pct}%` }} />
              </div>
            </div>
          );
        })}
      </div>
    </div>
  );
}

function FamilyStats({ range }: { range: StatsRange }) {
  const { t } = useT();
  const { data: family } = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false });
  const isAdmin = isFamilyAdmin(family?.my_role);
  const { data, isLoading } = useQuery({
    queryKey: ["family-stats", range],
    queryFn: () => getFamilyStats(range),
    enabled: isAdmin,
    retry: false,
  });

  if (!isAdmin) return null;
  if (isLoading) return <Skeleton className="h-32" />;
  if (!data || data.length === 0) return null;

  return (
    <section className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-lg font-semibold tracking-tight">
        <Users className="h-4 w-4 text-muted" /> {t("stats.family.title")}
        <span className="text-sm font-normal text-muted">· {t(RANGES.find((r) => r.value === range)?.label ?? "stats.range.month")}</span>
      </h2>
      <div className="divide-y divide-border rounded-card border border-border">
        {data.map((m) => (
          <div key={m.user_id} className="flex items-center gap-3 px-4 py-3">
            <span className="min-w-0 flex-1">
              <span className="block truncate text-sm font-medium">{m.display_label ?? m.display_name}</span>
              {m.hidden ? (
                <span className="block text-xs text-muted">{t("stats.family.keepsPrivate")}</span>
              ) : (
                <span className="block text-xs text-muted">
                  {(m.by_kind ?? []).map((k) => t("stats.family.kindTime", { kind: kindName(k.media_kind), time: formatDuration(k.seconds) })).join(" · ") ||
                    t("stats.family.nothingYet")}
                </span>
              )}
            </span>
            {/* A hidden member gets a row and no figures — never a zero, which
                would read as "listened to nothing". */}
            {m.hidden ? <Pill>{t("stats.family.private")}</Pill> : <span className="tabular-nums text-sm">{formatDuration(m.total_seconds ?? 0)}</span>}
          </div>
        ))}
      </div>
    </section>
  );
}

export default function StatsPage() {
  const { t, locale } = useT();
  const qc = useQueryClient();
  const [range, setRange] = useState<StatsRange>("30d");
  const [historyLimit, setHistoryLimit] = useState(25);
  const [wrappedOpen, setWrappedOpen] = useState(false);

  const { data, isLoading } = useQuery({ queryKey: ["stats", range], queryFn: () => getStats(range) });
  const { data: history = [], isLoading: historyLoading } = useQuery({
    queryKey: ["history", historyLimit],
    queryFn: () => listHistory(historyLimit),
  });

  const visibility = useMutation({
    mutationFn: (v: StatsVisibility) => setStatsVisibility(v),
    onSuccess: (_d, v) => {
      qc.invalidateQueries({ queryKey: ["family-stats"] });
      toast.success(v === "private" ? t("stats.visibility.nowPrivate") : t("stats.visibility.nowShared"));
    },
    onError: () => toast.error(t("stats.visibility.error")),
  });

  const topItems = useMemo(() => (data?.top_items ?? []).slice(0, 10), [data]);

  return (
    <Page
      title={t("stats.page.title")}
      actions={<SegmentedControl<StatsRange> size="sm" value={range} onChange={setRange} segments={RANGES.map(({ value, label }) => ({ value, label: t(label) }))} />}
      width="max-w-4xl"
    >
      {isLoading || !data ? (
        <div className="grid gap-3 sm:grid-cols-3">
          <Skeleton className="h-20" />
          <Skeleton className="h-20" />
          <Skeleton className="h-20" />
        </div>
      ) : (
        <>
          <div className="grid gap-3 sm:grid-cols-3">
            <Stat label={t("stats.stat.listened")} value={formatDuration(data.total_seconds)} hint={t(RANGES.find((r) => r.value === range)?.label ?? "stats.range.month")} />
            <Stat label={t("stats.stat.streak")} value={t("stats.stat.streakValue", { count: data.streak_days })} hint={t("stats.stat.streakHint")} />
            <Stat label={t("stats.stat.booksFinished")} value={data.completed_items.toLocaleString(locale)} hint={t("stats.stat.booksFinishedHint")} />
          </div>

          <button
            type="button"
            onClick={() => setWrappedOpen(true)}
            className="mt-4 flex w-full items-center gap-4 rounded-card border border-border bg-card p-4 text-left transition-colors hover:bg-bg-alt"
          >
            <span className="flex h-11 w-11 shrink-0 items-center justify-center rounded-pill bg-accent text-on-accent">
              <Sparkles className="h-5 w-5" />
            </span>
            <span className="min-w-0 flex-1">
              <span className="block font-semibold">{t("stats.wrappedCard.title")}</span>
              <span className="block text-sm text-muted">{t("stats.wrappedCard.description")}</span>
            </span>
          </button>

          {data.total_seconds === 0 ? (
            <div className="mt-6">
              <EmptyState
                icon={<Flame />}
                title={t("stats.empty.title")}
                description={t("stats.empty.description")}
              />
            </div>
          ) : (
            <div className="mt-4 space-y-3">
              <ListeningChart horizon={RANGES.find((r) => r.value === range)?.horizon ?? "month"} />
              <KindBars byKind={data.by_kind} total={data.total_seconds} />
            </div>
          )}

          {topItems.length > 0 && (
            <section className="mt-8">
              <h2 className="mb-3 text-lg font-semibold tracking-tight">{t("stats.top.title")}</h2>
              <div className="divide-y divide-border rounded-card border border-border">
                {topItems.map((item, i) => (
                  <div key={`${item.item_id}-${i}`} className="flex items-center gap-3 px-4 py-2.5">
                    <span className="w-5 text-right text-xs tabular-nums text-muted">{i + 1}</span>
                    <span className="min-w-0 flex-1 truncate text-sm">{item.title ?? t("stats.deletedItem")}</span>
                    <span className={cn("text-xs", KIND_STYLE[item.media_kind]?.text ?? "text-muted")}>{kindName(item.media_kind)}</span>
                    <span className="w-20 text-right text-sm tabular-nums text-muted">{formatDuration(item.seconds)}</span>
                  </div>
                ))}
              </div>
            </section>
          )}
        </>
      )}

      <section className="mt-8">
        <h2 className="mb-3 text-lg font-semibold tracking-tight">{t("stats.lastYear.title")}</h2>
        <ActivityWidget withTotal />
      </section>

      <FamilyStats range={range} />
      <WrappedDialog open={wrappedOpen} onOpenChange={setWrappedOpen} />

      <section className="mt-8">
        <h2 className="mb-3 text-lg font-semibold tracking-tight">{t("stats.history.title")}</h2>
        {historyLoading ? (
          <Skeleton className="h-40" />
        ) : history.length === 0 ? (
          <p className="text-sm text-muted">{t("stats.history.empty")}</p>
        ) : (
          <>
            <div className="divide-y divide-border rounded-card border border-border">
              {history.map((h) => (
                <div key={h.id} className="flex items-center gap-3 px-4 py-2.5">
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm">{h.title ?? t("stats.deletedItem")}</span>
                    <span className="block text-xs text-muted">
                      {/* "derived" means the server inferred this from a progress
                          save rather than the client reporting it. */}
                      {t(h.source === "derived" ? "stats.history.lineEstimated" : "stats.history.line", {
                        when: new Date(h.started_at).toLocaleString(locale),
                        device: deviceLabel(h.device_kind),
                      })}
                    </span>
                  </span>
                  <span className="text-sm tabular-nums text-muted">{formatDuration(h.seconds)}</span>
                </div>
              ))}
            </div>
            {history.length >= historyLimit && (
              <Button variant="secondary" size="sm" className="mt-3" onClick={() => setHistoryLimit((n) => n + 50)}>
                {t("common.action.showMore")}
              </Button>
            )}
          </>
        )}
      </section>

      <section className="mt-8 rounded-card border border-border p-4">
        <p className="text-sm font-medium">{t("stats.visibility.title")}</p>
        <p className="mt-0.5 text-xs text-muted">
          {t("stats.visibility.description")}
        </p>
        <div className="mt-3 flex gap-2">
          <Button size="sm" variant="secondary" loading={visibility.isPending} onClick={() => visibility.mutate("family_admin")}>
            {t("stats.visibility.share")}
          </Button>
          <Button size="sm" variant="ghost" loading={visibility.isPending} onClick={() => visibility.mutate("private")}>
            {t("stats.visibility.keepPrivate")}
          </Button>
        </div>
      </section>
    </Page>
  );
}
