// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";
import type { AudioBookFile, AudioBookChapter } from "../api/types";

export type SleepTimerOption = null | number | "end-of-chapter";

export interface AudiobookPlayerState {
  // ── Book context ────────────────────────────────
  bookId: string | null;
  files: AudioBookFile[];
  chapters: AudioBookChapter[];

  // ── Configurable playback settings ──────────────
  skipForwardSecs: number;
  skipBackwardSecs: number;
  playbackSpeed: number;

  // ── Sleep timer ─────────────────────────────────
  sleepTimer: SleepTimerOption;
  sleepTimerEndTime: number | null; // Date.now() + ms

  // ── Progress display mode ───────────────────────
  showBookProgress: boolean; // true = whole book, false = current file
  showPercentage: boolean;

  // ── Auto-bookmark ───────────────────────────────
  autoBookmarkEnabled: boolean;
  autoBookmarkIntervalMins: number; // minutes between auto-bookmarks

  // ── Seek bar options ────────────────────────────
  showBookmarkMarkers: boolean; // show bookmark pins on the seek bar
  seekBarMode: "chapters" | "files"; // chapters = per-file chapters, files = whole book by file

  // ── Book switch counter (presentation mode) ─────
  bookSwitchCount: number;
  bookSwitchLog: { fromBookId: string; toBookId: string; timestamp: number }[];

  // ── Actions ─────────────────────────────────────
  setBookContext: (
    bookId: string,
    files: AudioBookFile[],
    chapters: AudioBookChapter[],
    settings: { skipForwardSecs: number; skipBackwardSecs: number; playbackSpeed: number }
  ) => void;
  clearBookContext: () => void;

  setSkipForwardSecs: (v: number) => void;
  setSkipBackwardSecs: (v: number) => void;
  setPlaybackSpeed: (v: number) => void;

  setSleepTimer: (option: SleepTimerOption) => void;
  clearSleepTimer: () => void;

  toggleShowBookProgress: () => void;
  toggleShowPercentage: () => void;

  setAutoBookmarkEnabled: (v: boolean) => void;
  setAutoBookmarkIntervalMins: (v: number) => void;

  setShowBookmarkMarkers: (v: boolean) => void;
  setSeekBarMode: (v: "chapters" | "files") => void;

  recordBookSwitch: (fromBookId: string, toBookId: string) => void;
  resetBookSwitchCounter: () => void;
}

export const useAudiobookPlayerStore = create<AudiobookPlayerState>((set) => ({
  bookId: null,
  files: [],
  chapters: [],

  skipForwardSecs: 30,
  skipBackwardSecs: 15,
  playbackSpeed: 1.0,

  sleepTimer: null,
  sleepTimerEndTime: null,

  showBookProgress: true,
  showPercentage: false,

  autoBookmarkEnabled: false,
  autoBookmarkIntervalMins: 5,

  showBookmarkMarkers: true,
  seekBarMode: "chapters",

  bookSwitchCount: 0,
  bookSwitchLog: [],

  setBookContext: (bookId, files, chapters, settings) =>
    set({
      bookId,
      files,
      chapters,
      skipForwardSecs: settings.skipForwardSecs,
      skipBackwardSecs: settings.skipBackwardSecs,
      playbackSpeed: settings.playbackSpeed,
    }),

  clearBookContext: () =>
    set({
      bookId: null,
      files: [],
      chapters: [],
      sleepTimer: null,
      sleepTimerEndTime: null,
    }),

  setSkipForwardSecs: (v) => set({ skipForwardSecs: Math.max(1, Math.min(120, v)) }),
  setSkipBackwardSecs: (v) => set({ skipBackwardSecs: Math.max(1, Math.min(120, v)) }),
  setPlaybackSpeed: (v) => set({ playbackSpeed: Math.max(0.5, Math.min(3.0, v)) }),

  setSleepTimer: (option) => {
    if (option === null) {
      set({ sleepTimer: null, sleepTimerEndTime: null });
    } else if (option === "end-of-chapter") {
      set({ sleepTimer: "end-of-chapter", sleepTimerEndTime: null });
    } else {
      // option is minutes
      set({
        sleepTimer: option,
        sleepTimerEndTime: Date.now() + option * 60 * 1000,
      });
    }
  },

  clearSleepTimer: () => set({ sleepTimer: null, sleepTimerEndTime: null }),

  toggleShowBookProgress: () => set((s) => ({ showBookProgress: !s.showBookProgress })),
  toggleShowPercentage: () => set((s) => ({ showPercentage: !s.showPercentage })),

  setAutoBookmarkEnabled: (v) => set({ autoBookmarkEnabled: v }),
  setAutoBookmarkIntervalMins: (v) => set({ autoBookmarkIntervalMins: Math.max(1, Math.min(60, v)) }),

  setShowBookmarkMarkers: (v) => set({ showBookmarkMarkers: v }),
  setSeekBarMode: (v) => set({ seekBarMode: v }),

  recordBookSwitch: (fromBookId, toBookId) =>
    set((s) => ({
      bookSwitchCount: s.bookSwitchCount + 1,
      bookSwitchLog: [
        ...s.bookSwitchLog,
        { fromBookId, toBookId, timestamp: Date.now() },
      ],
    })),
  resetBookSwitchCounter: () => set({ bookSwitchCount: 0, bookSwitchLog: [] }),
}));
