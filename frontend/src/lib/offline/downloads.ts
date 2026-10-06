// SPDX-License-Identifier: AGPL-3.0-or-later
import axios from "axios";
import { create } from "zustand";
import { getPlaylist, getTrackStreamUrl, listPlaylistTracks, listTracks } from "../../api/music";
import { mediaUrl } from "../../api/client";
import type { MusicTrack } from "../../api/types";
import { useAuthStore } from "../../store/authStore";
import { usePlayerStore } from "../../store/playerStore";
import { toast } from "../toast";
import { t } from "../../i18n";
import * as db from "./db";
import type { OfflinePlaylist, OfflineTrack, QueuedDownload } from "./db";

/*
 * Songs kept on this device for offline playback.
 *
 * Every request is written to the queue in IndexedDB before anything is
 * fetched, and the queue runs one song at a time until it is empty. When the
 * network goes, the queue stops where it is and picks up again on the
 * `online` event, when the app comes back to the foreground, or on the next
 * launch. That is the whole resume story on iOS, which gives a home-screen web
 * app no background time at all: it can only download while it is open.
 *
 * One at a time on purpose — a phone on a weak connection finishes one song
 * sooner than it finishes three halves, and each song is held in memory until
 * it is written.
 */

const MAX_ATTEMPTS = 3;
/** When a request fails while the browser still claims to be online (captive
 *  portal, a tunnel), no `online` event will ever arrive to wake the queue. */
const STALLED_RETRY_MS = 30_000;
const OWNER_KEY = "owner";
/** Playlists with their own Home Screen icon, per device. Each one is a full
 *  offline copy, so the cap is really about storage and keeping it simple. */
export const MAX_HOME_PLAYLISTS = 5;

interface DownloadState {
  ready: boolean;
  /** Details of everything downloaded, keyed by track id. No audio in here. */
  downloaded: Record<string, OfflineTrack>;
  playlists: Record<string, OfflinePlaylist>;
  queue: QueuedDownload[];
  active: string | null;
  progress: { loaded: number; total: number | null } | null;
  waitingForNetwork: boolean;
  storageFull: boolean;
}

export const useDownloads = create<DownloadState>()(() => ({
  ready: false,
  downloaded: {},
  playlists: {},
  queue: [],
  active: null,
  progress: null,
  waitingForNetwork: false,
  storageFull: false,
}));

const set = useDownloads.setState;
const state = useDownloads.getState;

class HttpError extends Error {
  readonly status: number;
  constructor(status: number) {
    super(`HTTP ${status}`);
    this.status = status;
  }
}

function isNetworkFailure(e: unknown): boolean {
  if (!navigator.onLine) return true;
  // fetch() rejects with a TypeError for a failed or dropped connection; axios
  // reports the same thing as an error with no response.
  if (e instanceof TypeError) return true;
  return axios.isAxiosError(e) && !e.response;
}

function isQuotaError(e: unknown): boolean {
  return e instanceof DOMException && (e.name === "QuotaExceededError" || e.name === "NS_ERROR_DOM_QUOTA_REACHED");
}

const EXT_TYPES: Record<string, string> = {
  mp3: "audio/mpeg",
  m4a: "audio/mp4",
  mp4: "audio/mp4",
  aac: "audio/aac",
  flac: "audio/flac",
  ogg: "audio/ogg",
  opus: "audio/ogg",
  wav: "audio/wav",
};

/** Safari will not play a blob whose type it cannot place, and storage often
 *  answers with `application/octet-stream`. */
function audioType(contentType: string | null, url: string): string {
  if (contentType?.startsWith("audio/")) return contentType;
  const ext = new URL(url).pathname.split(".").pop()?.toLowerCase() ?? "";
  return EXT_TYPES[ext] ?? contentType ?? "audio/mpeg";
}

// ── Reading ──────────────────────────────────────────────────────────────

const streamUrls = new Map<string, string>();

/** A playable URL for a downloaded song, or null when it isn't on the device.
 *  Kept for the life of the page: the player may come back to it. */
export async function offlineStreamUrl(trackId: string): Promise<string | null> {
  const cached = streamUrls.get(trackId);
  if (cached) return cached;
  if (state().ready && !state().downloaded[trackId]) return null;
  try {
    const blob = await db.get<Blob>(db.STORES.audio, trackId);
    if (!blob) return null;
    const url = URL.createObjectURL(blob);
    streamUrls.set(trackId, url);
    return url;
  } catch {
    return null;
  }
}

/** A stored cover as a fresh object URL — the caller owns it and revokes it. */
export async function offlineCoverUrl(src: string): Promise<string | null> {
  try {
    const blob = await db.get<Blob>(db.STORES.covers, src);
    return blob ? URL.createObjectURL(blob) : null;
  } catch {
    return null;
  }
}

export function isDownloaded(trackId: string): boolean {
  return !!state().downloaded[trackId];
}

// ── The queue ────────────────────────────────────────────────────────────

let running = false;
let controller: AbortController | null = null;
let retryTimer: ReturnType<typeof setTimeout> | null = null;

/** `refresh` re-fetches a cover that is already kept, for when it may have
 *  changed; otherwise a kept one is left alone. */
export async function saveCover(src: string | null, { refresh = false }: { refresh?: boolean } = {}) {
  if (!src) return;
  try {
    if (!refresh && (await db.get(db.STORES.covers, src))) return;
    const token = useAuthStore.getState().token;
    const res = await fetch(mediaUrl(src) as string, {
      headers: token ? { Authorization: `Bearer ${token}` } : undefined,
    });
    if (!res.ok || !(res.headers.get("content-type") ?? "").startsWith("image/")) return;
    await db.put(db.STORES.covers, await res.blob(), src);
  } catch {
    // A song without its cover still plays; not worth failing the download.
  }
}

async function downloadOne(item: QueuedDownload, signal: AbortSignal) {
  const url = await getTrackStreamUrl(item.id);
  const res = await fetch(url, { signal });
  if (!res.ok) throw new HttpError(res.status);

  const total = Number(res.headers.get("content-length")) || item.track.size_bytes || null;
  const type = audioType(res.headers.get("content-type"), url);
  set({ progress: { loaded: 0, total } });

  let blob: Blob;
  if (res.body) {
    const reader = res.body.getReader();
    const chunks: BlobPart[] = [];
    let loaded = 0;
    let lastReport = 0;
    for (;;) {
      const { done, value } = await reader.read();
      if (done) break;
      chunks.push(value);
      loaded += value.byteLength;
      const now = Date.now();
      if (now - lastReport > 250) {
        lastReport = now;
        set({ progress: { loaded, total } });
      }
    }
    blob = new Blob(chunks, { type });
  } else {
    blob = new Blob([await res.blob()], { type });
  }

  await saveCover(item.track.cover_url);
  const entry: OfflineTrack = { id: item.id, track: item.track, bytes: blob.size, downloadedAt: Date.now() };
  await db.saveDownload(entry, blob);
  const owner = useAuthStore.getState().user?.id;
  if (owner) await db.put(db.STORES.meta, owner, OWNER_KEY);

  set((s) => ({
    downloaded: { ...s.downloaded, [entry.id]: entry },
    queue: s.queue.filter((q) => q.id !== entry.id),
  }));
}

function scheduleRetry() {
  if (retryTimer) return;
  retryTimer = setTimeout(() => {
    retryTimer = null;
    void runQueue();
  }, STALLED_RETRY_MS);
}

async function runQueue(): Promise<void> {
  if (running || !state().ready) return;
  running = true;
  try {
    for (;;) {
      const next = state().queue[0];
      if (!next) {
        set({ waitingForNetwork: false });
        return;
      }
      if (!navigator.onLine) {
        set({ waitingForNetwork: true });
        return;
      }
      set({ active: next.id, progress: null, waitingForNetwork: false, storageFull: false });
      controller = new AbortController();
      try {
        await downloadOne(next, controller.signal);
      } catch (e) {
        // Cancelled by the user: the entry is already gone, carry on.
        if (e instanceof DOMException && e.name === "AbortError") continue;
        if (isQuotaError(e)) {
          set({ storageFull: true });
          toast.error(t("common.downloads.outOfSpace"), t("common.downloads.outOfSpaceHint"));
          return;
        }
        if (isNetworkFailure(e)) {
          set({ waitingForNetwork: true });
          if (navigator.onLine) scheduleRetry();
          return;
        }
        // The server refused (deleted, no longer shared, …). Try again later
        // in the queue, and give up after a few rounds rather than forever.
        const attempts = next.attempts + 1;
        const lastError = e instanceof Error ? e.message : String(e);
        if (attempts >= MAX_ATTEMPTS) {
          await db.del(db.STORES.queue, next.id);
          set((s) => ({ queue: s.queue.filter((q) => q.id !== next.id) }));
          toast.error(t("common.downloads.songFailed"), next.track.title);
        } else {
          const retried = { ...next, attempts, lastError };
          await db.put(db.STORES.queue, retried);
          set((s) => ({ queue: [...s.queue.filter((q) => q.id !== next.id), retried] }));
        }
      }
    }
  } finally {
    controller = null;
    running = false;
    set({ active: null, progress: null });
  }
}

/** Queue songs for download. Returns how many were actually added. */
export async function downloadTracks(tracks: MusicTrack[]): Promise<number> {
  const s = state();
  const queued = new Set(s.queue.map((q) => q.id));
  const fresh = tracks.filter((t, i) => !s.downloaded[t.id] && !queued.has(t.id) && tracks.findIndex((x) => x.id === t.id) === i);
  if (fresh.length === 0) return 0;

  // Without this, Safari may clear the whole store under storage pressure.
  void navigator.storage?.persist?.().catch(() => {});

  const now = Date.now();
  const items: QueuedDownload[] = fresh.map((track, i) => ({ id: track.id, track, addedAt: now + i, attempts: 0 }));
  for (const item of items) await db.put(db.STORES.queue, item);
  set((st) => ({ queue: [...st.queue, ...items] }));
  void runQueue();
  return items.length;
}

export async function cancelDownload(trackId: string): Promise<void> {
  await db.del(db.STORES.queue, trackId);
  set((s) => ({ queue: s.queue.filter((q) => q.id !== trackId) }));
  if (state().active === trackId) controller?.abort();
}

export async function cancelAllDownloads(): Promise<void> {
  const ids = state().queue.map((q) => q.id);
  for (const id of ids) await db.del(db.STORES.queue, id);
  set({ queue: [] });
  controller?.abort();
}

export async function removeDownload(trackId: string): Promise<void> {
  const entry = state().downloaded[trackId];
  await db.deleteDownload(trackId);
  const url = streamUrls.get(trackId);
  if (url) {
    URL.revokeObjectURL(url);
    streamUrls.delete(trackId);
  }
  const rest = { ...state().downloaded };
  delete rest[trackId];
  set({ downloaded: rest, storageFull: false });
  const cover = entry?.track.cover_url;
  if (cover && !Object.values(rest).some((t) => t.track.cover_url === cover)) {
    await db.del(db.STORES.covers, cover).catch(() => {});
  }
  // Space was freed; a queue stopped on a full disk can go on.
  void runQueue();
}

/** Everything goes: downloads, covers and the queue. For signing out — the
 *  next person on this device must not find the last one's private music. */
export async function clearDownloads(): Promise<void> {
  controller?.abort();
  for (const url of streamUrls.values()) URL.revokeObjectURL(url);
  streamUrls.clear();
  set({ downloaded: {}, playlists: {}, queue: [], active: null, progress: null, waitingForNetwork: false, storageFull: false });
  await db.clearAll().catch(() => {});
}

// ── Playlists with their own Home Screen icon ───────────────────────────

export class HomePlaylistLimitError extends Error {
  constructor() {
    super(`Up to ${MAX_HOME_PLAYLISTS} playlists can have their own icon on this device.`);
  }
}

/** What updating a kept playlist would do, worked out before doing any of it. */
export interface PlaylistChanges {
  snapshot: OfflinePlaylist;
  /** In the playlist but not on the device: new songs, and any that failed before. */
  toDownload: MusicTrack[];
  /** On the device for this playlist, but no longer in it. */
  toRemove: MusicTrack[];
  /** Sum of the sizes the server knows; songs without one aren't counted. */
  bytes: number;
}

/** One small request: the playlist and its song list, nothing downloaded. */
export async function checkPlaylist(id: string): Promise<PlaylistChanges> {
  const [playlist, entries] = await Promise.all([getPlaylist(id), listPlaylistTracks(id)]);
  const snapshot: OfflinePlaylist = {
    id,
    name: playlist.name,
    coverUrl: playlist.cover_url,
    tracks: [...entries].sort((a, b) => a.position - b.position).map((e) => e.track),
    savedAt: Date.now(),
  };
  const { downloaded, playlists, queue } = state();
  const now = new Set(snapshot.tracks.map((t) => t.id));
  const queued = new Set(queue.map((q) => q.id));
  const toDownload = snapshot.tracks.filter((t) => !downloaded[t.id] && !queued.has(t.id));
  // Only songs this playlist brought, and that no other kept playlist still needs.
  const elsewhere = new Set(
    Object.values(playlists)
      .filter((p) => p.id !== id)
      .flatMap((p) => p.tracks.map((t) => t.id))
  );
  const before = playlists[id]?.tracks ?? [];
  const toRemove = before.filter((t) => !now.has(t.id) && downloaded[t.id] && !elsewhere.has(t.id));
  const bytes = toDownload.reduce((sum, t) => sum + (t.size_bytes ?? 0), 0);
  return { snapshot, toDownload, toRemove, bytes };
}

/** Do what `checkPlaylist` found: new list, new songs queued, removed ones deleted. */
export async function applyPlaylistChanges(changes: PlaylistChanges): Promise<void> {
  const { snapshot } = changes;
  // Cover before state: the page draws the Home Screen icon from it the
  // moment the playlist appears, and would otherwise find nothing yet.
  await saveCover(snapshot.coverUrl, { refresh: true });
  await db.put(db.STORES.playlists, snapshot);
  set((s) => ({ playlists: { ...s.playlists, [snapshot.id]: snapshot } }));
  for (const t of changes.toRemove) {
    const playing = usePlayerStore.getState().track;
    // Deleting the song that is playing would pull the file out from under it.
    if (playing?.kind === "music" && playing.trackId === t.id) continue;
    await removeDownload(t.id);
  }
  await downloadTracks(changes.toDownload);
}

/**
 * Keep a playlist, and every song in it, on this device — the first time,
 * when it is added to the Home Screen or its icon first opens.
 */
export async function savePlaylistOffline(id: string): Promise<OfflinePlaylist> {
  if (!state().playlists[id] && Object.keys(state().playlists).length >= MAX_HOME_PLAYLISTS) {
    throw new HomePlaylistLimitError();
  }
  const changes = await checkPlaylist(id);
  await applyPlaylistChanges(changes);
  return changes.snapshot;
}

/** Stop keeping the playlist. Its songs stay in Downloads — other playlists
 *  may share them, and removing songs is its own, explicit action there. */
export async function removePlaylistOffline(id: string): Promise<void> {
  await db.del(db.STORES.playlists, id);
  const rest = { ...state().playlists };
  delete rest[id];
  set({ playlists: rest });
}

// ── Start-up ─────────────────────────────────────────────────────────────

/**
 * Drop saved songs the library no longer has — deleted here, on another
 * device, or by a family admin. Deleting means gone everywhere
 * (docs/file-sync-plan.md §2 item 3), so a trashed song must not keep playing
 * offline in this browser.
 *
 * Only when signed in and online, and only on a list that actually loaded: an
 * unauthenticated request here would bounce a public page to sign-in, and a
 * failed fetch must never read as "nothing exists".
 */
async function dropDeletedSongs() {
  const saved = Object.keys(state().downloaded);
  if (saved.length === 0 || !useAuthStore.getState().token || !navigator.onLine) return;
  let live: Set<string>;
  try {
    live = new Set((await listTracks()).map((t) => t.id));
  } catch {
    return;
  }
  for (const id of saved) {
    if (!live.has(id)) await removeDownload(id).catch(() => {});
  }
}

async function dropIfOtherUser(userId: string | undefined) {
  if (!userId) return;
  const owner = await db.get<string>(db.STORES.meta, OWNER_KEY).catch(() => undefined);
  if (owner && owner !== userId) await clearDownloads();
}

let started = false;

export async function initDownloads(): Promise<void> {
  if (started || typeof indexedDB === "undefined") return;
  started = true;
  try {
    await dropIfOtherUser(useAuthStore.getState().user?.id);
    const [tracks, queue, playlists] = await Promise.all([
      db.getAll<OfflineTrack>(db.STORES.tracks),
      db.getAll<QueuedDownload>(db.STORES.queue),
      db.getAll<OfflinePlaylist>(db.STORES.playlists),
    ]);
    set({
      ready: true,
      downloaded: Object.fromEntries(tracks.map((t) => [t.id, t])),
      playlists: Object.fromEntries(playlists.map((p) => [p.id, p])),
      queue: queue.sort((a, b) => a.addedAt - b.addedAt),
    });
  } catch {
    // No IndexedDB (a locked-down private window): downloads just aren't offered.
    return;
  }

  useAuthStore.subscribe((s, prev) => {
    if (s.user?.id !== prev.user?.id) void dropIfOtherUser(s.user?.id);
  });

  void dropDeletedSongs();
  window.addEventListener("online", () => void runQueue());
  window.addEventListener("offline", () => set({ waitingForNetwork: state().queue.length > 0 }));
  // iOS suspends the page in the background; coming back is the moment to go on.
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible") void runQueue();
  });
  void runQueue();
}
