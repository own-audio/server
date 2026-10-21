// SPDX-License-Identifier: AGPL-3.0-or-later
import type { QueryClient } from "@tanstack/react-query";
import * as db from "./offline/db";
import { useAuthStore } from "../store/authStore";

/*
 * The music library as it was last seen, kept on the device, so the music pages
 * show it at once instead of waiting for the whole track list (with 9,000 songs:
 * 1.4 MB and ~0.6 s on the server, every time a page is opened). The request
 * still goes out; its answer replaces the copy a moment later.
 *
 * Kept per account in the offline database's `meta` store, which signing out
 * empties (`clearDownloads` → `clearAll`).
 */

const KEYS = [["music-tracks"], ["music-albums"]] as const;

interface Snapshot {
  userId: string;
  savedAt: number;
  data: unknown;
}

const metaKey = (key: readonly string[]) => `library:${key.join("/")}`;

export function initLibraryCache(queryClient: QueryClient) {
  const userId = useAuthStore.getState().user?.id;
  if (userId) {
    for (const key of KEYS) {
      void db
        .get<Snapshot>(db.STORES.meta, metaKey(key))
        .then((snap) => {
          // Fresh data that arrived first wins; so does another account's absence.
          if (!snap || snap.userId !== userId || queryClient.getQueryData(key) !== undefined) return;
          // Dated when it was saved, so it counts as stale and is fetched again.
          queryClient.setQueryData(key, snap.data, { updatedAt: snap.savedAt });
        })
        .catch(() => {});
    }
  }

  // Every fresh answer is kept, a moment later and once per burst.
  const timers = new Map<string, ReturnType<typeof setTimeout>>();
  queryClient.getQueryCache().subscribe((event) => {
    if (event.type !== "updated" || event.action.type !== "success" || event.action.manual) return;
    const key = event.query.queryKey;
    if (!KEYS.some((k) => k.length === key.length && k.every((part, i) => part === key[i]))) return;
    const id = metaKey(key as readonly string[]);
    clearTimeout(timers.get(id));
    timers.set(
      id,
      setTimeout(() => {
        const user = useAuthStore.getState().user?.id;
        if (!user) return;
        const snap: Snapshot = { userId: user, savedAt: Date.now(), data: event.query.state.data };
        void db.put(db.STORES.meta, snap, id).catch(() => {});
      }, 2000),
    );
  });
}
