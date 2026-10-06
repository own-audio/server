// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState, useSyncExternalStore, type ReactNode } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Archive, Check, Compass, ChevronDown, ChevronUp, CircleCheck, CloudDownload, Download, ImageDown, Inbox, Languages, Loader2, Pause, Play, RefreshCw, Save, Undo2, UserCheck } from "lucide-react";
import { getFeed, listEpisodes, similarFeeds, subscribeFeed, syncImages, refreshFeed, setFeedVisibility, downloadEpisode, getStreamUrl, setAutoStore, storeAll, type StoreAllResult } from "../../api/podcasts";
import { bulkSetEpisodesPlayed } from "../../api/playback";
import { listTranslations } from "../../api/podcastTranslate";
import { formatDate, formatDuration, stripHtml } from "../../lib/format";
import { playEpisode } from "../../lib/play";
import { getTranslationProgress, subscribeToTranslationProgress } from "../../lib/translationProgress";
import { playTranslation } from "../../lib/playTranslation";
import { usePlayerStore } from "../../store/playerStore";
import { Button, IconButton, Cover, Dialog, DialogContent, EmptyState, MenuItem, Pill, SearchField, SegmentedControl, Skeleton, toast } from "../../components/ui";
import { MoreMenuTrigger } from "../../components/library/BrowseControls";
import { ClampedText } from "../../components/library/ClampedText";
import TranslateEpisodeModal from "./TranslateEpisodeModal";
import SimilarShowsDialog from "./SimilarShowsDialog";
import type { PodcastEpisode, PodcastEpisodeTranslation, Visibility } from "../../api/types";
import { VisibilityField } from "../../components/library/VisibilityField";
import AudienceList from "../../components/library/AudienceList";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";

/**
 * Shows like this one.
 *
 * Not behind the recommendations switch, and the distinction is
 * deliberate: this reads no listening history at all. It answers "what
 * is like this show" about a feed the user has open — something they
 * asked for by opening it — and the server filters out what the
 * household already follows before answering.
 *
 * Renders nothing at all when there is nothing to say. A feed the
 * catalogue has never seen returns an empty list, and an empty
 * "You might also like" heading is worse than no heading.
 */
function SimilarShows({ feedId, feedTitle }: { feedId: string; feedTitle: string }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [pending, setPending] = useState<string | null>(null);
  const [browsing, setBrowsing] = useState(false);

  const { data: similar = [], isLoading } = useQuery({
    queryKey: ["podcast-similar", feedId],
    queryFn: () => similarFeeds(feedId),
    staleTime: 60 * 60 * 1000,
    retry: false,
  });

  const follow = useMutation({
    mutationFn: (feedUrl: string) => subscribeFeed(feedUrl, "private"),
    onSuccess: (feed) => {
      qc.invalidateQueries({ queryKey: ["feeds"] });
      // The list is filtered against what the household follows, so the
      // row the user just acted on should leave it.
      qc.invalidateQueries({ queryKey: ["podcast-similar"] });
      toast.success(t("podcasts.action.following"), feed.title);
      setPending(null);
    },
    onError: () => { setPending(null); toast.error(t("podcasts.error.followShow")); },
  });

  if (isLoading || similar.length === 0) return null;

  return (
    <section className="mt-8 border-t border-border pt-6">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold">{t("podcasts.similar.inlineTitle")}</h2>
          <p className="mt-0.5 text-xs text-muted">
            {t("podcasts.similar.inlineBody")}
          </p>
        </div>
        {similar.length > 8 && (
          <Button size="sm" variant="ghost" onClick={() => setBrowsing(true)}>{t("podcasts.similar.seeAll", { count: similar.length })}</Button>
        )}
      </div>
      {browsing && <SimilarShowsDialog feedId={feedId} feedTitle={feedTitle} onClose={() => setBrowsing(false)} />}
      <ul className="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-2">
        {similar.slice(0, 8).map((r) => (
          <li key={r.feed_url} className="flex gap-3 rounded-card border border-border p-3">
            <Cover kind="podcast" src={r.image_url} alt="" auth={false} className="h-14 w-14 shrink-0" />
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm font-medium">{r.title}</p>
              {r.author && <p className="truncate text-xs text-muted">{r.author}</p>}
              <Button
                size="sm"
                variant="secondary"
                className="mt-1.5"
                loading={pending === r.feed_url && follow.isPending}
                onClick={() => { setPending(r.feed_url); follow.mutate(r.feed_url); }}
              >
                {t("podcasts.action.follow")}
              </Button>
            </div>
          </li>
        ))}
      </ul>
    </section>
  );
}

const STATUS_LABEL: Record<PodcastEpisodeTranslation["status"], PlainKey> = {
  translating: "podcasts.translation.status.translating",
  narrating: "podcasts.translation.status.narrating",
  assembling: "podcasts.translation.status.assembling",
  complete: "podcasts.translation.status.complete",
  failed: "podcasts.translation.status.failed",
};

/* Translations of one episode. The server's `notice` is rendered verbatim
   beside a finished one — the contract requires it on every surface that
   offers this feature, result rows included — and there is deliberately no
   share or download affordance here. */
function Translations({
  episodeId,
  episodeTitle,
  showTitle,
  episodeDurationSecs,
  onTranslate,
}: {
  episodeId: string;
  episodeTitle: string;
  showTitle: string | null;
  /** The original's length: a translation runs about as long, and a bar needs a scale. */
  episodeDurationSecs: number | null;
  onTranslate: () => void;
}) {
  const { t, rich } = useT();
  const { data: translations = [] } = useQuery({
    queryKey: ["podcast-translations", episodeId],
    queryFn: () => listTranslations(episodeId),
    refetchInterval: (q) => ((q.state.data ?? []).some((tr) => tr.status !== "complete" && tr.status !== "failed") ? 3000 : false),
  });

  // Re-read on every save so a row follows what the player is doing to it.
  const saved = useSyncExternalStore(subscribeToTranslationProgress, () => localStorage.getItem("audio2.translationProgress.v1") ?? "");
  void saved;

  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);

  const start = (tr: PodcastEpisodeTranslation) =>
    playTranslation(tr, { id: episodeId, title: episodeTitle, durationSecs: episodeDurationSecs }, showTitle);

  return (
    <div className="mt-3 border-t border-border pt-3">
      {translations.length > 0 && (
        <ul className="mb-2 space-y-1.5">
          {translations.map((tr) => {
            const known = getTranslationProgress(tr.id);
            const isCurrent = current?.kind === "podcast" && current.translationId === tr.id;
            const fraction =
              known && !known.completed && known.positionSecs > 0 && known.durationSecs
                ? Math.min(1, known.positionSecs / known.durationSecs)
                : null;
            const left =
              known && !known.completed && known.positionSecs > 0 && known.durationSecs
                ? formatDuration(Math.max(0, known.durationSecs - known.positionSecs))
                : null;
            return (
              <li key={tr.id}>
                <div className="flex items-center justify-between gap-3 text-sm">
                  <span className="flex items-center gap-2 text-muted">
                    {/* Its own start and stop, deliberately smaller than the episode's above:
                        a translation sits under the episode rather than beside it. */}
                    {tr.status === "complete" && tr.stream_url && (
                      <button
                        type="button"
                        onClick={() => start(tr)}
                        aria-label={isCurrent && playing ? t("common.action.pause") : t("common.action.play")}
                        className="flex h-6 w-6 items-center justify-center rounded-full border border-border text-fg hover:bg-surface-2"
                      >
                        {isCurrent && playing ? <Pause className="h-3 w-3 fill-current" /> : <Play className="h-3 w-3 fill-current" />}
                      </button>
                    )}
                    <Languages className="h-3.5 w-3.5" />
                    <span>
                      {rich("podcasts.translation.row", {
                        language: tr.target_language.toUpperCase(),
                        state: t(STATUS_LABEL[tr.status]),
                        status: (c) => <span className={tr.status === "failed" ? "text-error" : tr.status === "complete" ? "text-success-text" : "text-muted"}>{c}</span>,
                      })}
                      {known?.completed && tr.status === "complete" && ` — ${t("podcasts.translation.played")}`}
                      {left && ` — ${t("podcasts.translation.left", { time: left })}`}
                    </span>
                  </span>
                </div>
                {/* Its own position, kept in this browser: the server has none for a
                    translation (guide §11a). */}
                {fraction !== null && (
                  <div className="ml-8 mt-1 h-0.5 w-40 overflow-hidden rounded bg-surface-2">
                    <div className="h-full bg-accent" style={{ width: `${Math.round(fraction * 100)}%` }} />
                  </div>
                )}
                {tr.status === "failed" && (
                  <p className="mt-0.5 text-xs text-muted">{tr.error ? t("podcasts.translation.notChargedWithError", { error: tr.error }) : t("podcasts.translation.notCharged")}</p>
                )}
                {tr.status === "complete" && tr.notice && <p className="mt-0.5 text-xs text-muted">{tr.notice}</p>}
              </li>
            );
          })}
        </ul>
      )}
      <Button size="sm" variant="secondary" icon={<Languages className="h-4 w-4" />} onClick={onTranslate}>{t("podcasts.translation.translateAnother")}</Button>
    </div>
  );
}

/**
 * "Save archive": the server downloads a show's episodes and keeps them — all, or the newest
 * N — so a paid feed stays in the library after the subscription ends. Says how many and
 * roughly how much storage first.
 */
function StoreAllDialog({ feedId, title, onClose }: { feedId: string; title: string; onClose: () => void }) {
  const { t, rich, locale } = useT();
  const [scope, setScope] = useState<"newest" | "all">("newest");
  const [latest, setLatest] = useState(100);
  const [queued, setQueued] = useState<number | null>(null);
  const limit = scope === "newest" ? latest : undefined;
  const { data: preview } = useQuery<StoreAllResult>({
    queryKey: ["store-all-preview", feedId, limit ?? "all"],
    queryFn: () => storeAll(feedId, true, limit),
  });
  const save = useMutation({
    mutationFn: () => storeAll(feedId, false, limit),
    onSuccess: (r) => setQueued(r.episodes),
    onError: () => toast.error(t("podcasts.archive.error")),
  });
  const gb = (preview?.estimated_bytes ?? 0) / 1e9;
  const size =
    gb >= 1
      ? new Intl.NumberFormat(locale, { style: "unit", unit: "gigabyte", maximumFractionDigits: 1, minimumFractionDigits: 1 }).format(gb)
      : new Intl.NumberFormat(locale, { style: "unit", unit: "megabyte", maximumFractionDigits: 0 }).format(Math.round(gb * 1000));
  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("podcasts.archive.title", { title })}
        description={t("podcasts.archive.description")}
        footer={
          queued === null ? (
            <>
              <Button variant="ghost" onClick={onClose}>{t("common.action.cancel")}</Button>
              <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!preview?.episodes}>{t("common.action.save")}</Button>
            </>
          ) : (
            <Button onClick={onClose}>{t("common.action.done")}</Button>
          )
        }
      >
        <div className="space-y-3 text-sm">
          <SegmentedControl
            value={scope}
            onChange={setScope}
            segments={[{ value: "newest", label: t("podcasts.archive.scope.newest") }, { value: "all", label: t("podcasts.archive.scope.all") }]}
          />
          {scope === "newest" && (
            <label className="flex items-center gap-2">
              {rich("podcasts.archive.newestCount", {
                count: latest,
                input: () => (
                  <input
                    type="number" min={1} max={5000} value={latest}
                    onChange={(e) => setLatest(Math.max(1, Number(e.target.value) || 1))}
                    className="w-24 rounded-card border border-border bg-bg-alt px-2 py-1"
                  />
                ),
              })}
            </label>
          )}
          {queued !== null ? (
            <p>{t("podcasts.archive.queued", { count: queued })}</p>
          ) : preview ? (
            <p className="text-muted">
              {t("podcasts.archive.preview", { count: preview.episodes, size })}
            </p>
          ) : (
            <Skeleton className="h-5 w-48" />
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}

export default function FeedDetail({ feedId }: { feedId: string }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [expanded, setExpanded] = useState<string | null>(null);
  const [translating, setTranslating] = useState<string | null>(null);
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const [busy, setBusy] = useState<string | null>(null);

  const [storingAll, setStoringAll] = useState(false);
  const [audienceOpen, setAudienceOpen] = useState(false);
  const [similarOpen, setSimilarOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [query, setQuery] = useState("");
  // Wait for typing to pause before asking the server.
  useEffect(() => {
    const timer = setTimeout(() => setQuery(search.trim()), 300);
    return () => clearTimeout(timer);
  }, [search]);

  const { data: feed, isLoading } = useQuery({ queryKey: ["feed", feedId], queryFn: () => getFeed(feedId) });
  const { data: episodes = [], isLoading: episodesLoading } = useQuery({
    queryKey: ["episodes", feedId, query],
    queryFn: () => listEpisodes(feedId, 200, 0, query || undefined),
  });
  const autoStore = useMutation({
    mutationFn: (enabled: boolean) => setAutoStore(feedId, enabled),
    onSuccess: (updated) => {
      qc.setQueryData(["feed", feedId], updated);
      toast.success(updated.auto_store ? t("podcasts.feed.autoStore.on") : t("podcasts.feed.autoStore.off"));
    },
    onError: () => toast.error(t("podcasts.feed.autoStore.error")),
  });

  const sync = useMutation({
    mutationFn: () => syncImages(feedId),
    onSuccess: (updated) => { qc.setQueryData(["feed", feedId], updated); qc.invalidateQueries({ queryKey: ["episodes", feedId] }); },
  });
  const refresh = useMutation({
    mutationFn: () => refreshFeed(feedId),
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["episodes", feedId] }); toast.success(t("podcasts.toast.refreshed")); },
    onError: () => toast.error(t("podcasts.error.refresh")),
  });

  // The one bulk endpoint the API offers; everything else is N requests.
  const changeVisibility = useMutation({
    mutationFn: (v: Visibility) => setFeedVisibility(feedId, v),
    onSuccess: (_d, v) => {
      qc.invalidateQueries({ queryKey: ["feed", feedId] });
      qc.invalidateQueries({ queryKey: ["feeds"] });
      toast.success(v === "family" ? t("podcasts.feed.visibility.shared") : t("podcasts.feed.visibility.private"));
    },
    onError: () => toast.error(t("podcasts.feed.visibility.error")),
  });

  const setPlayed = useMutation({
    mutationFn: ({ ids, completed }: { ids: string[]; completed: boolean }) => bulkSetEpisodesPlayed(ids, completed),
    onSuccess: (updated, { completed }) => {
      qc.invalidateQueries({ queryKey: ["episodes", feedId] });
      qc.invalidateQueries({ queryKey: ["continue-listening"] });
      toast.success(completed ? t("podcasts.feed.markedPlayed", { count: updated }) : t("podcasts.feed.markedUnplayed", { count: updated }));
    },
    onError: () => toast.error(t("podcasts.feed.error.markPlayed")),
  });

  /* Fetching is normally a side effect of pressing play. Doing it on purpose
     is worth its own action: it is how you get an episode ready before a train
     journey, and the server has no progress to report, so the UI can only say
     honestly that it is working. */
  const fetchToServer = useMutation({
    mutationFn: (ep: PodcastEpisode) => downloadEpisode(feedId, ep.id),
    onSuccess: (updated) => {
      qc.setQueryData<PodcastEpisode[]>(["episodes", feedId, query], (prev) =>
        prev?.map((e) => (e.id === updated.id ? updated : e)) ?? [updated]
      );
      toast.success(t("podcasts.feed.readyToPlay"), updated.title);
    },
    onError: () => toast.error(t("podcasts.feed.error.fetch"), t("podcasts.feed.error.fetchBody")),
  });

  /** Hands the browser the presigned URL so the listener keeps a copy. */
  async function saveCopy(ep: PodcastEpisode) {
    try {
      const url = await getStreamUrl(feedId, ep.id);
      const a = document.createElement("a");
      a.href = url;
      a.download = `${ep.title}.mp3`;
      a.rel = "noopener";
      document.body.appendChild(a);
      a.click();
      a.remove();
    } catch {
      toast.error(t("podcasts.feed.error.download"));
    }
  }

  async function play(ep: PodcastEpisode) {
    setBusy(ep.id);
    try {
      const ready = await playEpisode(feedId, ep, feed);
      if (ready !== ep) qc.setQueryData<PodcastEpisode[]>(["episodes", feedId, query], (prev) => prev?.map((e) => (e.id === ready.id ? ready : e)) ?? [ready]);
    } catch {
      toast.error(t("podcasts.feed.error.play"));
    } finally {
      setBusy(null);
    }
  }

  const actions: { key: string; label: string; icon: ReactNode; onSelect: () => void; loading?: boolean; title?: string; phoneOnly?: boolean }[] = [];
  if (feed && !feed.image_url)
    actions.push({ key: "artwork", label: t("podcasts.feed.fetchArtwork"), icon: <ImageDown className="h-4 w-4" />, loading: sync.isPending, onSelect: () => sync.mutate() });
  if (feed?.is_owner)
    actions.push({
      key: "autoStore",
      label: feed.auto_store ? t("podcasts.feed.autoStore.buttonOn") : t("podcasts.feed.autoStore.buttonOff"),
      icon: <Inbox className="h-4 w-4" />,
      loading: autoStore.isPending,
      title: t("podcasts.feed.autoStore.hint"),
      onSelect: () => autoStore.mutate(!feed.auto_store),
    });
  if (feed?.is_owner)
    actions.push({ key: "archive", label: t("podcasts.feed.saveArchive"), icon: <Archive className="h-4 w-4" />, onSelect: () => setStoringAll(true) });
  // The dialog, not the inline list at the bottom: that one stays hidden when the catalogue
  // has nothing, and on a phone it is a long scroll away. The dialog says so when it is empty.
  if (feed)
    actions.push({ key: "similar", label: t("podcasts.action.similarShows"), icon: <Compass className="h-4 w-4" />, onSelect: () => setSimilarOpen(true) });
  if (feed?.visibility === "family")
    actions.push({ key: "audience", label: t("podcasts.feed.whoCanHear"), icon: <UserCheck className="h-4 w-4" />, phoneOnly: true, onSelect: () => setAudienceOpen(true) });
  if (episodes.some((e) => !e.completed))
    actions.push({
      key: "played",
      label: t("podcasts.feed.markAllPlayed"),
      icon: <CircleCheck className="h-4 w-4" />,
      loading: setPlayed.isPending,
      onSelect: () => setPlayed.mutate({ ids: episodes.filter((e) => !e.completed).map((e) => e.id), completed: true }),
    });

  if (isLoading || !feed) return <div className="space-y-4 p-6"><Skeleton className="h-24 w-24" /><Skeleton className="h-6 w-56" /><Skeleton className="h-40" /></div>;

  return (
    <div className="pb-8">
      {/* items-start, or the flex row stretches the cover and kills its aspect ratio. On a phone
          the cover sits centred above the title, so the rest gets the full width. */}
      <div className="flex flex-col items-center gap-4 px-4 pb-6 pt-2 text-center sm:flex-row sm:items-start sm:p-6 sm:text-left">
        <Cover kind="podcast" src={feed.image_url} alt={feed.title} auth={false} className="w-44 shrink-0 shadow-card sm:w-28" />
        <div className="w-full min-w-0 flex-1">
          <h1 className="text-2xl font-semibold leading-tight tracking-tight sm:text-xl">{feed.title}</h1>
          {feed.author && <p className="mt-0.5 text-sm text-muted">{feed.author}</p>}
          <div className="mt-2 flex flex-wrap items-center justify-center gap-2 sm:justify-start">
            {feed.source_type === "youtube" && <Pill tone="error">YouTube</Pill>}
            <Pill>{t("podcasts.episodeCount", { count: episodes.length })}</Pill>
            {feed.has_transcripts && (
              <Pill tone="accent" className="gap-1">
                <Languages className="h-3 w-3" aria-hidden />
                {t("common.badge.translatable")}
              </Pill>
            )}
          </div>
          {feed.description && <ClampedText text={stripHtml(feed.description)} lines={3} className="mt-3" />}
          <div className="mt-4 max-w-md text-left">
            <VisibilityField
              value={feed.visibility}
              disabled={!feed.is_owner || changeVisibility.isPending}
              onChange={(v) => changeVisibility.mutate(v)}
            />
          </div>
          {/* On a phone this list would push the episodes a screen down; there it is a sheet
              opened from the ⋯ menu. */}
          {feed.visibility === "family" && (
            <div className="mt-4 max-w-md text-left max-sm:hidden">
              <p className="mb-1.5 text-[13px] font-medium">{t("podcasts.feed.whoCanHear")}</p>
              <AudienceList kind="podcast" itemId={feedId} />
            </div>
          )}
          {/* Four wide buttons stacked one per line on a phone; there, Refresh stays a button and
              the rest are a menu with the same words. */}
          <div className="mt-4 flex flex-wrap gap-2 sm:mt-3">
            <Button size="sm" variant="secondary" className="max-sm:flex-1" icon={<RefreshCw className="h-4 w-4" />} onClick={() => refresh.mutate()} loading={refresh.isPending}>{t("podcasts.action.refresh")}</Button>
            {actions.filter((a) => !a.phoneOnly).map((a) => (
              <Button key={a.key} size="sm" variant="secondary" className="max-sm:hidden" icon={a.icon} loading={a.loading} title={a.title} onClick={a.onSelect}>
                {a.label}
              </Button>
            ))}
            {actions.length > 0 && (
              <span className="sm:hidden">
                <MoreMenuTrigger size="lg">
                  {actions.map((a) => (
                    <MenuItem key={a.key} icon={a.icon} disabled={a.loading} onSelect={a.onSelect}>{a.label}</MenuItem>
                  ))}
                </MoreMenuTrigger>
              </span>
            )}
          </div>
        </div>
      </div>

      {similarOpen && <SimilarShowsDialog feedId={feedId} feedTitle={feed.title} onClose={() => setSimilarOpen(false)} />}

      <Dialog open={audienceOpen} onOpenChange={setAudienceOpen}>
        {audienceOpen && (
          <DialogContent title={t("podcasts.feed.whoCanHear")} variant="sheet">
            <div className="px-6 pb-6">
              <AudienceList kind="podcast" itemId={feedId} />
            </div>
          </DialogContent>
        )}
      </Dialog>

      {storingAll && <StoreAllDialog feedId={feedId} title={feed.title} onClose={() => { setStoringAll(false); qc.invalidateQueries({ queryKey: ["episodes", feedId] }); }} />}

      <div className="px-3 sm:px-6">
        <SearchField
          placeholder={t("podcasts.feed.searchEpisodes")}
          value={search}
          onChange={(e) => setSearch(e.target.value)}
          className="mb-3 w-full max-w-md"
          aria-label={t("podcasts.feed.searchEpisodes")}
        />
        {episodesLoading && <div className="space-y-2">{[...Array(5)].map((_, i) => <Skeleton key={i} className="h-14" />)}</div>}
        {!episodesLoading && episodes.length === 0 && <EmptyState title={t("podcasts.feed.empty.title")} description={t("podcasts.feed.empty.body")} />}

        <ul className="space-y-1.5">
          {episodes.map((ep) => {
            const isOpen = expanded === ep.id;
            const isCurrent = current?.kind === "podcast" && current.epId === ep.id;
            const fetching = (fetchToServer.isPending && fetchToServer.variables?.id === ep.id) || busy === ep.id;
            const pct = ep.progress_secs != null && ep.duration_secs ? Math.min(100, (ep.progress_secs / ep.duration_secs) * 100) : 0;
            return (
              <li key={ep.id} className={cn("rounded-card border transition-colors", isCurrent ? "border-accent/60 bg-accent/5" : "border-border")}>
                <div className="flex items-start gap-2 p-3 sm:gap-3">
                  <div className="min-w-0 flex-1 cursor-pointer" onClick={() => setExpanded(isOpen ? null : ep.id)}>
                    <p className={cn("text-sm font-medium leading-snug", isCurrent && "text-accent")}>{ep.title}</p>
                    <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-xs text-muted">
                      {ep.episode_number != null && <span>{t("podcasts.feed.episode.number", { number: ep.episode_number })}</span>}
                      {ep.has_transcript && (
                        <span title={t("common.badge.translatable")} className="flex items-center text-accent-text">
                          <Languages className="h-3.5 w-3.5" aria-hidden />
                          <span className="sr-only">{t("common.badge.translatable")}</span>
                        </span>
                      )}
                      {ep.published_at && <span>{formatDate(ep.published_at)}</span>}
                      {ep.duration_secs != null && <span>{formatDuration(ep.duration_secs)}</span>}
                      {ep.completed && <span className="text-success">{t("podcasts.feed.episode.played")}</span>}
                      {fetching ? (
                        <span className="flex items-center gap-1 text-accent">
                          <Loader2 className="h-3 w-3 animate-spin" /> {t("podcasts.feed.episode.fetching")}
                        </span>
                      ) : (
                        !ep.has_local && ep.audio_url && <span>{t("podcasts.feed.episode.notOnServer")}</span>
                      )}
                    </div>
                    {pct > 0 && !ep.completed && (
                      <div className="mt-1.5 h-0.5 w-40 rounded-pill bg-border"><div className="h-full rounded-pill bg-accent" style={{ width: `${pct}%` }} /></div>
                    )}
                  </div>
                  {ep.audio_url && (
                    <IconButton
                      size="sm"
                      label={isCurrent && playing ? t("common.action.pause") : ep.has_local ? t("common.action.play") : t("podcasts.feed.episode.fetchAndPlay")}
                      onClick={() => void play(ep)}
                      disabled={fetching}
                      tone="accent"
                    >
                      {fetching ? (
                        <Loader2 className="h-4 w-4 animate-spin" />
                      ) : isCurrent && playing ? (
                        <Pause className="h-4 w-4 fill-current" />
                      ) : ep.has_local ? (
                        <Play className="h-4 w-4 translate-x-px fill-current" />
                      ) : (
                        <Download className="h-4 w-4" />
                      )}
                    </IconButton>
                  )}
                  <MoreMenuTrigger>
                    <MenuItem
                      icon={ep.completed ? <Undo2 /> : <Check />}
                      onSelect={() => setPlayed.mutate({ ids: [ep.id], completed: !ep.completed })}
                    >
                      {ep.completed ? t("podcasts.feed.episode.markUnplayed") : t("podcasts.feed.episode.markPlayed")}
                    </MenuItem>
                    {ep.audio_url && !ep.has_local && (
                      <MenuItem
                        icon={<CloudDownload />}
                        disabled={fetching}
                        onSelect={() => fetchToServer.mutate(ep)}
                      >
                        {t("podcasts.feed.episode.downloadToLibrary")}
                      </MenuItem>
                    )}
                    {ep.has_local && (
                      <MenuItem icon={<Save />} onSelect={() => void saveCopy(ep)}>
                        {t("podcasts.feed.episode.saveCopy")}
                      </MenuItem>
                    )}
                    {/* Also here, not only under the notes: a long description pushed the button a
                        few screens down on a phone. */}
                    {ep.has_transcript && (
                      <MenuItem icon={<Languages />} onSelect={() => setTranslating(ep.id)}>
                        {t("podcasts.translate.title")}
                      </MenuItem>
                    )}
                  </MoreMenuTrigger>
                  {/* On a phone the title itself opens the notes; the chevron's width goes to the title. */}
                  <IconButton size="sm" className="max-sm:hidden" label={isOpen ? t("podcasts.feed.episode.collapse") : t("podcasts.feed.episode.expand")} onClick={() => setExpanded(isOpen ? null : ep.id)}>
                    {isOpen ? <ChevronUp className="h-4 w-4" /> : <ChevronDown className="h-4 w-4" />}
                  </IconButton>
                </div>
                {isOpen && (
                  <div className="border-t border-border px-3 py-3">
                    {ep.description && <p className="whitespace-pre-line text-sm leading-relaxed text-muted">{stripHtml(ep.description)}</p>}
                    {ep.has_transcript && (
                      <Translations
                        episodeId={ep.id}
                        episodeTitle={ep.title}
                        showTitle={feed.title}
                        episodeDurationSecs={ep.duration_secs ?? null}
                        onTranslate={() => setTranslating(ep.id)}
                      />
                    )}
                  </div>
                )}
              </li>
            );
          })}
        </ul>
      </div>

      <div className="px-3 sm:px-6">
        <SimilarShows feedId={feedId} feedTitle={feed.title} />
      </div>

      {translating && <TranslateEpisodeModal episodeId={translating} sourceLanguage={feed.language} onClose={() => setTranslating(null)} />}
    </div>
  );
}
