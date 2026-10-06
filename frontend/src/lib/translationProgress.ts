// SPDX-License-Identifier: AGPL-3.0-or-later
/*
 * Where a translated episode got to, remembered in this browser.
 *
 * The server keeps no progress for a translation and must not be asked to:
 * its id belongs to `podcast_episode_translations`, and the episode's own
 * position belongs to the original recording — writing one against the other
 * would move someone's place in a different piece of audio
 * (`docs/android-client-guide.md` §11a, "Known caveats").
 *
 * So it lives in `localStorage`, together with enough about the translation to
 * list it again (title, show, language) without asking the server for anything.
 */

const KEY = "audio2.translationProgress.v1";

export interface TranslationProgress {
  translationId: string;
  episodeId: string;
  episodeTitle: string;
  showTitle: string | null;
  targetLanguage: string;
  streamUrl?: string;
  positionSecs: number;
  durationSecs: number | null;
  completed: boolean;
  updatedAt: number;
}

type Store = Record<string, TranslationProgress>;

function read(): Store {
  try {
    const raw = localStorage.getItem(KEY);
    return raw ? (JSON.parse(raw) as Store) : {};
  } catch {
    // A browser with storage blocked simply forgets; it must never break playback.
    return {};
  }
}

function write(store: Store): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(store));
  } catch {
    // Full or blocked: the position is lost, which is a worse experience, not a broken one.
  }
  listeners.forEach((l) => l());
}

const listeners = new Set<() => void>();

/** Subscribe for React: every save notifies, so a row and the Home widget both follow. */
export function subscribeToTranslationProgress(listener: () => void): () => void {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Records what a translation is, before it plays — a position alone could never be listed. */
export function rememberTranslation(
  entry: Omit<TranslationProgress, "positionSecs" | "completed" | "updatedAt"> &
    Partial<Pick<TranslationProgress, "positionSecs" | "completed">>
): void {
  const store = read();
  const known = store[entry.translationId];
  store[entry.translationId] = {
    ...known,
    ...entry,
    positionSecs: entry.positionSecs ?? known?.positionSecs ?? 0,
    completed: entry.completed ?? known?.completed ?? false,
    updatedAt: Date.now(),
  };
  write(store);
}

/** Ignores a position for a translation nobody has remembered: it could never be offered again. */
export function saveTranslationPosition(translationId: string, positionSecs: number, completed: boolean): void {
  const store = read();
  const known = store[translationId];
  if (!known) return;
  store[translationId] = { ...known, positionSecs: Math.max(0, positionSecs), completed, updatedAt: Date.now() };
  write(store);
}

export function getTranslationProgress(translationId: string): TranslationProgress | undefined {
  return read()[translationId];
}

/** Unfinished translations, newest first — what Continue listening offers. */
export function listTranslationsInProgress(): TranslationProgress[] {
  return Object.values(read())
    .filter((t) => !t.completed && t.positionSecs > 0)
    .sort((a, b) => b.updatedAt - a.updatedAt);
}
