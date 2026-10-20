// SPDX-License-Identifier: AGPL-3.0-or-later
import { mediaUrl } from "../api/client";
import { offlineCoverUrl } from "./offline/downloads";

// Images that need the auth header, loaded for every <AuthImage> at once.
// Scrolling a 9,000-song list asked for thousands of covers together: the
// browser ran out of connections and the server's rate limit refused the rest.
// Here a few load at a time, one request per address however many places show
// it, the last few hundred are kept, and a request nobody waits for any more
// is dropped before it starts.

const MAX_PARALLEL = 4;
// The server allows a device about 20 requests a second; covers take half of
// that at most, so a fast scroll never starves playback of its share.
const MIN_GAP_MS = 100;
const KEEP = 400;

interface Entry {
  promise: Promise<string | null>;
  waiters: number;
  started: boolean;
  drop: () => void;
}

const entries = new Map<string, Entry>();
const queue: Array<{ key: string; run: () => void }> = [];
let running = 0;
let lastStart = 0;
let timer: ReturnType<typeof setTimeout> | null = null;

function next() {
  if (timer) return;
  while (running < MAX_PARALLEL && queue.length) {
    const wait = lastStart + MIN_GAP_MS - Date.now();
    if (wait > 0) {
      timer = setTimeout(() => {
        timer = null;
        next();
      }, wait);
      return;
    }
    lastStart = Date.now();
    // Newest first: what was just scrolled to is what is on screen.
    queue.pop()!.run();
  }
}

function forget(key: string) {
  const i = queue.findIndex((q) => q.key === key);
  if (i >= 0) queue.splice(i, 1);
}

function trim() {
  if (entries.size <= KEEP) return;
  for (const [key, entry] of entries) {
    if (entries.size <= KEEP) break;
    if (entry.waiters > 0 || !entry.started) continue;
    entries.delete(key);
    void entry.promise.then((url) => url?.startsWith("blob:") && URL.revokeObjectURL(url));
  }
}

async function fetchImage(src: string, token: string | null): Promise<string | null> {
  // A cover kept on the device for offline listening needs no request at all.
  const stored = await offlineCoverUrl(src);
  if (stored) return stored;
  const res = await fetch(mediaUrl(src) as string, { headers: token ? { Authorization: `Bearer ${token}` } : undefined });
  if (!res.ok) return null;
  // A 200 is not proof of an image: an SPA fallback answers every path with
  // index.html, and that would be an <img> that silently fails to decode.
  if (!(res.headers.get("content-type") ?? "").startsWith("image/")) return null;
  return URL.createObjectURL(await res.blob());
}

/** An object URL for `src`, or null when it is not an image. Call the returned
 *  `release` when the image is no longer wanted. */
export function loadAuthImage(src: string, token: string | null): { url: Promise<string | null>; release: () => void } {
  const key = `${token ?? ""} ${src}`;
  let entry = entries.get(key);
  if (!entry) {
    let resolve!: (v: string | null) => void;
    const promise = new Promise<string | null>((r) => (resolve = r));
    const created: Entry = {
      promise,
      waiters: 0,
      started: false,
      drop: () => {
        forget(key);
        entries.delete(key);
        resolve(null);
      },
    };
    entry = created;
    entries.set(key, created);
    queue.push({
      key,
      run: () => {
        created.started = true;
        running++;
        fetchImage(src, token)
          .catch(() => null)
          .then((url) => {
            // A failure is not kept: the next time it is shown, it is asked for again.
            if (!url) entries.delete(key);
            resolve(url);
          })
          .finally(() => {
            running--;
            next();
            trim();
          });
      },
    });
    next();
  }
  const held = entry;
  held.waiters++;
  return {
    url: held.promise,
    release: () => {
      held.waiters--;
      if (held.waiters === 0 && !held.started) held.drop();
    },
  };
}
