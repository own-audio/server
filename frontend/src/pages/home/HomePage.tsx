// SPDX-License-Identifier: AGPL-3.0-or-later
import { useLayoutEffect, useMemo, useRef, useState, useSyncExternalStore, type ReactNode } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { ArrowDown, ArrowUp, Play, SlidersHorizontal } from "lucide-react";
import { getContinueListening } from "../../api/library";
import { listBooks } from "../../api/audiobooks";
import { listPlaylists, listTracks } from "../../api/music";
import { getFeed, listEpisodes, listFeeds } from "../../api/podcasts";
import { listHistory } from "../../api/stats";
import { formatBytes, formatMicro, getBilling, getFamilyStorage } from "../../api/billing";
import { useServerFeatures } from "../../lib/features";
import { useAuthStore } from "../../store/authStore";
import { usePlayerStore } from "../../store/playerStore";
import { listTranslationsInProgress, subscribeToTranslationProgress } from "../../lib/translationProgress";
import { formatDuration } from "../../lib/format";
import { playBook, playEpisode, playTracks } from "../../lib/play";
import { Page } from "../../components/shell/SplitView";
import { Button, IconButton, Cover, Dialog, DialogContent, EmptyState, Skeleton } from "../../components/ui";
import { MediaCard } from "../../components/library/MediaCard";
import { albumKey } from "../music/albumKey";
import { ALL_WIDGETS, useHomeWidgets, type HomeWidgetId } from "./widgets";
import { GetStarted } from "./GetStarted";
import { ActivityWidget } from "./ActivityWidget";
import { WeekSummaryWidget } from "./WeekSummaryWidget";
import { QuickLinksWidget } from "./QuickLinksWidget";
import { ListeningChart } from "../stats/ListeningChart";
import type { AudioBook, ContinueItem, MusicTrack } from "../../api/types";
import { cn } from "../../lib/cn";
import { useT, type MessageKey, type PlainKey } from "../../i18n";

function Section({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  return (
    <section className="mb-9">
      {(title || action) && (
        <div className="mb-3 flex items-center justify-between">
          <h2 className="text-lg font-semibold tracking-tight">{title}</h2>
          {action}
        </div>
      )}
      {children}
    </section>
  );
}

function Carousel({ children }: { children: ReactNode }) {
  return <div className="-mx-1.5 flex gap-1 overflow-x-auto pb-1 [scrollbar-width:thin]">{children}</div>;
}

function EqualizerBars() {
  return (
    <span className="flex h-3 items-end gap-0.5">
      <span className="w-0.5 animate-pulse bg-current" style={{ height: 8 }} />
      <span className="w-0.5 animate-pulse bg-current" style={{ height: 12, animationDelay: "0.2s" }} />
      <span className="w-0.5 animate-pulse bg-current" style={{ height: 6, animationDelay: "0.4s" }} />
    </span>
  );
}

/* One row of Continue listening, whatever it came from. A translated episode is in here
   because the server cannot hold it: it keeps no position for a translation (guide 11a), so
   this browser's own memory is the only record there is. */
interface ContinueRow {
  id: string;
  title: string;
  subtitle: string | null;
  positionSecs: number;
  totalSecs: number | null;
  updatedAt: number;
  link: string;
  coverKind: "audiobook" | "podcast";
  coverUrl: string | null;
  isActive: boolean;
  resume: () => void | Promise<void>;
}

/** `booksOnly` is the Audiobooks in progress widget: the same rows, books only. */
function ContinueWidget({ booksOnly = false }: { booksOnly?: boolean }) {
  const { t } = useT();
  const { data: items, isLoading } = useQuery({ queryKey: ["continue-listening"], queryFn: getContinueListening });
  const { data: books = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const { data: feeds = [] } = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const play = usePlayerStore((s) => s.play);
  const setPlaying = usePlayerStore((s) => s.setPlaying);
  const [busy, setBusy] = useState<string | null>(null);

  // Re-read whenever a position is saved, so a translation's row follows the player.
  useSyncExternalStore(subscribeToTranslationProgress, () => localStorage.getItem("audio2.translationProgress.v1") ?? "");

  async function resume(item: ContinueItem) {
    const id = item.kind === "Book" ? item.book_id : item.episode_id;
    setBusy(id);
    try {
      if (item.kind === "Book") {
        const book = books.find((b) => b.id === item.book_id);
        if (book) await playBook(book);
      } else {
        const [feed, episodes] = await Promise.all([getFeed(item.feed_id), listEpisodes(item.feed_id, 200)]);
        const ep = episodes.find((e) => e.id === item.episode_id);
        if (ep) await playEpisode(item.feed_id, ep, feed);
      }
    } finally {
      setBusy(null);
    }
  }

  if (isLoading) {
    return (
      <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
        <Skeleton className="h-20" />
        <Skeleton className="h-20" />
        <Skeleton className="h-20" />
      </div>
    );
  }
  const serverRows: ContinueRow[] = (items ?? []).filter((item) => !booksOnly || item.kind === "Book").map((item) => {
    const isBook = item.kind === "Book";
    const id = isBook ? item.book_id : item.episode_id;
    const book = isBook ? books.find((b) => b.id === item.book_id) : undefined;
    const feed = isBook ? undefined : feeds.find((f) => f.id === item.feed_id);
    return {
      id,
      title: isBook ? item.book_title : item.episode_title,
      subtitle: isBook ? item.author : item.feed_title,
      positionSecs: item.position_secs,
      totalSecs: isBook ? item.total_duration_secs : item.duration_secs,
      updatedAt: Date.parse(item.updated_at),
      link: isBook ? `/audiobooks/${item.book_id}` : `/podcasts/${item.feed_id}`,
      coverKind: isBook ? "audiobook" : "podcast",
      coverUrl: isBook ? (book?.cover_url ?? null) : (feed?.image_url ?? null),
      isActive: isBook
        ? current?.kind === "audiobook" && current.bookId === id
        : current?.kind === "podcast" && !current.translationId && current.epId === id,
      resume: () => resume(item),
    };
  });

  const translationRows: ContinueRow[] = (booksOnly ? [] : listTranslationsInProgress()).map((entry) => ({
    id: entry.translationId,
    title: entry.episodeTitle,
    subtitle: t("home.continue.translated", { show: entry.showTitle ?? t("home.continue.podcastFallback"), language: entry.targetLanguage.toUpperCase() }),
    positionSecs: entry.positionSecs,
    totalSecs: entry.durationSecs,
    updatedAt: entry.updatedAt,
    link: "/podcasts",
    coverKind: "podcast",
    coverUrl: null,
    isActive: current?.kind === "podcast" && current.translationId === entry.translationId,
    resume: () => {
      if (current?.kind === "podcast" && current.translationId === entry.translationId) {
        setPlaying(!playing);
        return;
      }
      if (!entry.streamUrl) return;
      play({
        kind: "podcast",
        epId: entry.episodeId,
        translationId: entry.translationId,
        feedId: "",
        title: entry.episodeTitle,
        feedTitle: entry.showTitle,
        imageUrl: null,
        streamUrl: entry.streamUrl,
        durationSecs: entry.durationSecs,
        resumePosition: entry.positionSecs,
      });
    },
  }));

  const rows = [...serverRows, ...translationRows].sort((a, b) => b.updatedAt - a.updatedAt);
  if (rows.length === 0)
    return <p className="text-sm text-muted">{booksOnly ? t("home.continue.noBook") : t("home.continue.nothing")}</p>;

  return (
    <div className="grid grid-cols-1 gap-2 sm:grid-cols-2 xl:grid-cols-3">
      {rows.slice(0, 6).map((row) => {
        const pct = row.totalSecs ? Math.min(100, Math.round((row.positionSecs / row.totalSecs) * 100)) : null;

        return (
          <div key={row.id} className="flex items-center gap-3 rounded-card border border-border bg-card p-2.5 pr-3">
            <Link to={row.link} className="shrink-0" aria-label={row.title}>
              <Cover kind={row.coverKind} src={row.coverUrl} alt={row.title} aspect="square" className="h-14 w-14 rounded-lg" />
            </Link>
            <div className="min-w-0 flex-1">
              <Link to={row.link} className={cn("block truncate text-sm font-medium leading-tight hover:underline", row.isActive && "text-accent")}>
                {row.title}
              </Link>
              {row.subtitle && <p className="truncate text-xs text-muted">{row.subtitle}</p>}
              <div className="mt-1.5 flex items-center gap-2">
                <div className="h-1 flex-1 rounded-pill bg-border">
                  <div className="h-full rounded-pill bg-accent" style={{ width: `${pct ?? 15}%` }} />
                </div>
                <span className="text-[11px] tabular-nums text-muted">
                  {pct != null ? t("stats.wrapped.share", { share: pct / 100 }) : formatDuration(Math.round(row.positionSecs))}
                </span>
              </div>
            </div>
            <IconButton
              label={row.isActive && playing ? t("common.action.pause") : t("common.action.resume")}
              onClick={() => void row.resume()}
              disabled={busy === row.id}
              tone="accent"
            >
              {row.isActive && playing ? <EqualizerBars /> : <Play className="h-4 w-4 translate-x-px fill-current" />}
            </IconButton>
          </div>
        );
      })}
    </div>
  );
}

/* Recent songs and albums come from the listening history, deduplicated to one
   entry per item with the most recent first — the same derivation the Mac uses,
   since a play session records the track, not the album or playlist. */
function useRecentTracks() {
  const history = useQuery({ queryKey: ["history", 100], queryFn: () => listHistory(100) });
  const tracks = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks });

  const recent = useMemo(() => {
    if (!history.data || !tracks.data) return [];
    const byId = new Map(tracks.data.map((t) => [t.id, t]));
    const seen = new Set<string>();
    const out: MusicTrack[] = [];
    for (const h of history.data) {
      if (h.media_kind !== "music" || seen.has(h.item_id)) continue;
      const t = byId.get(h.item_id);
      if (t) {
        out.push(t);
        seen.add(h.item_id);
      }
    }
    return out;
  }, [history.data, tracks.data]);

  return { recent, all: tracks.data ?? [], isLoading: history.isLoading || tracks.isLoading };
}

function CardSkeletons() {
  return (
    <Carousel>
      {[0, 1, 2, 3, 4].map((i) => (
        <Skeleton key={i} className="h-44 w-36 shrink-0" />
      ))}
    </Carousel>
  );
}

function RecentSongsWidget() {
  const { t } = useT();
  const { recent, isLoading } = useRecentTracks();
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);

  if (isLoading) return <CardSkeletons />;
  if (recent.length === 0) return <p className="text-sm text-muted">{t("home.recentSongs.empty")}</p>;

  const shown = recent.slice(0, 12);
  return (
    <Carousel>
      {shown.map((track, i) => {
        const active = current?.kind === "music" && current.trackId === track.id;
        return (
          <MediaCard
            key={track.id}
            kind="music"
            title={track.title}
            subtitle={track.artist}
            cover={track.cover_url}
            active={active}
            playing={active && playing}
            onPlay={() => void playTracks(shown, i)}
            onClick={() => void playTracks(shown, i)}
            className="w-36 shrink-0"
          />
        );
      })}
    </Carousel>
  );
}

function RecentAlbumsWidget() {
  const { t } = useT();
  const { recent, all, isLoading } = useRecentTracks();
  const navigate = useNavigate();

  const albums = useMemo(() => {
    const seen = new Set<string>();
    const out: { artist: string; album: string; cover: string | null }[] = [];
    for (const track of recent) {
      if (!track.album) continue;
      const artist = track.artist ?? t("home.recentAlbums.unknownArtist");
      const key = albumKey(artist, track.album);
      if (seen.has(key)) continue;
      seen.add(key);
      const cover = track.cover_url ?? all.find((x) => x.album === track.album && x.artist === track.artist && x.cover_url)?.cover_url ?? null;
      out.push({ artist, album: track.album, cover });
    }
    return out.slice(0, 12);
  }, [recent, all, t]);

  if (isLoading) return <CardSkeletons />;
  if (albums.length === 0) return <p className="text-sm text-muted">{t("home.recentAlbums.empty")}</p>;

  return (
    <Carousel>
      {albums.map((a) => (
        <MediaCard
          key={albumKey(a.artist, a.album)}
          kind="music"
          title={a.album}
          subtitle={a.artist}
          cover={a.cover}
          onClick={() => navigate(`/music/albums/${encodeURIComponent(albumKey(a.artist, a.album))}`)}
          className="w-36 shrink-0"
        />
      ))}
    </Carousel>
  );
}

function RecentPlaylistsWidget() {
  const { t } = useT();
  const { data, isLoading } = useQuery({ queryKey: ["playlists"], queryFn: listPlaylists });
  const navigate = useNavigate();

  if (isLoading) return <CardSkeletons />;
  const sorted = [...(data ?? [])].sort((a, b) => b.updated_at.localeCompare(a.updated_at)).slice(0, 12);
  if (sorted.length === 0) return <p className="text-sm text-muted">{t("home.recentPlaylists.empty")}</p>;

  return (
    <Carousel>
      {sorted.map((p) => (
        <MediaCard
          key={p.id}
          kind="music"
          title={p.name}
          subtitle={t("home.playlist.songCount", { count: p.track_count })}
          cover={p.cover_url}
          onClick={() => navigate(`/music/playlists/${p.id}`)}
          className="w-36 shrink-0"
        />
      ))}
    </Carousel>
  );
}

function NewBooksWidget() {
  const { t } = useT();
  const { data, isLoading } = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const navigate = useNavigate();
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);

  const books = useMemo(() => [...(data ?? [])].sort((a, b) => b.created_at.localeCompare(a.created_at)).slice(0, 12), [data]);

  if (isLoading) return <CardSkeletons />;
  if (books.length === 0) return <p className="text-sm text-muted">{t("home.newBooks.empty")}</p>;
  return (
    <Carousel>
      {books.map((b) => {
        const active = current?.kind === "audiobook" && current.bookId === b.id;
        return (
          <MediaCard
            key={b.id}
            kind="audiobook"
            title={b.title}
            subtitle={b.author}
            cover={b.cover_url}
            active={active}
            playing={active && playing}
            onPlay={() => void playBook(b)}
            onClick={() => navigate(`/audiobooks/${b.id}`)}
            className="w-36 shrink-0"
          />
        );
      })}
    </Carousel>
  );
}

/* Recently played books come from the listening history, one entry per book,
   newest first — as Recent songs does for music. */
function RecentBooksWidget() {
  const { t } = useT();
  const history = useQuery({ queryKey: ["history", 100], queryFn: () => listHistory(100) });
  const booksQuery = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const navigate = useNavigate();

  const books = useMemo(() => {
    if (!history.data || !booksQuery.data) return [];
    const byId = new Map(booksQuery.data.map((b) => [b.id, b]));
    const seen = new Set<string>();
    const out: AudioBook[] = [];
    for (const h of history.data) {
      if (h.media_kind !== "audiobook" || seen.has(h.item_id)) continue;
      const b = byId.get(h.item_id);
      if (b) {
        out.push(b);
        seen.add(h.item_id);
      }
    }
    return out.slice(0, 12);
  }, [history.data, booksQuery.data]);

  if (history.isLoading || booksQuery.isLoading) return <CardSkeletons />;
  if (books.length === 0) return <p className="text-sm text-muted">{t("home.recentBooks.empty")}</p>;
  return (
    <Carousel>
      {books.map((b) => (
        <MediaCard
          key={b.id}
          kind="audiobook"
          title={b.title}
          subtitle={b.author}
          cover={b.cover_url}
          onPlay={() => void playBook(b)}
          onClick={() => navigate(`/audiobooks/${b.id}`)}
          className="w-36 shrink-0"
        />
      ))}
    </Carousel>
  );
}

/* There is no "newest across every show" endpoint, so this asks the most
   recently refreshed shows for their latest few episodes and merges them. A
   cap keeps a large subscription list from firing dozens of requests. */
const LATEST_FEEDS = 20;

function LatestEpisodesWidget() {
  const { t } = useT();
  const { data: feeds, isLoading: feedsLoading } = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });
  const navigate = useNavigate();

  const recentFeeds = useMemo(
    () =>
      [...(feeds ?? [])]
        .sort((a, b) => (b.last_refreshed_at ?? "").localeCompare(a.last_refreshed_at ?? ""))
        .slice(0, LATEST_FEEDS),
    [feeds]
  );

  const { data: episodes, isLoading } = useQuery({
    queryKey: ["latest-episodes", recentFeeds.map((f) => f.id)],
    enabled: recentFeeds.length > 0,
    queryFn: async () => {
      const lists = await Promise.all(
        recentFeeds.map((f) => listEpisodes(f.id, 3).then((eps) => eps.map((ep) => ({ ep, feed: f })), () => []))
      );
      return lists
        .flat()
        .filter((x) => x.ep.published_at)
        .sort((a, b) => b.ep.published_at!.localeCompare(a.ep.published_at!))
        .slice(0, 12);
    },
  });

  if (feedsLoading || (recentFeeds.length > 0 && isLoading)) return <CardSkeletons />;
  if (!episodes || episodes.length === 0) return <p className="text-sm text-muted">{t("home.latestEpisodes.empty")}</p>;
  return (
    <Carousel>
      {episodes.map(({ ep, feed }) => (
        <MediaCard
          key={ep.id}
          kind="podcast"
          title={ep.title}
          subtitle={feed.title}
          cover={ep.image_url ?? feed.image_url}
          completed={ep.completed}
          onPlay={() => void playEpisode(feed.id, ep, feed)}
          onClick={() => navigate(`/podcasts/${feed.id}`)}
          className="w-36 shrink-0"
        />
      ))}
    </Carousel>
  );
}

function StorageWidget() {
  const { t } = useT();
  const { features } = useServerFeatures();
  const storage = useQuery({ queryKey: ["family-storage"], queryFn: getFamilyStorage, retry: false });
  // Credit and runway exist only where the server bills; elsewhere the widget is bytes alone.
  const billing = useQuery({ queryKey: ["billing"], queryFn: getBilling, retry: false, enabled: features.billing });
  if (storage.isLoading || (features.billing && billing.isLoading)) return <Skeleton className="h-20" />;
  const credit = features.billing ? billing.data : undefined;
  // A hosted server from before `/family/storage` existed still reports the
  // bytes inside `/family/billing`; take them from there until it is upgraded.
  const bytes = storage.data ?? credit?.storage;
  if (!bytes) return <p className="text-sm text-muted">{t("home.storage.unavailable")}</p>;

  return (
    <div className={cn("grid grid-cols-1 gap-2", credit && "sm:grid-cols-3")}>
      <div className="rounded-card border border-border px-4 py-3">
        <p className="text-xs uppercase tracking-wide text-muted">{t("home.storage.stored")}</p>
        <p className="mt-1 text-xl font-semibold tabular-nums">{formatBytes(bytes.total_bytes)}</p>
      </div>
      {credit && (
        <>
          <div className="rounded-card border border-border px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-muted">{t("home.storage.credit")}</p>
            <p className={cn("mt-1 text-xl font-semibold tabular-nums", credit.balance_micro <= 0 && "text-error")}>
              {formatMicro(credit.balance_micro, credit.pricing.currency)}
            </p>
          </div>
          <div className="rounded-card border border-border px-4 py-3">
            <p className="text-xs uppercase tracking-wide text-muted">{t("home.storage.daysLeft")}</p>
            <p className="mt-1 text-xl font-semibold tabular-nums">{credit.days_remaining ?? "—"}</p>
          </div>
          <Link to="/billing" className="text-sm text-muted hover:text-foreground sm:col-span-3">
            {t("common.action.seeMore")}
          </Link>
        </>
      )}
    </div>
  );
}

/* An empty title means no heading: Quick links explain themselves. Titles are
   catalog keys, translated where the section renders. */
const WIDGETS: Record<HomeWidgetId, { title?: PlainKey; render: () => ReactNode; more?: string; moreLabel?: PlainKey }> = {
  quickLinks: { render: () => <QuickLinksWidget /> },
  continue: { title: "home.widget.continue.label", render: () => <ContinueWidget /> },
  // "See more" lives inside the widget: it points at Billing, which only some servers have.
  storage: { title: "home.widget.storage.label", render: () => <StorageWidget /> },
  recentSongs: { title: "home.widget.recentSongs.label", render: () => <RecentSongsWidget />, more: "/music" },
  recentAlbums: { title: "home.widget.recentAlbums.label", render: () => <RecentAlbumsWidget />, more: "/music/albums" },
  recentPlaylists: { title: "home.widget.recentPlaylists.label", render: () => <RecentPlaylistsWidget />, more: "/music/playlists" },
  booksInProgress: { title: "home.widget.booksInProgress.label", render: () => <ContinueWidget booksOnly /> },
  newBooks: { title: "home.widget.newBooks.label", render: () => <NewBooksWidget />, more: "/audiobooks" },
  recentBooks: { title: "home.widget.recentBooks.label", render: () => <RecentBooksWidget />, more: "/audiobooks" },
  latestEpisodes: { title: "home.widget.latestEpisodes.label", render: () => <LatestEpisodesWidget />, more: "/podcasts" },
  weekSummary: { title: "home.widget.weekSummary.label", render: () => <WeekSummaryWidget />, more: "/stats", moreLabel: "common.action.seeMore" },
  listeningChart: { title: "home.widget.listeningChart.label", render: () => <ListeningChart horizon="week" compact />, more: "/stats", moreLabel: "common.action.seeMore" },
  activity: { title: "home.widget.activity.label", render: () => <ActivityWidget />, more: "/stats", moreLabel: "common.action.seeMore" },
};

const REORDER_EASE = "cubic-bezier(0.22, 1, 0.36, 1)";

/* Rows are listed the way Home shows them — the shown widgets in their order,
   then the hidden ones — so moving one moves it here too. A moved row slides
   from where it was (FLIP) rather than jumping, and the arrow that moved it
   keeps focus so it can be pressed again. */
function WidgetSettings({ open, onOpenChange }: { open: boolean; onOpenChange: (v: boolean) => void }) {
  const { t } = useT();
  const { enabled, toggle, move } = useHomeWidgets();
  const rows = useRef(new Map<HomeWidgetId, HTMLLIElement>());
  const tops = useRef(new Map<HomeWidgetId, number>());
  const refocus = useRef<{ id: HomeWidgetId; dir: -1 | 1 } | null>(null);

  const shown = enabled.map((id) => ALL_WIDGETS.find((w) => w.id === id)).filter((w) => w != null);
  const hidden = ALL_WIDGETS.filter((w) => !enabled.includes(w.id));

  // Where every row is just before a change. offsetTop, not the on-screen
  // position: the sheet's own slide-in would otherwise read as rows moving.
  function snapshot() {
    tops.current = new Map([...rows.current].map(([id, el]) => [id, el.offsetTop]));
  }

  useLayoutEffect(() => {
    const before = tops.current;
    tops.current = new Map();
    const still = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    for (const [id, el] of rows.current) {
      const was = before.get(id);
      const now = el.offsetTop;
      if (still || was == null || was === now) continue;
      el.animate([{ transform: `translateY(${was - now}px)` }, { transform: "translateY(0)" }], {
        duration: 260,
        easing: REORDER_EASE,
      });
    }
    const r = refocus.current;
    refocus.current = null;
    if (r) {
      const row = rows.current.get(r.id);
      (row?.querySelector<HTMLButtonElement>(`[data-dir="${r.dir}"]:not(:disabled)`) ?? row?.querySelector<HTMLButtonElement>("[data-dir]:not(:disabled)"))?.focus();
    }
  });

  function moveRow(id: HomeWidgetId, dir: -1 | 1) {
    snapshot();
    refocus.current = { id, dir };
    move(id, dir);
  }

  function toggleRow(id: HomeWidgetId) {
    snapshot();
    toggle(id);
  }

  const row = (w: (typeof ALL_WIDGETS)[number]) => {
    const idx = enabled.indexOf(w.id);
    const on = idx >= 0;
    return (
      <li
        key={w.id}
        ref={(el) => {
          if (el) rows.current.set(w.id, el);
          else rows.current.delete(w.id);
        }}
        className="flex items-center gap-3 rounded-[10px] bg-card px-2 py-2 hover:bg-bg-alt"
      >
        <input
          id={`widget-${w.id}`}
          type="checkbox"
          checked={on}
          onChange={() => toggleRow(w.id)}
          className="h-4 w-4 accent-[var(--accent)]"
        />
        <label htmlFor={`widget-${w.id}`} className="min-w-0 flex-1 cursor-pointer">
          <span className={cn("block text-sm font-medium", !on && "text-muted")}>{t(w.label)}</span>
          <span className="block text-xs text-muted">{t(w.description)}</span>
        </label>
        {on && (
          <span className="flex">
            <IconButton size="sm" label={t("home.settings.moveUp")} data-dir={-1} disabled={idx === 0} onClick={() => moveRow(w.id, -1)}>
              <ArrowUp className="h-4 w-4" />
            </IconButton>
            <IconButton size="sm" label={t("home.settings.moveDown")} data-dir={1} disabled={idx === enabled.length - 1} onClick={() => moveRow(w.id, 1)}>
              <ArrowDown className="h-4 w-4" />
            </IconButton>
          </span>
        )}
      </li>
    );
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={t("home.settings.title")} description={t("home.settings.description")} variant="sheet">
        <ul className="space-y-1">
          {shown.map(row)}
          {hidden.length > 0 && shown.length > 0 && (
            <li key="hidden-label" aria-hidden="true" className="px-2 pb-1 pt-4 text-[11px] font-semibold uppercase tracking-wider text-muted">
              {t("home.settings.notOnHome")}
            </li>
          )}
          {hidden.map(row)}
        </ul>
      </DialogContent>
    </Dialog>
  );
}

type DayPart = "night" | "morning" | "afternoon" | "evening";
const GREETING: Record<DayPart, PlainKey> = {
  night: "home.greeting.night",
  morning: "home.greeting.morning",
  afternoon: "home.greeting.afternoon",
  evening: "home.greeting.evening",
};
const GREETING_NAMED: Record<DayPart, Extract<MessageKey, `home.greeting.${string}Name`>> = {
  night: "home.greeting.nightName",
  morning: "home.greeting.morningName",
  afternoon: "home.greeting.afternoonName",
  evening: "home.greeting.eveningName",
};

export default function HomePage() {
  const user = useAuthStore((s) => s.user);
  const { enabled } = useHomeWidgets();
  const [settingsOpen, setSettingsOpen] = useState(false);

  const { t } = useT();
  const hour = new Date().getHours();
  const firstName = user?.display_name.split(" ")[0];
  const part = hour < 5 ? "night" : hour < 12 ? "morning" : hour < 18 ? "afternoon" : "evening";
  const greeting = firstName ? t(GREETING_NAMED[part], { name: firstName }) : t(GREETING[part]);

  return (
    <Page
      title={greeting}
      actions={
        <IconButton label={t("home.settings.title")} onClick={() => setSettingsOpen(true)}>
          <SlidersHorizontal className="h-4 w-4" />
        </IconButton>
      }
      width="max-w-6xl"
    >
      <GetStarted />

      {enabled.length === 0 && (
        <EmptyState
          title={t("home.empty.title")}
          description={t("home.empty.description")}
          action={
            <Button variant="secondary" onClick={() => setSettingsOpen(true)}>
              {t("home.empty.chooseWidgets")}
            </Button>
          }
        />
      )}

      {enabled.map((id) => {
        const w = WIDGETS[id];
        return (
          <Section
            key={id}
            title={w.title ? t(w.title) : ""}
            action={
              w.more && (
                <Link to={w.more} className="text-sm font-medium text-accent hover:underline">
                  {t(w.moreLabel ?? "common.action.seeAll")}
                </Link>
              )
            }
          >
            {w.render()}
          </Section>
        );
      })}

      <WidgetSettings open={settingsOpen} onOpenChange={setSettingsOpen} />
    </Page>
  );
}
