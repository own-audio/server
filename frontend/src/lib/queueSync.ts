// SPDX-License-Identifier: AGPL-3.0-or-later
import { getQueue, putQueue, type QueueItem } from "../api/playback";
import { getBook } from "../api/audiobooks";
import { getFeed, listEpisodes } from "../api/podcasts";
import { getTrack } from "../api/music";
import { usePlayerStore, type PlayerTrack } from "../store/playerStore";
import { playBook, playEpisode, playTracks } from "./play";

/**
 * The cross-device play queue (`GET|PUT /playback/queue`), shared with the
 * iOS/Android/Mac clients and with any Subsonic app through `savePlayQueue`.
 *
 * Semantics are last-write-wins, so this deliberately does **not** push on
 * every tick: it writes when the queue itself changes, carrying the last saved
 * position so another device resumes where you actually are.
 *
 * A queue written elsewhere is *reported*, never applied on its own. Silently
 * replacing what someone started on their phone is worse than doing nothing —
 * `followQueue()` is what an explicit "continue here" runs.
 */

const PUSH_DEBOUNCE_MS = 2_000;

let pushTimer: ReturnType<typeof setTimeout> | null = null;
let lastPushedSignature = "";
let lastSeenUpdatedAt: string | null = null;

function toQueueItem(t: PlayerTrack): QueueItem {
  // `item_id` is the thing, `part_id` the piece of it — so a podcast item is
  // the *feed* plus the episode. Sending the episode as `item_id` loses the
  // feed, and no other device can resolve a stream URL without it.
  if (t.kind === "audiobook") return { media_kind: "audiobook", item_id: t.bookId, part_id: t.fileId };
  if (t.kind === "podcast") return { media_kind: "podcast", item_id: t.feedId, part_id: t.epId };
  return { media_kind: "music", item_id: t.trackId };
}

function signature(items: QueueItem[], index: number): string {
  return `${index}|${items.map((i) => `${i.media_kind}:${i.item_id}:${i.part_id ?? ""}`).join(",")}`;
}

/** Push the current queue, debounced. Safe to call on every store change. */
export function scheduleQueuePush(): void {
  if (pushTimer) clearTimeout(pushTimer);
  pushTimer = setTimeout(() => {
    void pushNow();
  }, PUSH_DEBOUNCE_MS);
}

/** `force` writes even when the queue itself hasn't changed — used at the
 *  moments a handoff is likely (a pause, the page going away), so the other
 *  device resumes at the right place rather than at zero. */
export async function pushNow(force = false): Promise<void> {
  const { queue, queueIndex, position } = usePlayerStore.getState();
  if (queue.length === 0) return;
  const items = queue.map(toQueueItem);
  const sig = signature(items, queueIndex);
  if (!force && sig === lastPushedSignature) return;
  lastPushedSignature = sig;
  try {
    const res = await putQueue({ items, current_index: queueIndex, position_secs: Math.max(0, Math.round(position)) });
    lastSeenUpdatedAt = res.updated_at;
  } catch {
    // A queue that failed to sync is a cosmetic loss; the next change retries.
    lastPushedSignature = "";
  }
}

export interface ForeignQueue {
  device: string | null;
  deviceLabel: string | null;
  itemCount: number;
  updatedAt: string;
  items: QueueItem[];
  currentIndex: number;
  positionSecs: number;
}

/**
 * Read the server's queue. Returns it when another device has written one
 * worth offering; null when it is ours, unchanged, or empty.
 *
 * `firstLook` is the page-load case: there is no "since last time" to compare
 * against, so anything another device left behind counts.
 */
export async function checkForeignQueue(firstLook = false): Promise<ForeignQueue | null> {
  try {
    const res = await getQueue();
    if (res.items.length === 0) return null;

    const unchanged = res.updated_at === lastSeenUpdatedAt;
    lastSeenUpdatedAt = res.updated_at;
    if (!firstLook && unchanged) return null;
    if (res.updated_by_device === "web") return null;

    return {
      device: res.updated_by_device,
      deviceLabel: res.updated_by_device_label,
      itemCount: res.items.length,
      updatedAt: res.updated_at,
      items: res.items,
      currentIndex: res.current_index,
      positionSecs: res.position_secs,
    };
  } catch {
    return null;
  }
}

/**
 * Play what another device left in the queue.
 *
 * Each kind needs a different amount of rebuilding: the queue stores ids, not
 * playable tracks, and stream URLs are resolved at play time because they
 * expire. Music is the only kind where the whole queue is restored — an
 * audiobook or podcast queue is one item and its own ordering.
 */
export async function followQueue(q: ForeignQueue): Promise<void> {
  const current = q.items[Math.min(Math.max(0, q.currentIndex), q.items.length - 1)];
  if (!current) return;

  if (current.media_kind === "audiobook") {
    const book = await getBook(current.item_id);
    await playBook(book, current.part_id, q.positionSecs);
    return;
  }

  if (current.media_kind === "podcast") {
    const [feed, episodes] = await Promise.all([getFeed(current.item_id), listEpisodes(current.item_id, 200)]);
    const episode = episodes.find((e) => e.id === current.part_id);
    if (episode) await playEpisode(current.item_id, episode, feed, q.positionSecs);
    return;
  }

  const tracks = await Promise.all(q.items.filter((i) => i.media_kind === "music").map((i) => getTrack(i.item_id)));
  const startAt = Math.max(0, q.items.slice(0, q.currentIndex).filter((i) => i.media_kind === "music").length);
  await playTracks(tracks, Math.min(startAt, Math.max(0, tracks.length - 1)));
}

/** Start syncing: push on queue changes, and look for a takeover on load and
 *  whenever the window regains focus. */
export function startQueueSync(onForeignQueue: (q: ForeignQueue) => void): () => void {
  let previous = signature(
    usePlayerStore.getState().queue.map(toQueueItem),
    usePlayerStore.getState().queueIndex
  );

  const unsubscribe = usePlayerStore.subscribe((state) => {
    const sig = signature(state.queue.map(toQueueItem), state.queueIndex);
    if (sig === previous) return;
    previous = sig;
    if (state.queue.length > 0) scheduleQueuePush();
  });

  // On load: nothing is playing here yet, so anything on the server is worth
  // offering. Without this the app only noticed a takeover after you switched
  // away and back, which is the one moment you don't need telling.
  void checkForeignQueue(true).then((q) => {
    if (q && !usePlayerStore.getState().track) onForeignQueue(q);
  });

  const onFocus = () => {
    void checkForeignQueue().then((q) => {
      if (q) onForeignQueue(q);
    });
  };
  window.addEventListener("focus", onFocus);

  return () => {
    unsubscribe();
    window.removeEventListener("focus", onFocus);
    if (pushTimer) clearTimeout(pushTimer);
  };
}
