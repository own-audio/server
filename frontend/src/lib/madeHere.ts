// SPDX-License-Identifier: AGPL-3.0-or-later
import type { AudioBook, GenerationJobStatus } from "../api/types";
import { isMachineTranslatedBook } from "./machineTranslation";

/* What Create ▸ Made here shows of a narration job — the Mac's „Vytvořeno“ (audio2-mac
   MadeHereView). Kept apart from the page so it can be tested without a server. */

export type JobState = "running" | "ready" | "failed";

export function jobState(job: GenerationJobStatus): JobState {
  if (job.stage === "failed") return "failed";
  if (job.stage === "complete") return "ready";
  return "running";
}

/** Blocks done of all: the job's own totals (since 2026-11-22), else its chapters'. */
export function jobBlocks(job: GenerationJobStatus): { done: number; total: number } {
  if (typeof job.blocks_total === "number" && job.blocks_total > 0) {
    return { done: Math.min(job.blocks_done ?? 0, job.blocks_total), total: job.blocks_total };
  }
  const chapters = job.chapters ?? [];
  return {
    done: chapters.reduce((n, c) => n + c.blocks_done, 0),
    total: chapters.reduce((n, c) => n + c.blocks_total, 0),
  };
}

/** A translated book: its book says so, or the job read it in another language than it was in. */
export function isTranslationJob(job: GenerationJobStatus, book?: AudioBook | null): boolean {
  if (isMachineTranslatedBook(book)) return true;
  return !!job.source_language && !!job.target_language && job.source_language.toLowerCase() !== job.target_language.toLowerCase();
}

/** Newest first, each job once: the server's list already is; the jobs remembered in this
 *  browser (an older server without the list) are sorted by when they were made. */
export function sortJobs(jobs: GenerationJobStatus[]): GenerationJobStatus[] {
  const seen = new Set<string>();
  const unique = jobs.filter((j) => (seen.has(j.id) ? false : (seen.add(j.id), true)));
  return unique
    .map((job, i) => ({ job, i }))
    .sort((a, b) => {
      const at = a.job.created_at ? Date.parse(a.job.created_at) : NaN;
      const bt = b.job.created_at ? Date.parse(b.job.created_at) : NaN;
      if (Number.isNaN(at) || Number.isNaN(bt)) return a.i - b.i;
      return bt - at;
    })
    .map(({ job }) => job);
}

export const hasRunningJob = (jobs: GenerationJobStatus[]) => jobs.some((j) => jobState(j) === "running");
