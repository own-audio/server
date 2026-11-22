// SPDX-License-Identifier: AGPL-3.0-or-later
import type { GenerationJobStatus } from "../api/types";

/** What a finished narration's cover can still become. */
export function coverActions(job: Pick<GenerationJobStatus, "cover_mode" | "cover_regenerations_left" | "source_has_cover">) {
  const left = job.cover_regenerations_left ?? 0;
  return {
    regenerate: left > 0,
    // Only when the server read the file and found one; `null` (not read, older job) offers nothing.
    useBook: job.source_has_cover === true && job.cover_mode !== "book",
    remove: job.cover_mode !== "none",
  };
}
