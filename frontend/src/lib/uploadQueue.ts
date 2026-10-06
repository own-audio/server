// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

/**
 * One upload queue for the whole app, so uploads keep running while the user
 * browses elsewhere.
 *
 * Two things it is honest about, because the API gives us nothing better:
 *
 * - **A failed upload restarts from zero.** There is no resumable upload and no
 *   progress endpoint; the bytes already sent are gone. Anything that starts a
 *   large upload has to say so before the user commits to it.
 * - Progress is counted **client-side**, from what the browser has sent. The
 *   server does not report ingest progress, so "uploaded" means "the request
 *   finished", not "the file is playable".
 */

export type UploadStatus = "queued" | "uploading" | "done" | "failed" | "cancelled";

export interface UploadItem {
  id: string;
  /** What the user recognises: a book title, or a track's file name. */
  label: string;
  kind: "audiobook" | "music" | "cover";
  status: UploadStatus;
  /** 0–1, counted from bytes sent. */
  progress: number;
  bytes: number;
  error?: string;
}

interface UploadQueueState {
  items: UploadItem[];
  /** True while anything is in flight; drives the sidebar indicator. */
  active: boolean;
  add: (item: Omit<UploadItem, "progress" | "status">) => void;
  update: (id: string, patch: Partial<UploadItem>) => void;
  remove: (id: string) => void;
  clearFinished: () => void;
}

export const useUploadQueue = create<UploadQueueState>()((set) => ({
  items: [],
  active: false,
  add: (item) =>
    set((s) => ({
      items: [...s.items, { ...item, status: "queued", progress: 0 }],
      active: true,
    })),
  update: (id, patch) =>
    set((s) => {
      const items = s.items.map((i) => (i.id === id ? { ...i, ...patch } : i));
      return { items, active: items.some((i) => i.status === "uploading" || i.status === "queued") };
    }),
  remove: (id) =>
    set((s) => {
      const items = s.items.filter((i) => i.id !== id);
      return { items, active: items.some((i) => i.status === "uploading" || i.status === "queued") };
    }),
  clearFinished: () =>
    set((s) => ({ items: s.items.filter((i) => i.status === "uploading" || i.status === "queued") })),
}));

export function newUploadId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

/** Run `worker` over `items` with at most `limit` in flight, collecting failures
 *  rather than aborting the rest — a batch of 12 with one bad file should land 11. */
export async function runWithLimit<T>(
  items: T[],
  limit: number,
  worker: (item: T, index: number) => Promise<void>
): Promise<{ failed: { item: T; error: unknown }[] }> {
  const failed: { item: T; error: unknown }[] = [];
  let next = 0;
  await Promise.all(
    Array.from({ length: Math.min(limit, items.length) }, async () => {
      while (next < items.length) {
        const i = next++;
        try {
          await worker(items[i], i);
        } catch (error) {
          failed.push({ item: items[i], error });
        }
      }
    })
  );
  return { failed };
}
