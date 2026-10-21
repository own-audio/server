// SPDX-License-Identifier: AGPL-3.0-or-later
/**
 * Chapters inside one music track — a whole concert as one file, a chapter where each song
 * starts (docs/concerts-plan.md). Pure, so next/previous can be tested without audio.
 */
export interface TrackChapter {
  title: string;
  startSecs: number;
}

/** The chapter playing at `time`: the last one started. A seek lands a hair early, hence the slack. */
export function chapterIndex(chapters: TrackChapter[], time: number): number | null {
  for (let i = chapters.length - 1; i >= 0; i--) {
    if (chapters[i].startSecs <= time + 0.05) return i;
  }
  return null;
}

/** The next chapter's start; null in the last one, where next goes to the next track. */
export function nextChapterStart(chapters: TrackChapter[], time: number): number | null {
  const next = (chapterIndex(chapters, time) ?? -1) + 1;
  return next < chapters.length ? chapters[next].startSecs : null;
}

/** As previous does for tracks: a few seconds in it restarts the chapter, earlier it goes to the
 *  one before. Null in the first chapter, where the track's own rule applies. */
export function previousChapterStart(chapters: TrackChapter[], time: number, restartSecs = 3): number | null {
  const current = chapterIndex(chapters, time);
  if (current == null || current === 0) return null;
  return time - chapters[current].startSecs > restartSecs ? chapters[current].startSecs : chapters[current - 1].startSecs;
}

/** `9:26`, `1:02:03`. */
export function formatChapterTime(secs: number): string {
  const total = Math.max(0, Math.floor(secs));
  const h = Math.floor(total / 3600), m = Math.floor(total / 60) % 60, s = total % 60;
  const two = (n: number) => String(n).padStart(2, "0");
  return h > 0 ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
}

/** `9:26`, `1:02:03` or plain seconds; null for anything else. */
export function parseChapterTime(text: string): number | null {
  const parts = text.trim().split(":");
  if (parts.length < 1 || parts.length > 3 || parts.some((p) => !/^\d+(\.\d+)?$/.test(p))) return null;
  let secs = 0;
  for (let i = 0; i < parts.length; i++) {
    const v = Number(parts[i]);
    if (i > 0 && v >= 60) return null;
    secs = secs * 60 + v;
  }
  return secs;
}
