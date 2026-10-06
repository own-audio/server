// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Link, useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Languages, Loader2, Pause, Play } from "lucide-react";
import { listEpisodes, listFeeds } from "../../api/podcasts";
import { Page } from "../../components/shell/SplitView";
import SectionTheme from "../../components/shell/SectionTheme";
import { MediaCard, MediaGrid, MediaRow } from "../../components/library/MediaCard";
import { EmptyState, Skeleton } from "../../components/ui";
import { formatDate, formatDuration } from "../../lib/format";
import type { PodcastEpisodeTranslation, PodcastFeed, PodcastTranslationStatus } from "../../api/types";
import { listFeedTranslations, listRecentTranslations } from "../../api/podcastTranslate";
import { playTranslation } from "../../lib/playTranslation";
import { usePlayerStore } from "../../store/playerStore";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";
import TranslateEpisodeModal from "./TranslateEpisodeModal";

/**
 * Translate and narrate a podcast episode, as its own place under Create.
 *
 * The same feature lives inside each episode's menu, where nobody who doesn't
 * already know about it looks. Here it starts from what can be translated:
 * only shows that publish transcripts, then only their episodes that have one.
 */
const STATUS: Record<PodcastTranslationStatus, PlainKey> = {
  translating: "podcasts.translation.status.translating",
  narrating: "podcasts.translation.status.narrating",
  assembling: "podcasts.translation.status.assembling",
  complete: "podcasts.translation.status.complete",
  failed: "podcasts.translation.status.failed",
};

/** A finished translation's language, and the button that plays it. */
function LanguageChip({ tr, active, playing, onPlay }: { tr: PodcastEpisodeTranslation; active: boolean; playing: boolean; onPlay: () => void }) {
  const { t } = useT();
  if (tr.status !== "complete") {
    return (
      <span className="inline-flex h-8 shrink-0 items-center gap-1 rounded-pill bg-bg-alt px-2.5 text-xs text-muted pointer-coarse:h-9" title={t(STATUS[tr.status])}>
        <Loader2 className="h-3 w-3 animate-spin" aria-hidden />
        {tr.target_language.toUpperCase()}
      </span>
    );
  }
  return (
    <button
      type="button"
      onClick={onPlay}
      aria-label={t("podcasts.translatePage.playLanguage", { language: tr.target_language.toUpperCase() })}
      className={cn(
        "inline-flex h-8 shrink-0 items-center gap-1 rounded-pill px-2.5 text-xs font-semibold transition-colors pointer-coarse:h-9",
        active ? "bg-accent text-on-accent" : "bg-accent/12 text-accent-text hover:bg-accent/20"
      )}
    >
      {active && playing ? <Pause className="h-3 w-3 fill-current" /> : <Play className="h-3 w-3 fill-current" />}
      {tr.target_language.toUpperCase()}
    </button>
  );
}

export default function TranslatePage() {
  const { t } = useT();
  const navigate = useNavigate();
  const current = usePlayerStore((st) => st.track);
  const playing = usePlayerStore((st) => st.playing);
  const isPlaying = (translationId: string) => current?.kind === "podcast" && current.translationId === translationId;
  const [show, setShow] = useState<PodcastFeed | null>(null);
  const [translating, setTranslating] = useState<string | null>(null);

  const { data: feeds = [], isLoading } = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });
  const shows = feeds.filter((f) => f.has_transcripts);

  const { data: episodes = [], isLoading: episodesLoading } = useQuery({
    queryKey: ["episodes", show?.id ?? "", ""],
    queryFn: () => listEpisodes(show!.id, 200),
    enabled: !!show,
  });
  const translatable = episodes.filter((e) => e.has_transcript);

  const { data: recent = [] } = useQuery({
    queryKey: ["podcast-translations", "recent"],
    queryFn: listRecentTranslations,
    // A translation takes minutes; the list follows it while any is still being made.
    refetchInterval: (q) => ((q.state.data ?? []).some((tr) => tr.status !== "complete") ? 5000 : false),
  });
  const { data: feedTranslations = [] } = useQuery({
    queryKey: ["podcast-translations", "feed", show?.id ?? ""],
    queryFn: () => listFeedTranslations(show!.id),
    enabled: !!show,
    refetchInterval: (q) => ((q.state.data ?? []).some((tr) => tr.status !== "complete" && tr.status !== "failed") ? 5000 : false),
  });
  const byEpisode = new Map<string, PodcastEpisodeTranslation[]>();
  for (const tr of feedTranslations) {
    if (tr.status === "failed") continue;
    byEpisode.set(tr.episode_id, [...(byEpisode.get(tr.episode_id) ?? []), tr]);
  }

  return (
    <SectionTheme cloud="podcast">
      {/* Back steps out of a show first, then to Podcasts — it is reached from the sidebar, which a phone hides. */}
      <Page title={t("podcasts.translatePage.title")} onBack={() => (show ? setShow(null) : navigate("/podcasts"))} width="max-w-3xl">
        <p className="-mt-4 mb-6 flex gap-2 text-sm text-muted">
          <Languages className="mt-0.5 h-4 w-4 shrink-0 text-accent-text" aria-hidden />
          {t("podcasts.translatePage.intro")}
        </p>

        {!show ? (
          <>
            {recent.length > 0 && (
              <section className="mb-8">
                <h2 className="mb-1 text-sm font-semibold uppercase tracking-wide text-muted">{t("podcasts.translatePage.yours")}</h2>
                <div className="-mx-2">
                  {recent.slice(0, 10).map((tr) => (
                    <MediaRow
                      key={tr.id}
                      kind="podcast"
                      title={tr.episode_title}
                      subtitle={tr.status === "complete" ? tr.feed_title : `${tr.feed_title} · ${t(STATUS[tr.status])}`}
                      cover={tr.image_url}
                      coverAuth={false}
                      active={isPlaying(tr.id)}
                      onClick={() => navigate(`/podcasts/${tr.feed_id}`)}
                      action={
                        <LanguageChip
                          tr={tr}
                          active={isPlaying(tr.id)}
                          playing={playing}
                          onPlay={() => playTranslation(tr, { id: tr.episode_id, title: tr.episode_title, durationSecs: tr.duration_secs ?? null, imageUrl: tr.image_url, feedId: tr.feed_id }, tr.feed_title)}
                        />
                      }
                    />
                  ))}
                </div>
              </section>
            )}
            <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-muted">{t("podcasts.translatePage.pickShow")}</h2>
            {isLoading ? (
              <Skeleton className="h-48" />
            ) : shows.length === 0 ? (
              <EmptyState icon={<Languages />} title={t("podcasts.translatePage.noShowsTitle")} description={t("podcasts.translatePage.noShowsBody")} />
            ) : (
              <MediaGrid className="-mx-3">
                {shows.map((f) => (
                  <MediaCard key={f.id} kind="podcast" title={f.title} subtitle={f.author} cover={f.image_url} coverAuth={false} translatable onClick={() => setShow(f)} />
                ))}
              </MediaGrid>
            )}
          </>
        ) : (
          <>
            <div className="mb-3 flex flex-wrap items-center gap-2">
              <Link to={`/podcasts/${show.id}`} className="ml-auto text-sm font-medium text-accent-text hover:underline">
                {t("podcasts.translatePage.openShow")}
              </Link>
            </div>
            <div className="mb-4 -mx-2">
              <MediaRow kind="podcast" title={show.title} subtitle={show.author} cover={show.image_url} coverAuth={false} />
            </div>
            <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-muted">{t("podcasts.translatePage.pickEpisode")}</h2>
            {episodesLoading ? (
              <Skeleton className="h-48" />
            ) : translatable.length === 0 ? (
              <EmptyState title={t("podcasts.translatePage.noEpisodes")} />
            ) : (
              <div className="-mx-2">
                {translatable.map((ep) => (
                  <MediaRow
                    key={ep.id}
                    kind="podcast"
                    title={ep.title}
                    subtitle={[ep.published_at && formatDate(ep.published_at), ep.duration_secs != null && formatDuration(ep.duration_secs)].filter(Boolean).join(" · ") || null}
                    cover={ep.image_url ?? show.image_url}
                    coverAuth={false}
                    translatable
                    onClick={() => setTranslating(ep.id)}
                    action={
                      <span className="flex shrink-0 items-center gap-1">
                        {(byEpisode.get(ep.id) ?? []).map((tr) => (
                          <LanguageChip
                            key={tr.id}
                            tr={tr}
                            active={isPlaying(tr.id)}
                            playing={playing}
                            onPlay={() => playTranslation(tr, { id: ep.id, title: ep.title, durationSecs: tr.duration_secs ?? ep.duration_secs, imageUrl: ep.image_url ?? show.image_url, feedId: show.id }, show.title)}
                          />
                        ))}
                      </span>
                    }
                  />
                ))}
              </div>
            )}
          </>
        )}

        {translating && show && (
          <TranslateEpisodeModal episodeId={translating} sourceLanguage={show.language} onClose={() => setTranslating(null)} />
        )}
      </Page>
    </SectionTheme>
  );
}
