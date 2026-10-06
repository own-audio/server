// SPDX-License-Identifier: AGPL-3.0-or-later
import type { MusicTrack } from "../../api/types";

/*
 * The offline library, in IndexedDB.
 *
 * Audio lives in its own store, apart from the track details: listing what is
 * downloaded must not pull every file off disk, and Safari reads a stored Blob
 * lazily only if nothing asks for it.
 *
 * The queue is stored too, not held in memory. A download asked for on a train
 * is still owed once the connection or the app comes back — which on iOS is the
 * only way it can finish, since a home-screen web app gets no background time.
 */

const DB_NAME = "own-audio-offline";
const DB_VERSION = 2;

export const STORES = {
  tracks: "tracks",
  audio: "audio",
  covers: "covers",
  queue: "queue",
  meta: "meta",
  playlists: "playlists",
} as const;

export interface OfflineTrack {
  id: string;
  /** Snapshot taken at download time, so the Downloads view renders offline. */
  track: MusicTrack;
  bytes: number;
  downloadedAt: number;
}

/** A playlist kept for its own Home Screen icon: its details and song order at
 *  the last sync, so the player page can open with no connection. */
export interface OfflinePlaylist {
  id: string;
  name: string;
  coverUrl: string | null;
  tracks: MusicTrack[];
  savedAt: number;
}

export interface QueuedDownload {
  id: string;
  track: MusicTrack;
  addedAt: number;
  attempts: number;
  lastError?: string;
}

let dbPromise: Promise<IDBDatabase> | null = null;

function openDb(): Promise<IDBDatabase> {
  dbPromise ??= new Promise((resolve, reject) => {
    const req = indexedDB.open(DB_NAME, DB_VERSION);
    req.onupgradeneeded = (e) => {
      const db = req.result;
      if (e.oldVersion < 1) {
        db.createObjectStore(STORES.tracks, { keyPath: "id" });
        db.createObjectStore(STORES.audio);
        db.createObjectStore(STORES.covers);
        db.createObjectStore(STORES.queue, { keyPath: "id" });
        db.createObjectStore(STORES.meta);
      }
      if (e.oldVersion < 2) db.createObjectStore(STORES.playlists, { keyPath: "id" });
    };
    req.onsuccess = () => {
      const db = req.result;
      // Another tab upgrading the schema must not be blocked by this one.
      db.onversionchange = () => {
        db.close();
        dbPromise = null;
      };
      resolve(db);
    };
    req.onerror = () => {
      dbPromise = null;
      reject(req.error);
    };
  });
  return dbPromise;
}

function done(tx: IDBTransaction): Promise<void> {
  return new Promise((resolve, reject) => {
    tx.oncomplete = () => resolve();
    tx.onabort = () => reject(tx.error ?? new DOMException("Transaction aborted", "AbortError"));
    tx.onerror = () => reject(tx.error);
  });
}

function result<T>(req: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

type StoreName = (typeof STORES)[keyof typeof STORES];

export async function get<T>(store: StoreName, key: IDBValidKey): Promise<T | undefined> {
  const db = await openDb();
  return result(db.transaction(store).objectStore(store).get(key)) as Promise<T | undefined>;
}

export async function getAll<T>(store: StoreName): Promise<T[]> {
  const db = await openDb();
  return result(db.transaction(store).objectStore(store).getAll()) as Promise<T[]>;
}

export async function put(store: StoreName, value: unknown, key?: IDBValidKey): Promise<void> {
  const db = await openDb();
  const tx = db.transaction(store, "readwrite");
  tx.objectStore(store).put(value, key);
  return done(tx);
}

export async function del(store: StoreName, key: IDBValidKey): Promise<void> {
  const db = await openDb();
  const tx = db.transaction(store, "readwrite");
  tx.objectStore(store).delete(key);
  return done(tx);
}

/** A finished download: file and details land together or not at all, so a
 *  crash mid-write can never leave a listed track with no audio behind it. */
export async function saveDownload(entry: OfflineTrack, audio: Blob): Promise<void> {
  const db = await openDb();
  const tx = db.transaction([STORES.tracks, STORES.audio, STORES.queue], "readwrite");
  tx.objectStore(STORES.audio).put(audio, entry.id);
  tx.objectStore(STORES.tracks).put(entry);
  tx.objectStore(STORES.queue).delete(entry.id);
  return done(tx);
}

export async function deleteDownload(id: string): Promise<void> {
  const db = await openDb();
  const tx = db.transaction([STORES.tracks, STORES.audio], "readwrite");
  tx.objectStore(STORES.tracks).delete(id);
  tx.objectStore(STORES.audio).delete(id);
  return done(tx);
}

export async function clearAll(): Promise<void> {
  const db = await openDb();
  const names = Object.values(STORES);
  const tx = db.transaction(names, "readwrite");
  for (const n of names) tx.objectStore(n).clear();
  return done(tx);
}
