// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Link } from "react-router-dom";
import { useInfiniteQuery, useQuery } from "@tanstack/react-query";
import { Pause, Play, Sparkles } from "lucide-react";
import SectionTheme from "../../components/shell/SectionTheme";
import { Page } from "../../components/shell/SplitView";
import { AiVoiceBadge, Button, Cover, EmptyState, MachineTranslationBadge, Skeleton } from "../../components/ui";
import { getGenerationStatus, listGenerationJobs, startedJobIds } from "../../api/generation";
import { listBooks } from "../../api/audiobooks";
import { listRecentTranslations } from "../../api/podcastTranslate";
import type { AudioBook, GenerationJobStatus, GenerationStage, RecentPodcastTranslation } from "../../api/types";
import { formatAmount } from "../../api/billing";
import { chargedMicro } from "../../lib/price";
import { useServerFeatures } from "../../lib/features";
import { useMyPermissions } from "../../lib/permissions";
import { episodeMachineTranslationLine, isMachineTranslatedEpisode, languageName } from "../../lib/machineTranslation";
import { hasRunningJob, isTranslationJob, jobBlocks, jobState, sortJobs } from "../../lib/madeHere";
import { playBook } from "../../lib/play";
import { playTranslation } from "../../lib/playTranslation";
import { usePlayerStore } from "../../store/playerStore";
import { useT, type PlainKey } from "../../i18n";
import JobCoverPanel from "./JobCoverPanel";

/**
 * Create ▸ Made here: everything this account had narrated or translated — its narration jobs
 * from any device (`GET /audiobook-gen/jobs`), and the family's translated episodes
 * (`/podcast-translate/recent`, failed ones left out by the server). The Mac's „Vytvořeno“.
 * A server without the job list answers 404; then the jobs started in this browser are shown.
 */
const STAGE: Partial<Record<GenerationStage, PlainKey>> = {
  extracting: "generate.stage.extracting",
  translating: "generate.stage.translating",
  preprocessing: "generate.stage.preprocessing",
  narrating: "generate.stage.narrating",
  assembling: "generate.stage.assembling",
};

const EPISODE_STATUS: Record<RecentPodcastTranslation["status"], PlainKey> = {
  translating: "podcasts.translation.status.translating",
  narrating: "podcasts.translation.status.narrating",
  assembling: "podcasts.translation.status.assembling",
  complete: "podcasts.translation.status.complete",
  failed: "podcasts.translation.status.failed",
};

async function rememberedJobs(): Promise<GenerationJobStatus[]> {
  const jobs = await Promise.all(startedJobIds().map((id) => getGenerationStatus(id).catch(() => null)));
  return jobs.filter((j): j is GenerationJobStatus => !!j);
}

export function JobRow({ job, book }: { job: GenerationJobStatus; book?: AudioBook }) {
  const { t } = useT();
  const { canGenerate } = useMyPermissions();
  const [coverOpen, setCoverOpen] = useState(false);
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const state = jobState(job);
  const blocks = jobBlocks(job);
  const charged = chargedMicro(job);
  const translated = isTranslationJob(job, book);
  const isCurrent = !!book && current?.kind === "audiobook" && current.bookId === book.id;
  const stage = STAGE[job.stage];
  const target = job.target_language ?? book?.language ?? null;
  const source = job.source_language ?? book?.origin?.source_language ?? null;
  const languages = translated && source && target
    ? t("generate.madeHere.languages", { source: languageName(source), target: languageName(target) })
    : target
      ? languageName(target)
      : null;
  const canChangeCover = state === "ready" && !!job.book_id && canGenerate && job.cover_regenerations_left !== undefined;

  return (
    <li className="rounded-xl border border-border bg-card p-4">
      <div className="flex gap-3">
        <Cover kind="audiobook" src={book?.cover_url ?? null} alt="" className="w-14 shrink-0" />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Link to={`/generate/${job.id}`} className="min-w-0 truncate font-medium hover:underline">{job.title}</Link>
            {translated ? <MachineTranslationBadge /> : <AiVoiceBadge />}
          </div>
          {languages && <p className="text-xs text-muted">{languages}</p>}

          {state === "running" && (
            <div className="mt-1.5">
              <p className="text-xs text-muted">
                {stage ? t(stage) : t("generate.status.narrating")}
                {blocks.total > 0 && ` · ${t("generate.status.blocks", { done: blocks.done, total: blocks.total })}`}
              </p>
              {blocks.total > 0 && (
                <div
                  className="mt-1 h-1.5 max-w-xs rounded-full bg-bg-alt"
                  role="progressbar"
                  aria-label={t("generate.status.progressTitle")}
                  aria-valuemin={0}
                  aria-valuemax={blocks.total}
                  aria-valuenow={blocks.done}
                >
                  <div className="h-1.5 rounded-full bg-accent transition-all" style={{ width: `${(blocks.done / blocks.total) * 100}%` }} />
                </div>
              )}
            </div>
          )}

          {state === "ready" && (
            <p className="mt-1 text-xs text-success-text">
              {t("generate.madeHere.ready")}
              {charged != null && <span className="text-muted"> · {t("generate.status.actualCost")}: {formatAmount(charged, job.currency)}</span>}
            </p>
          )}

          {state === "failed" && (
            <div className="mt-1 text-xs">
              <p className="font-medium text-error">{t("generate.madeHere.failed")} — {job.error ?? t("generate.status.failedFallback")}</p>
              <p className="text-muted">
                {charged
                  ? t("generate.status.chargedPartial", { amount: formatAmount(charged, job.currency) })
                  : t("generate.status.notCharged")}
              </p>
            </div>
          )}

          <div className="mt-2 flex flex-wrap items-center gap-2">
            {state === "ready" && book && (
              <Button
                size="sm"
                icon={isCurrent && playing ? <Pause className="h-3.5 w-3.5 fill-current" /> : <Play className="h-3.5 w-3.5 fill-current" />}
                onClick={() => void playBook(book)}
              >
                {isCurrent && playing ? t("common.action.pause") : t("generate.status.listenNow")}
              </Button>
            )}
            {state === "ready" && job.book_id && !book && (
              <Link to={`/audiobooks/${job.book_id}`} className="text-sm font-medium text-accent-text hover:underline">{t("generate.status.listenNow")}</Link>
            )}
            {canChangeCover && (
              <Button size="sm" variant="secondary" aria-expanded={coverOpen} onClick={() => setCoverOpen((v) => !v)}>
                {coverOpen ? t("generate.madeHere.hideCover") : t("generate.madeHere.changeCover")}
              </Button>
            )}
            <Link to={`/generate/${job.id}`} className="text-sm text-muted hover:text-accent-text hover:underline">{t("generate.madeHere.details")}</Link>
          </div>
        </div>
      </div>
      {canChangeCover && coverOpen && job.book_id && <JobCoverPanel job={{ ...job, book_id: job.book_id }} />}
    </li>
  );
}

export function EpisodeRow({ tr }: { tr: RecentPodcastTranslation }) {
  const { t } = useT();
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const isCurrent = current?.kind === "podcast" && current.translationId === tr.id;
  const line = tr.status === "complete" ? episodeMachineTranslationLine(tr) : null;
  const target = languageName(tr.target_language);

  return (
    <li className="rounded-xl border border-border bg-card p-4">
      <div className="flex gap-3">
        <Cover kind="podcast" src={tr.image_url} alt="" aspect="square" className="h-14 w-14 shrink-0 rounded-lg" />
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <Link to={`/podcasts/${tr.feed_id}`} className="min-w-0 truncate font-medium hover:underline">{tr.episode_title}</Link>
            {isMachineTranslatedEpisode(tr) && <MachineTranslationBadge />}
          </div>
          <p className="truncate text-xs text-muted">
            {tr.feed_title} · {t("generate.madeHere.languages", { source: languageName(tr.source_language), target })}
          </p>
          <p className={tr.status === "complete" ? "mt-1 text-xs text-success-text" : "mt-1 text-xs text-muted"}>{t(EPISODE_STATUS[tr.status])}</p>
          {line && <p className="mt-1 text-xs text-muted">{line}</p>}
          {tr.status === "complete" && <p className="mt-1 text-xs text-muted">{tr.notice || t("generate.personalUse.short")}</p>}
          {tr.status === "complete" && tr.stream_url && (
            <Button
              size="sm"
              className="mt-2"
              icon={isCurrent && playing ? <Pause className="h-3.5 w-3.5 fill-current" /> : <Play className="h-3.5 w-3.5 fill-current" />}
              onClick={() =>
                playTranslation(tr, { id: tr.episode_id, title: tr.episode_title, durationSecs: tr.duration_secs ?? null, imageUrl: tr.image_url, feedId: tr.feed_id }, tr.feed_title)
              }
            >
              {isCurrent && playing ? t("common.action.pause") : t("podcasts.translatePage.playLanguage", { language: target })}
            </Button>
          )}
        </div>
      </div>
    </li>
  );
}

export default function MadeHerePage() {
  const { t } = useT();
  const { features } = useServerFeatures();
  const { canGenerate } = useMyPermissions();
  const showBooks = features.narration && canGenerate;

  const list = useInfiniteQuery({
    queryKey: ["generation-jobs"],
    queryFn: ({ pageParam }) => listGenerationJobs(pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last?.next ?? undefined,
    enabled: showBooks,
    refetchInterval: (q) => (hasRunningJob((q.state.data?.pages ?? []).flatMap((p) => p?.items ?? [])) ? 5000 : false),
  });
  const noList = list.isSuccess && list.data.pages[0] === null;
  const remembered = useQuery({
    queryKey: ["generation-jobs", "remembered"],
    queryFn: rememberedJobs,
    enabled: showBooks && noList,
    refetchInterval: (q) => (hasRunningJob(q.state.data ?? []) ? 5000 : false),
  });
  const jobs = sortJobs(noList ? (remembered.data ?? []) : (list.data?.pages ?? []).flatMap((p) => p?.items ?? []));

  const { data: books = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks, enabled: showBooks && jobs.length > 0 });
  const bookById = new Map(books.map((b) => [b.id, b]));

  const episodes = useQuery({
    queryKey: ["podcast-translations", "recent"],
    queryFn: listRecentTranslations,
    enabled: features.translation,
    refetchInterval: (q) => ((q.state.data ?? []).some((tr) => tr.status !== "complete" && tr.status !== "failed") ? 5000 : false),
  });
  const recent = (episodes.data ?? []).filter((tr) => tr.status !== "failed");

  const loading = (showBooks && (list.isLoading || (noList && remembered.isLoading))) || (features.translation && episodes.isLoading);
  const failed = (showBooks && list.isError) || (features.translation && episodes.isError);

  return (
    <SectionTheme cloud="book">
      <Page title={t("generate.madeHere.title")} width="max-w-3xl">
        {loading ? (
          <Skeleton className="h-48" />
        ) : (
          <>
            {failed && <p role="alert" className="mb-4 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{t("generate.madeHere.error")}</p>}

            {jobs.length > 0 && (
              <section className="mb-8" aria-labelledby="made-here-books">
                <h2 id="made-here-books" className="mb-2 text-sm font-semibold uppercase tracking-wide text-muted">{t("generate.madeHere.books")}</h2>
                <ul className="space-y-2">
                  {jobs.map((job) => (
                    <JobRow key={job.id} job={job} book={job.book_id ? bookById.get(job.book_id) : undefined} />
                  ))}
                </ul>
                {list.hasNextPage && !noList && (
                  <Button size="sm" variant="secondary" className="mt-3" loading={list.isFetchingNextPage} onClick={() => void list.fetchNextPage()}>
                    {t("generate.madeHere.older")}
                  </Button>
                )}
              </section>
            )}

            {recent.length > 0 && (
              <section aria-labelledby="made-here-episodes">
                <h2 id="made-here-episodes" className="mb-2 text-sm font-semibold uppercase tracking-wide text-muted">{t("generate.madeHere.episodes")}</h2>
                <ul className="space-y-2">
                  {recent.map((tr) => <EpisodeRow key={tr.id} tr={tr} />)}
                </ul>
              </section>
            )}

            {jobs.length === 0 && recent.length === 0 && !failed && (
              <EmptyState icon={<Sparkles />} title={t("generate.madeHere.title")} description={t("generate.madeHere.empty")} />
            )}
          </>
        )}
      </Page>
    </SectionTheme>
  );
}
