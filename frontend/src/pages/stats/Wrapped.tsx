// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useMemo, useState, type ReactNode } from "react";
import * as D from "@radix-ui/react-dialog";
import { useQuery } from "@tanstack/react-query";
import { ChevronLeft, ChevronRight, X } from "lucide-react";
import { getFamilyStats, getStats, listHistorySince } from "../../api/stats";
import { getFamily, isFamilyAdmin } from "../../api/family";
import { cn } from "../../lib/cn";
import {
  KIND_LABEL,
  bestMonth,
  busiestDay,
  dayParts,
  devices,
  familySummary,
  hoursText,
  longestStreak,
  topPerKind,
  weekdayNames,
  weekdays,
} from "./insights";
import { useT, type PlainKey } from "../../i18n";

/* A year of listening told as a story, one card at a time — tap the right of a
   card (or →) for the next, the left (or ←) for the previous. Every figure is
   derived from the stats the page already loads; see insights.ts. */

const KIND_COLOUR: Record<string, string> = { audiobook: "var(--book)", podcast: "var(--podcast)", music: "var(--music)" };
const TOP_NOUN: Record<string, PlainKey> = {
  audiobook: "stats.wrapped.favourites.book",
  podcast: "stats.wrapped.favourites.show",
  music: "stats.wrapped.favourites.song",
};
const WEEKDAY_ID = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];
const bold = (c: ReactNode) => <span className="font-semibold">{c}</span>;

interface Card {
  key: string;
  tint: string;
  eyebrow: string;
  body: ReactNode;
}

function Big({ children }: { children: ReactNode }) {
  return <p className="text-5xl font-bold leading-none tracking-tight tabular-nums sm:text-6xl">{children}</p>;
}

function Bars({ rows }: { rows: { label: string; share: number; colour: string; value?: string }[] }) {
  const { t } = useT();
  return (
    <div className="space-y-3">
      {rows.map((r) => (
        <div key={r.label}>
          <div className="mb-1 flex justify-between text-sm">
            <span className="font-medium">{r.label}</span>
            <span className="tabular-nums opacity-70">{r.value ?? t("stats.wrapped.share", { share: r.share })}</span>
          </div>
          <div className="h-2 rounded-pill bg-fg/10">
            <div className="h-full rounded-pill" style={{ width: `${Math.max(2, r.share * 100)}%`, background: r.colour }} />
          </div>
        </div>
      ))}
    </div>
  );
}

function useCards(open: boolean): { cards: Card[]; loading: boolean } {
  const { t, rich, locale } = useT();
  const stats = useQuery({ queryKey: ["stats", "365d"], queryFn: () => getStats("365d"), enabled: open });
  const history = useQuery({ queryKey: ["history-since", "365d"], queryFn: () => listHistorySince(new Date(Date.now() - 365 * 86_400_000)), enabled: open });
  const { data: family } = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false, enabled: open });
  const admin = isFamilyAdmin(family?.my_role);
  const familyStats = useQuery({
    queryKey: ["family-stats", "365d"],
    queryFn: () => getFamilyStats("365d"),
    enabled: open && admin,
    retry: false,
  });

  const cards = useMemo<Card[]>(() => {
    const s = stats.data;
    if (!s) return [];
    const longDate = new Intl.DateTimeFormat(locale, { weekday: "long", day: "numeric", month: "long" });
    const monthFmt = new Intl.DateTimeFormat(locale, { month: "long", year: "numeric" });
    if (s.total_seconds < 60) {
      return [
        {
          key: "empty",
          tint: "var(--accent)",
          eyebrow: t("stats.wrapped.lastYear"),
          body: <p className="text-2xl font-semibold">{t("stats.wrapped.empty")}</p>,
        },
      ];
    }

    const out: Card[] = [];
    const hours = Math.round(s.total_seconds / 3600);
    const days = Math.floor(s.total_seconds / 86_400);
    out.push({
      key: "total",
      tint: "var(--accent)",
      eyebrow: t("stats.wrapped.lastYear"),
      body: (
        <>
          <Big>{hours >= 1 ? t("common.duration.hours", { h: hours }) : hoursText(s.total_seconds)}</Big>
          <p className="mt-4 text-xl">{days >= 1 ? t("stats.wrapped.totalKindsDays", { days }) : t("stats.wrapped.totalKinds")}</p>
        </>
      ),
    });

    const kinds = s.by_kind.filter((k) => k.media_kind in KIND_LABEL).sort((a, b) => b.seconds - a.seconds);
    if (kinds.length > 0) {
      out.push({
        key: "mix",
        tint: KIND_COLOUR[kinds[0].media_kind],
        eyebrow: t("stats.wrapped.mix.eyebrow"),
        body: (
          <>
            <p className="mb-6 text-2xl font-semibold">{t("stats.wrapped.mix.mostly", { kind: kinds[0].media_kind })}</p>
            <Bars
              rows={kinds.map((k) => ({
                label: t(KIND_LABEL[k.media_kind]),
                share: k.seconds / s.total_seconds,
                colour: KIND_COLOUR[k.media_kind],
                value: t("stats.wrapped.timeShare", { time: hoursText(k.seconds), share: k.seconds / s.total_seconds }),
              }))}
            />
          </>
        ),
      });
    }

    const tops = topPerKind(s.top_items);
    const topEntries = (["audiobook", "podcast", "music"] as const).filter((k) => tops[k]);
    if (topEntries.length > 0) {
      out.push({
        key: "favourites",
        tint: KIND_COLOUR[topEntries[0]],
        eyebrow: t("stats.wrapped.favourites.eyebrow"),
        body: (
          <div className="space-y-6">
            {topEntries.map((k) => (
              <div key={k}>
                <p className="text-sm font-medium" style={{ color: KIND_COLOUR[k] }}>
                  {t(TOP_NOUN[k])}
                </p>
                <p className="text-2xl font-semibold leading-tight">{tops[k]!.title ?? t("stats.wrapped.favourites.deleted")}</p>
                <p className="mt-1 text-sm opacity-70">
                  {t("stats.wrapped.favourites.sessions", { time: hoursText(tops[k]!.seconds), count: tops[k]!.sessions })}
                </p>
              </div>
            ))}
          </div>
        ),
      });
    }

    const busiest = busiestDay(s.by_day);
    const month = bestMonth(s.by_day);
    const streak = longestStreak(s.by_day);
    out.push({
      key: "records",
      tint: "var(--accent)",
      eyebrow: t("stats.wrapped.records.eyebrow"),
      body: (
        <div className="space-y-5">
          {busiest && (
            <div>
              <p className="text-sm opacity-70">{t("stats.wrapped.records.busiestDay")}</p>
              <p className="text-2xl font-semibold">{longDate.format(new Date(`${busiest.day}T12:00:00`))}</p>
              <p className="text-sm opacity-70">{t("stats.wrapped.records.inOneDay", { time: hoursText(busiest.seconds) })}</p>
            </div>
          )}
          {month && (
            <div>
              <p className="text-sm opacity-70">{t("stats.wrapped.records.bestMonth")}</p>
              <p className="text-2xl font-semibold">{monthFmt.format(new Date(`${month.month}-15T12:00:00`))}</p>
              <p className="text-sm opacity-70">{hoursText(month.seconds)}</p>
            </div>
          )}
          <div>
            <p className="text-sm opacity-70">{t("stats.wrapped.records.longestStreak")}</p>
            <p className="text-2xl font-semibold">{t("stats.wrapped.records.inARow", { count: streak.length })}</p>
            {s.streak_days > 0 && <p className="text-sm opacity-70">{t("stats.wrapped.records.currentStreak", { count: s.streak_days })}</p>}
          </div>
          {s.completed_items > 0 && (
            <div>
              <p className="text-sm opacity-70">{t("stats.wrapped.records.booksFinished")}</p>
              <p className="text-2xl font-semibold">{s.completed_items}</p>
            </div>
          )}
        </div>
      ),
    });

    const week = weekdays(s.by_day);
    const sessions = history.data ?? [];
    const parts = dayParts(sessions);
    const where = devices(sessions).slice(0, 4);
    // "Unrecognised apps" is a bucket, not a place — never the headline.
    const topDevice = where.find((w) => w.kind !== "other");
    const letters = weekdayNames("narrow");
    const weekMax = Math.max(1, ...week.totals);
    out.push({
      key: "rhythm",
      tint: "var(--podcast)",
      eyebrow: t("stats.wrapped.rhythm.eyebrow"),
      body: (
        <div className="space-y-6">
          {week.favouriteIndex != null && (
            <div>
              <p className="text-2xl font-semibold">{t("stats.wrapped.rhythm.weekday", { day: WEEKDAY_ID[week.favouriteIndex] })}</p>
              <div className="mt-3 flex h-16 items-end gap-1.5" aria-hidden="true">
                {week.totals.map((total, i) => (
                  <div key={i} className="flex h-full flex-1 flex-col items-center justify-end gap-1">
                    <div className="w-full rounded-t-sm bg-current opacity-80" style={{ height: `${Math.max(4, (total / weekMax) * 100)}%` }} />
                    <span className="text-[10px] opacity-60">{letters[i]}</span>
                  </div>
                ))}
              </div>
            </div>
          )}
          {parts.favourite && (
            <p className="text-lg">{rich("stats.wrapped.rhythm.dayPart", { part: parts.favourite, b: bold })}</p>
          )}
          {where.length > 0 && (
            <div>
              {topDevice && (
                <p className="mb-3 text-lg">
                  {rich("stats.wrapped.rhythm.device", {
                    lead: topDevice === where[0] ? "mostly" : "often",
                    device: topDevice.kind,
                    name: topDevice.label,
                    b: bold,
                  })}
                </p>
              )}
              <Bars rows={where.map((w) => ({ label: w.label, share: w.share, colour: "currentColor" }))} />
            </div>
          )}
          {sessions.length > 0 && <p className="text-xs opacity-60">{t("stats.wrapped.rhythm.footnote", { count: sessions.length })}</p>}
        </div>
      ),
    });

    const fam = familyStats.data ? familySummary(familyStats.data) : null;
    if (fam && fam.members.length > 1) {
      out.push({
        key: "family",
        tint: "var(--book)",
        eyebrow: t("stats.wrapped.family.eyebrow"),
        body: (
          <div className="space-y-6">
            <div>
              <Big>{t("common.duration.hours", { h: Math.round(fam.totalSeconds / 3600) })}</Big>
              <p className="mt-3 text-xl">{t("stats.wrapped.family.together", { count: fam.members.length })}</p>
            </div>
            <Bars rows={fam.members.slice(0, 5).map((m) => ({ label: m.name, share: m.share, colour: "var(--accent)" }))} />
            {fam.leaders.length > 0 && (
              <div className="space-y-1 text-sm">
                {fam.leaders.map((l) => (
                  <p key={l.kind}>{rich("stats.wrapped.family.leader", { kind: l.kind, name: l.name, b: bold })}</p>
                ))}
              </div>
            )}
            {fam.privateCount > 0 && (
              <p className="text-xs opacity-60">{t("stats.wrapped.family.private", { count: fam.privateCount })}</p>
            )}
          </div>
        ),
      });
    }

    out.push({
      key: "end",
      tint: "var(--accent)",
      eyebrow: t("stats.wrapped.end.eyebrow"),
      body: (
        <p className="text-3xl font-semibold leading-tight">
          {hours >= 1 ? t("stats.wrapped.end.hours", { hours }) : t("stats.wrapped.end.one")}
        </p>
      ),
    });
    return out;
  }, [stats.data, history.data, familyStats.data, t, rich, locale]);

  return { cards, loading: stats.isLoading };
}

export function WrappedDialog({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const { t } = useT();
  const { cards, loading } = useCards(open);
  const [index, setIndex] = useState(0);
  const [openedAt, setOpenedAt] = useState(open);
  // Start from the first card each time it opens (adjusted during render).
  if (open !== openedAt) {
    setOpenedAt(open);
    if (open) setIndex(0);
  }
  const card = cards[Math.min(index, cards.length - 1)];
  const last = index >= cards.length - 1;

  useEffect(() => {
    if (!open) return;
    function onKey(e: KeyboardEvent) {
      if (e.key === "ArrowRight") setIndex((i) => Math.min(i + 1, cards.length - 1));
      if (e.key === "ArrowLeft") setIndex((i) => Math.max(i - 1, 0));
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, cards.length]);

  return (
    <D.Root open={open} onOpenChange={onOpenChange}>
      <D.Portal>
        <D.Overlay className="fixed inset-0 z-50 bg-overlay data-[state=open]:animate-fade-in data-[state=closed]:animate-fade-out" />
        <D.Content
          aria-describedby={undefined}
          className="fixed inset-0 z-50 flex flex-col overflow-hidden bg-bg text-fg outline-none data-[state=open]:animate-pop-in data-[state=closed]:animate-pop-out sm:inset-auto sm:left-1/2 sm:top-1/2 sm:h-[min(760px,90vh)] sm:w-[420px] sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-sheet sm:shadow-pop"
        >
          <D.Title className="sr-only">{t("stats.wrapped.title")}</D.Title>
          {/* The card's colour washes the top of the page and fades out. */}
          <div
            aria-hidden="true"
            className="pointer-events-none absolute inset-x-0 top-0 h-2/3 transition-[background] duration-500"
            style={{ background: card ? `linear-gradient(180deg, color-mix(in srgb, ${card.tint} 28%, transparent), transparent)` : undefined }}
          />
          <div className="relative flex gap-1 px-4 pt-[max(1rem,env(safe-area-inset-top))]">
            {cards.map((c, i) => (
              <span key={c.key} className="h-1 flex-1 overflow-hidden rounded-pill bg-fg/15">
                <span className={cn("block h-full rounded-pill bg-fg transition-[width] duration-300", i <= index ? "w-full" : "w-0")} />
              </span>
            ))}
          </div>
          <div className="relative flex items-center justify-between px-3 pt-2">
            <span className="px-1 text-xs font-medium opacity-70">{t("stats.wrapped.brand")}</span>
            <D.Close aria-label={t("common.action.close")} className="inline-flex h-10 w-10 items-center justify-center rounded-pill hover:bg-fg/10">
              <X className="h-5 w-5" />
            </D.Close>
          </div>

          {/* Tapping the card steps through it like a story — the left third goes
              back. The buttons below do the same for keyboards and screen readers. */}
          <div
            className="relative flex min-h-0 flex-1 cursor-pointer select-none flex-col justify-center overflow-y-auto px-7 pb-6"
            aria-live="polite"
            onClick={(e) => {
              const r = e.currentTarget.getBoundingClientRect();
              if (e.clientX - r.left < r.width / 3) setIndex((i) => Math.max(0, i - 1));
              else setIndex((i) => Math.min(cards.length - 1, i + 1));
            }}
          >
            {loading || !card ? (
              <p className="text-lg opacity-70">{t("stats.wrapped.loading")}</p>
            ) : (
              <div key={card.key} className="animate-pop-in">
                <p className="mb-5 text-sm font-semibold uppercase tracking-widest" style={{ color: card.tint }}>
                  {card.eyebrow}
                </p>
                {card.body}
              </div>
            )}
          </div>

          <div className="relative flex items-center justify-between gap-3 px-4 pb-[max(1rem,env(safe-area-inset-bottom))]">
            <button
              type="button"
              aria-label={t("common.action.previous")}
              disabled={index === 0}
              onClick={() => setIndex((i) => Math.max(0, i - 1))}
              className="inline-flex h-11 w-11 items-center justify-center rounded-pill hover:bg-fg/10 disabled:opacity-30"
            >
              <ChevronLeft className="h-5 w-5" />
            </button>
            <span className="text-xs tabular-nums opacity-60">
              {cards.length > 0 ? t("stats.wrapped.counter", { index: index + 1, total: cards.length }) : ""}
            </span>
            {last ? (
              <D.Close className="inline-flex h-11 items-center rounded-pill bg-fg px-5 text-sm font-semibold text-bg">{t("common.action.done")}</D.Close>
            ) : (
              <button
                type="button"
                aria-label={t("common.action.next")}
                onClick={() => setIndex((i) => Math.min(cards.length - 1, i + 1))}
                className="inline-flex h-11 w-11 items-center justify-center rounded-pill bg-fg text-bg"
              >
                <ChevronRight className="h-5 w-5" />
              </button>
            )}
          </div>
        </D.Content>
      </D.Portal>
    </D.Root>
  );
}
