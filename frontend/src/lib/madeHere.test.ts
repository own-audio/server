// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import type { AudioBook, GenerationJobStatus } from "../api/types";
import { hasRunningJob, isTranslationJob, jobBlocks, jobState, sortJobs } from "./madeHere";

const job = (extra: Partial<GenerationJobStatus> = {}): GenerationJobStatus => ({
  id: "j1",
  title: "Book",
  stages: ["extracting", "narrating", "assembling"],
  stage: "narrating",
  error: null,
  chapters: [],
  download_url: null,
  book_id: null,
  quoted_price_cents: 1000,
  actual_cost_cents: null,
  charged_price_cents: null,
  currency: "USD",
  ...extra,
});

describe("Made here", () => {
  it("tells running, ready and failed apart", () => {
    expect(jobState(job())).toBe("running");
    expect(jobState(job({ stage: "complete" }))).toBe("ready");
    expect(jobState(job({ stage: "failed" }))).toBe("failed");
    expect(hasRunningJob([job({ stage: "complete" }), job()])).toBe(true);
    expect(hasRunningJob([job({ stage: "failed" })])).toBe(false);
  });

  it("counts blocks from the job, else from its chapters", () => {
    expect(jobBlocks(job({ blocks_done: 30, blocks_total: 120 }))).toEqual({ done: 30, total: 120 });
    const chapters = [
      { id: "a", title: "1", status: "complete", blocks_done: 5, blocks_total: 5 },
      { id: "b", title: "2", status: "in_progress", blocks_done: 2, blocks_total: 10 },
    ] as GenerationJobStatus["chapters"];
    expect(jobBlocks(job({ chapters }))).toEqual({ done: 7, total: 15 });
  });

  it("knows a translation by its languages or its book", () => {
    expect(isTranslationJob(job({ source_language: "cs", target_language: "en" }))).toBe(true);
    expect(isTranslationJob(job({ source_language: "cs", target_language: "cs" }))).toBe(false);
    expect(isTranslationJob(job())).toBe(false);
    const book = { origin: { kind: "machine_translated" } } as AudioBook;
    expect(isTranslationJob(job(), book)).toBe(true);
  });

  it("lists newest first, each job once", () => {
    const sorted = sortJobs([
      job({ id: "old", created_at: "2026-11-01T10:00:00Z" }),
      job({ id: "new", created_at: "2026-11-22T10:00:00Z" }),
      job({ id: "new", created_at: "2026-11-22T10:00:00Z" }),
    ]);
    expect(sorted.map((j) => j.id)).toEqual(["new", "old"]);
    // An older server's jobs have no dates: kept in the order they came.
    expect(sortJobs([job({ id: "a" }), job({ id: "b" })]).map((j) => j.id)).toEqual(["a", "b"]);
  });
});
