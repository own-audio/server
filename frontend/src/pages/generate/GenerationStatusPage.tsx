// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from "react";
import SectionTheme from "../../components/shell/SectionTheme";
import { Page } from "../../components/shell/SplitView";
import { useParams, Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { getGenerationStatus } from "../../api/generation";
import type { GenerationStage } from "../../api/types";
import { formatCents, formatDuration } from "../../lib/format";
import { useT, type PlainKey } from "../../i18n";

/** One (time, blocksDone) sample used to estimate narration throughput. */
interface ProgressSample {
  atMs: number;
  blocksDone: number;
}

/** Keeps a short rolling window of samples and turns it into an ETA string. */
function useNarrationEta(blocksDone: number, blocksTotal: number, active: boolean): string | null {
  const [samples, setSamples] = useState<ProgressSample[]>([]);
  const { t } = useT();

  // Reading the wall clock is inherently impure, so it belongs in an effect
  // (React's own carve-out for "external system" reads) rather than render.
  useEffect(() => {
    if (!active) {
      // eslint-disable-next-line react-hooks/set-state-in-effect
      setSamples([]);
      return;
    }
    const now = Date.now();
    setSamples((prev) =>
      // Keep roughly the last 2 minutes of samples so the rate reflects
      // current throughput, not the average since the page was opened.
      [...prev, { atMs: now, blocksDone }].filter((s) => now - s.atMs <= 120_000)
    );
  }, [active, blocksDone]);

  if (!active || blocksTotal === 0) return null;

  const oldest = samples[0];
  const newest = samples[samples.length - 1];
  if (!oldest || !newest || newest.atMs === oldest.atMs) return t("generate.eta.estimating");

  const blocksPerMs = (newest.blocksDone - oldest.blocksDone) / (newest.atMs - oldest.atMs);
  if (blocksPerMs <= 0) return t("generate.eta.estimating");

  const remainingBlocks = blocksTotal - blocksDone;
  const remainingMs = remainingBlocks / blocksPerMs;
  const remainingMins = Math.round(remainingMs / 60_000);

  if (remainingMins <= 0) return t("generate.eta.finishing");
  if (remainingMins < 60) return t("generate.eta.minutes", { count: remainingMins });
  return t("generate.eta.duration", { duration: formatDuration(remainingMins * 60) });
}

const STAGE_ORDER: { stage: GenerationStage; label: PlainKey }[] = [
  { stage: "extracting", label: "generate.stage.extracting" },
  { stage: "translating", label: "generate.stage.translating" },
  { stage: "preprocessing", label: "generate.stage.preprocessing" },
  { stage: "narrating", label: "generate.stage.narrating" },
  { stage: "assembling", label: "generate.stage.assembling" },
];

function stageRank(stages: GenerationStage[], stage: GenerationStage): number {
  if (stage === "complete") return stages.length;
  if (stage === "failed") return -1;
  return stages.indexOf(stage);
}

export default function GenerationStatusPage() {
  const { jobId } = useParams<{ jobId: string }>();
  const { t } = useT();

  const { data: job } = useQuery({
    queryKey: ["generation-status", jobId],
    queryFn: () => getGenerationStatus(jobId as string),
    enabled: !!jobId,
    refetchInterval: (query) => {
      const stage = query.state.data?.stage;
      return stage === "complete" || stage === "failed" ? false : 1000;
    },
  });

  const blocksDone = job?.chapters.reduce((sum, c) => sum + c.blocks_done, 0) ?? 0;
  const blocksTotal = job?.chapters.reduce((sum, c) => sum + c.blocks_total, 0) ?? 0;
  const eta = useNarrationEta(blocksDone, blocksTotal, job?.stage === "narrating");

  if (!job) {
    return <p className="text-sm text-muted">{t("common.loading")}</p>;
  }

  const currentRank = stageRank(job.stages, job.stage);
  const relevantStages = STAGE_ORDER.filter((s) => job.stages.includes(s.stage));

  const done = job.stage === "complete";

  return (
    <SectionTheme cloud="book">
    <Page title={job.title} back="/generate" width="max-w-2xl">
      <p className="-mt-4 mb-6 text-sm text-muted">
        {done ? t("generate.status.finished") : job.stage === "failed" ? t("generate.status.stoppedEarly") : t("generate.status.narrating")}
      </p>

      <div className="rounded-xl border border-border bg-card p-6 shadow-sm">
        {job.stage === "failed" ? (
          <div className="rounded-card border border-error/40 bg-error/10 p-4 text-sm text-error">
            <p className="font-semibold">{t("generate.status.failedTitle")}</p>
            <p className="mt-1">{job.error ?? t("generate.status.failedFallback")}</p>
          </div>
        ) : (
          <ol className="space-y-3">
            {relevantStages.map(({ stage, label }) => {
              const rank = job.stages.indexOf(stage);
              const done = currentRank > rank || job.stage === "complete";
              const active = job.stage === stage;
              return (
                <li key={stage} className="flex items-center gap-3 text-sm">
                  <span
                    className={`flex h-6 w-6 shrink-0 items-center justify-center rounded-full text-xs ${
 done
 ? "bg-accent text-on-accent"
 : active
 ? "bg-accent/15 text-accent "
 : "bg-bg-alt text-muted "
 }`}
                  >
                    {done ? "✓" : active ? "…" : ""}
                  </span>
                  <span className={active ? "font-semibold" : done ? "" : "text-muted"}>
                    {t(label)}
                  </span>
                </li>
              );
            })}
          </ol>
        )}

        {job.stage === "narrating" && blocksTotal > 0 && (
          <div className="mt-5 border-t border-border pt-4">
            <div className="flex justify-between text-sm">
              <span className="font-semibold">{t("generate.status.progressTitle")}</span>
              <span className="tabular-nums text-muted">
                {t("generate.status.blocks", { done: blocksDone, total: blocksTotal })}
              </span>
            </div>
            <div className="mt-2 h-2 rounded-full bg-bg-alt">
              <div
                className="h-2 rounded-full bg-accent transition-all"
                style={{ width: `${(blocksDone / blocksTotal) * 100}%` }}
              />
            </div>
            {eta && <p className="mt-2 text-xs text-muted">{eta}</p>}
          </div>
        )}

        {job.stage === "narrating" && job.chapters.length > 0 && (
          <div className="mt-5 space-y-2 border-t border-border pt-4">
            {job.chapters.map((chapter) => (
              <div key={chapter.id} className="text-sm">
                <div className="flex justify-between text-xs text-muted">
                  <span>{chapter.title}</span>
                  <span>
                    {chapter.blocks_done}/{chapter.blocks_total}
                  </span>
                </div>
                <div className="mt-1 h-1.5 rounded-full bg-bg-alt">
                  <div
                    className="h-1.5 rounded-full bg-accent transition-all"
                    style={{ width: `${(chapter.blocks_done / chapter.blocks_total) * 100}%` }}
                  />
                </div>
              </div>
            ))}
          </div>
        )}

        {/* Gated on the stage, not on `download_url`. A per-chapter job never
            has one, and hiding the whole block behind it left those jobs with
            no "it worked", no way into the book, and no final price. */}
        {done && (
          <div className="mt-5 border-t border-border pt-4">
            <p className="mb-3 text-sm font-semibold text-success-text">{t("generate.status.ready")}</p>
            <div className="flex flex-wrap gap-2">
              {job.book_id && (
                <Link
                  to={`/audiobooks/${job.book_id}`}
                  className="inline-block rounded-lg bg-accent px-5 py-2 text-sm font-medium text-on-accent hover:brightness-110"
                >
                  {t("generate.status.listenNow")}
                </Link>
              )}
              {job.download_url && (
                <a
                  href={job.download_url}
                  className="inline-block rounded-lg border border-border px-5 py-2 text-sm font-medium hover:bg-bg-alt"
                >
                  {t("generate.status.download")}
                </a>
              )}
            </div>
            {!job.download_url && (
              <p className="mt-2 text-xs text-muted">{t("generate.status.perChapterNote")}</p>
            )}

            {job.charged_price_cents != null && (
              <dl className="mt-4 space-y-1 text-xs text-muted">
                <div className="flex justify-between">
                  <dt>{t("generate.status.quoted")}</dt>
                  <dd className="tabular-nums">{formatCents(job.quoted_price_cents, job.currency)}</dd>
                </div>
                <div className="flex justify-between font-medium text-fg">
                  <dt>{t("generate.status.actualCost")}</dt>
                  <dd className="tabular-nums">{formatCents(job.charged_price_cents, job.currency)}</dd>
                </div>
              </dl>
            )}
          </div>
        )}
      </div>

      <Link to="/generate" className="mt-4 inline-block text-sm text-muted hover:text-accent">
        {t("generate.status.another")}
      </Link>
    </Page>
    </SectionTheme>
  );
}
