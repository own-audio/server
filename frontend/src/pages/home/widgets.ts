// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";
import type { PlainKey } from "../../i18n";

/* Which Home widgets show, and in what order — a per-viewer preference, so
   localStorage is the right home for it (mirrors the Mac's HomeWidgetSettingsStore). */

export type HomeWidgetId =
  | "continue"
  | "storage"
  | "recentSongs"
  | "recentAlbums"
  | "recentPlaylists"
  | "booksInProgress"
  | "newBooks"
  | "recentBooks"
  | "latestEpisodes"
  | "activity"
  | "weekSummary"
  | "listeningChart"
  | "quickLinks";

/* Label and description are catalog keys, translated where the sheet renders them. */
export const ALL_WIDGETS: { id: HomeWidgetId; label: PlainKey; description: PlainKey }[] = [
  { id: "quickLinks", label: "home.widget.quickLinks.label", description: "home.widget.quickLinks.description" },
  { id: "weekSummary", label: "home.widget.weekSummary.label", description: "home.widget.weekSummary.description" },
  { id: "continue", label: "home.widget.continue.label", description: "home.widget.continue.description" },
  { id: "storage", label: "home.widget.storage.label", description: "home.widget.storage.description" },
  { id: "recentSongs", label: "home.widget.recentSongs.label", description: "home.widget.recentSongs.description" },
  { id: "recentAlbums", label: "home.widget.recentAlbums.label", description: "home.widget.recentAlbums.description" },
  { id: "recentPlaylists", label: "home.widget.recentPlaylists.label", description: "home.widget.recentPlaylists.description" },
  { id: "booksInProgress", label: "home.widget.booksInProgress.label", description: "home.widget.booksInProgress.description" },
  { id: "newBooks", label: "home.widget.newBooks.label", description: "home.widget.newBooks.description" },
  { id: "recentBooks", label: "home.widget.recentBooks.label", description: "home.widget.recentBooks.description" },
  { id: "latestEpisodes", label: "home.widget.latestEpisodes.label", description: "home.widget.latestEpisodes.description" },
  { id: "listeningChart", label: "home.widget.listeningChart.label", description: "home.widget.listeningChart.description" },
  { id: "activity", label: "home.widget.activity.label", description: "home.widget.activity.description" },
];

const KEY = "own-audio-home-widgets";
/* Widgets added later are opt-in, except Quick links and This week, which a
   fresh Home opens with. */
const DEFAULT: HomeWidgetId[] = ["quickLinks", "weekSummary", "continue", "storage", "recentSongs", "recentAlbums", "recentPlaylists"];

/* Widgets too useful to leave for people to discover: added once to the top
   of a layout saved before they existed. Remembered as offered, so hiding one
   afterwards sticks. */
const OFFER_ONCE: HomeWidgetId[] = ["quickLinks"];
const OFFERED_KEY = "own-audio-home-widgets-offered";

function offerNew(saved: HomeWidgetId[]): HomeWidgetId[] {
  try {
    const offered = new Set<string>(JSON.parse(localStorage.getItem(OFFERED_KEY) ?? "[]") as string[]);
    const fresh = OFFER_ONCE.filter((id) => !offered.has(id) && !saved.includes(id));
    localStorage.setItem(OFFERED_KEY, JSON.stringify([...new Set([...offered, ...OFFER_ONCE])]));
    if (fresh.length === 0) return saved;
    const next = [...fresh, ...saved];
    localStorage.setItem(KEY, JSON.stringify(next));
    return next;
  } catch {
    return saved;
  }
}

function read(): HomeWidgetId[] {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) {
      // A fresh Home already has them; hiding one later must stick.
      localStorage.setItem(OFFERED_KEY, JSON.stringify(OFFER_ONCE));
      return DEFAULT;
    }
    const parsed = JSON.parse(raw) as unknown;
    if (!Array.isArray(parsed)) return DEFAULT;
    const valid = new Set<string>(ALL_WIDGETS.map((w) => w.id));
    return offerNew(parsed.filter((x): x is HomeWidgetId => typeof x === "string" && valid.has(x)));
  } catch {
    return DEFAULT;
  }
}

interface WidgetState {
  enabled: HomeWidgetId[];
  toggle: (id: HomeWidgetId) => void;
  move: (id: HomeWidgetId, dir: -1 | 1) => void;
}

export const useHomeWidgets = create<WidgetState>()((set) => {
  const persist = (enabled: HomeWidgetId[]) => {
    try {
      localStorage.setItem(KEY, JSON.stringify(enabled));
    } catch {
      // per-viewer convenience only — a blocked store just means no memory
    }
    return { enabled };
  };
  return {
    enabled: read(),
    toggle: (id) =>
      set((s) => persist(s.enabled.includes(id) ? s.enabled.filter((x) => x !== id) : [...s.enabled, id])),
    move: (id, dir) =>
      set((s) => {
        const i = s.enabled.indexOf(id);
        const j = i + dir;
        if (i < 0 || j < 0 || j >= s.enabled.length) return s;
        const next = [...s.enabled];
        [next[i], next[j]] = [next[j], next[i]];
        return persist(next);
      }),
  };
});
