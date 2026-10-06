// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";
import type { AudioBookChapter } from "../api/types";

export interface PodcastPlayerTrack {
  kind: "podcast";
  epId: string;
  feedId: string;
  title: string;
  feedTitle: string | null;
  imageUrl: string | null;
  streamUrl: string;
  /** Duration in seconds (from episode metadata, may be null). */
  durationSecs: number | null;
  /** Resume from this position in seconds (0 = start from beginning). */
  resumePosition: number;
  /** The episode's notes from the feed, for the player's "Episode notes" sheet. */
  description?: string | null;
  /*
   * Set when this is a translated episode (guide §11a). Its id belongs to
   * `podcast_episode_translations`, so nothing about it may be reported: the
   * server has no progress endpoint for one, and `epId` is the episode whose
   * own position belongs to the original recording. The position is kept in
   * this browser instead (`lib/translationProgress`).
   */
  translationId?: string;
}

export interface AudiobookPlayerTrack {
  kind: "audiobook";
  bookId: string;
  fileId: string;
  filePosition: number;
  title: string;
  bookTitle: string;
  imageUrl: string | null;
  streamUrl: string;
  durationSecs: number | null;
  resumePosition: number;
  bookPositionOffsetSecs: number;
  bookTotalDurationSecs: number | null;
}

export interface MusicPlayerTrack {
  kind: "music";
  trackId: string;
  title: string;
  artist: string | null;
  /** For the Now Playing screen's links to the album; older saved queues lack them. */
  album?: string | null;
  albumArtist?: string | null;
  imageUrl: string | null;
  streamUrl: string;
  durationSecs: number | null;
  resumePosition: number;
}

export type PlayerTrack = PodcastPlayerTrack | AudiobookPlayerTrack | MusicPlayerTrack;

/** Where the queue was started from — Now Playing names it and links back to it. */
export interface QueueSource {
  kind: "playlist" | "album" | "artist" | "genre";
  label: string;
  /** The app route to open it. */
  path: string;
}

export type RepeatMode = "none" | "all" | "one";

/** Off, a wall-clock deadline in ms, or "stop when this track ends". */
export type SleepTimer = { mode: "off" } | { mode: "at"; endsAt: number } | { mode: "endOfTrack" };

export const SPEED_MIN = 0.5;
export const SPEED_MAX = 3;

interface PlayerState {
  track: PlayerTrack | null;
  source: QueueSource | null;
  queue: PlayerTrack[];
  /** The original (un-shuffled) queue for restoring order */
  originalQueue: PlayerTrack[];
  queueIndex: number;
  playing: boolean;
  shuffle: boolean;
  repeat: RepeatMode;
  volume: number;
  muted: boolean;
  /** Show the queue sidebar / panel */
  showQueue: boolean;
  /** Crossfade duration in seconds (0 = disabled) */
  crossfadeSecs: number;
  /** Playback rate. Server clamps its stored default to 0.5–3×; match that here. */
  speed: number;
  sleepTimer: SleepTimer;
  /** Chapters of the audiobook now playing, when it has any. Empty is normal —
   *  nothing extracts chapters from M4B/ID3 server-side yet. */
  chapters: AudioBookChapter[];
  /** How far into the whole book each queued file starts, by file id. */
  skipForwardSecs: number;
  skipBackwardSecs: number;
  /** Last saved position, in seconds. Kept here so the cross-device queue can
   *  hand off where you actually are, not where the item started. */
  position: number;

  // ── Actions ────────────────────────────────────
  play: (track: PlayerTrack) => void;
  playQueue: (queue: PlayerTrack[], startIndex?: number, source?: QueueSource | null) => void;
  playNext: () => void;
  playPrev: () => void;
  addToQueue: (track: PlayerTrack) => void;
  removeFromQueue: (index: number) => void;
  clearQueue: () => void;
  stop: () => void;
  setPlaying: (playing: boolean) => void;
  toggleShuffle: () => void;
  cycleRepeat: () => void;
  setVolume: (v: number) => void;
  toggleMute: () => void;
  setShowQueue: (v: boolean) => void;
  setCrossfadeSecs: (secs: number) => void;
  setSpeed: (v: number) => void;
  setSleepTimer: (t: SleepTimer) => void;
  setChapters: (c: AudioBookChapter[]) => void;
  setSkipSecs: (forward: number, backward: number) => void;
  setPosition: (secs: number) => void;
  /** Jump to a queue index directly (queue panel, chapter list). */
  playIndex: (index: number) => void;
}

function shuffleArray<T>(arr: T[]): T[] {
  const a = [...arr];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

export const usePlayerStore = create<PlayerState>((set, get) => ({
  track: null,
  source: null,
  queue: [],
  originalQueue: [],
  queueIndex: 0,
  playing: false,
  shuffle: false,
  repeat: "none",
  volume: 1,
  muted: false,
  showQueue: false,
  crossfadeSecs: 0,
  speed: 1,
  sleepTimer: { mode: "off" },
  chapters: [],
  skipForwardSecs: 30,
  skipBackwardSecs: 15,
  position: 0,

  play: (track) =>
    set({
      track,
      source: null,
      queue: [track],
      originalQueue: [track],
      queueIndex: 0,
      playing: true,
    }),

  playQueue: (queue, startIndex = 0, source = null) => {
    const state = get();
    set({ source });
    if (state.shuffle) {
      // Keep the starting track first, shuffle the rest
      const startTrack = queue[startIndex];
      const rest = queue.filter((_, i) => i !== startIndex);
      const shuffled = [startTrack, ...shuffleArray(rest)];
      set({
        originalQueue: queue,
        queue: shuffled,
        queueIndex: 0,
        track: shuffled[0] ?? null,
        playing: queue.length > 0,
      });
    } else {
      set({
        originalQueue: queue,
        queue,
        queueIndex: startIndex,
        track: queue[startIndex] ?? null,
        playing: queue.length > 0,
      });
    }
  },

  playNext: () =>
    set((state) => {
      const nextIndex = state.queueIndex + 1;

      // Repeat One — restart current track
      if (state.repeat === "one") {
        return {
          ...state,
          // The caller (PlayerBar) will detect same track and seek to 0
          playing: true,
        };
      }

      if (nextIndex >= state.queue.length) {
        // Repeat All — wrap around
        if (state.repeat === "all" && state.queue.length > 0) {
          return {
            ...state,
            queueIndex: 0,
            track: state.queue[0],
            playing: true,
          };
        }
        // No repeat — stop
        return { ...state, playing: false };
      }

      return {
        ...state,
        queueIndex: nextIndex,
        track: state.queue[nextIndex],
        playing: true,
      };
    }),

  playPrev: () =>
    set((state) => {
      const prevIndex = state.queueIndex - 1;
      if (prevIndex < 0) {
        // Wrap around if repeat-all, otherwise stay at start
        if (state.repeat === "all" && state.queue.length > 0) {
          const lastIdx = state.queue.length - 1;
          return { ...state, queueIndex: lastIdx, track: state.queue[lastIdx], playing: true };
        }
        return state;
      }
      return {
        ...state,
        queueIndex: prevIndex,
        track: state.queue[prevIndex],
        playing: true,
      };
    }),

  addToQueue: (track) =>
    set((state) => ({
      queue: [...state.queue, track],
      originalQueue: [...state.originalQueue, track],
      // If nothing is playing, start playing the added track
      ...(state.track === null
        ? { track, queueIndex: state.queue.length, playing: true }
        : {}),
    })),

  removeFromQueue: (index) =>
    set((state) => {
      const newQueue = state.queue.filter((_, i) => i !== index);
      let newIndex = state.queueIndex;
      let newTrack = state.track;

      if (index < state.queueIndex) {
        newIndex--;
      } else if (index === state.queueIndex) {
        // Removed the currently-playing track
        if (newQueue.length === 0) {
          return { queue: newQueue, originalQueue: newQueue, track: null, queueIndex: 0, playing: false };
        }
        newIndex = Math.min(newIndex, newQueue.length - 1);
        newTrack = newQueue[newIndex];
      }

      return { queue: newQueue, queueIndex: newIndex, track: newTrack };
    }),

  clearQueue: () =>
    set((state) => {
      // Keep current track, remove everything after it
      if (!state.track) return { queue: [], originalQueue: [], queueIndex: 0 };
      return {
        queue: [state.track],
        originalQueue: [state.track],
        queueIndex: 0,
      };
    }),

  playIndex: (index) =>
    set((state) => {
      if (index < 0 || index >= state.queue.length) return state;
      return { queueIndex: index, track: state.queue[index], playing: true };
    }),

  setSpeed: (v) => set({ speed: Math.min(SPEED_MAX, Math.max(SPEED_MIN, v)) }),
  setSleepTimer: (sleepTimer) => set({ sleepTimer }),
  setChapters: (chapters) => set({ chapters }),
  setSkipSecs: (forward, backward) => set({ skipForwardSecs: forward, skipBackwardSecs: backward }),
  setPosition: (position) => set({ position }),

  stop: () =>
    set({
      track: null,
      source: null,
      queue: [],
      originalQueue: [],
      queueIndex: 0,
      playing: false,
      showQueue: false,
      chapters: [],
      sleepTimer: { mode: "off" },
    }),

  setPlaying: (playing) => set({ playing }),

  toggleShuffle: () =>
    set((state) => {
      const newShuffle = !state.shuffle;
      if (newShuffle) {
        // Shuffle: keep current track at position 0, shuffle the rest
        const currentTrack = state.track;
        const rest = state.queue.filter((_, i) => i !== state.queueIndex);
        const shuffled = currentTrack
          ? [currentTrack, ...shuffleArray(rest)]
          : shuffleArray(state.queue);
        return { shuffle: true, queue: shuffled, queueIndex: 0 };
      } else {
        // Un-shuffle: restore original order, find current track
        const currentTrack = state.track;
        const oq = state.originalQueue;
        const idx = currentTrack
          ? oq.findIndex((t) => trackId(t) === trackId(currentTrack))
          : 0;
        return { shuffle: false, queue: oq, queueIndex: idx >= 0 ? idx : 0 };
      }
    }),

  cycleRepeat: () =>
    set((state) => {
      const next: Record<RepeatMode, RepeatMode> = {
        none: "all",
        all: "one",
        one: "none",
      };
      return { repeat: next[state.repeat] };
    }),

  setVolume: (v) => set({ volume: Math.max(0, Math.min(1, v)), muted: false }),
  toggleMute: () => set((state) => ({ muted: !state.muted })),
  setShowQueue: (v) => set({ showQueue: v }),
  setCrossfadeSecs: (secs) => set({ crossfadeSecs: Math.max(0, Math.min(12, secs)) }),
}));

/** Unique identifier for a PlayerTrack regardless of kind */
function trackId(t: PlayerTrack): string {
  if (t.kind === "podcast") return `podcast:${t.epId}`;
  if (t.kind === "audiobook") return `book:${t.bookId}:${t.fileId}`;
  return `music:${t.trackId}`;
}
