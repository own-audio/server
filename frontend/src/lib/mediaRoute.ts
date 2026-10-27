// SPDX-License-Identifier: AGPL-3.0-or-later

/* Some networks (company firewalls, mostly) block the object store's host, so
   a presigned link fails while the rest of the console works. A server with a
   media proxy offers the same audio by its own host: `fallback_url` on v1
   stream answers, `route=proxy` on v2 stream links (API_COMPATIBILITY §6).
   Once the proxy has worked here, the rest of the session goes straight to it
   rather than failing once per track. */

const KEY = "own-audio.media-route";

/** Fallbacks the server named, by the primary link they stand in for. */
const known = new Map<string, string>();

/** Storage that may be missing or throw (private windows, blocked site data). */
export interface RouteStore {
  getItem(key: string): string | null;
  setItem(key: string, value: string): void;
}

function sessionStore(): RouteStore | null {
  try {
    return typeof sessionStorage === "undefined" ? null : sessionStorage;
  } catch {
    return null;
  }
}

/** The proxy route for a v2 stream link to the original; null for anything else.
 *  A transcode is streamed by the server itself, and `route` changes nothing there. */
export function proxyRouteOf(url: string): string | null {
  const q = url.indexOf("?");
  if (q < 0 || !url.slice(0, q).endsWith("/api/v2/stream")) return null;
  const params = new URLSearchParams(url.slice(q + 1));
  if (params.get("f") !== "orig" || params.has("route")) return null;
  return `${url}&route=proxy`;
}

/** Records the server's fallback for a link, and returns the one to use now. */
export function streamUrlFrom(url: string, fallbackUrl?: string | null, store = sessionStore()): string {
  if (fallbackUrl) known.set(url, fallbackUrl);
  return routed(url, store);
}

/** The other way to the same audio, if there is one. */
export function fallbackFor(url: string): string | null {
  return known.get(url) ?? proxyRouteOf(url);
}

export function prefersFallback(store = sessionStore()): boolean {
  try {
    return store?.getItem(KEY) === "proxy";
  } catch {
    return false;
  }
}

export function rememberFallback(store = sessionStore()): void {
  try {
    store?.setItem(KEY, "proxy");
  } catch {
    /* storage may be blocked: the next track fails over again, nothing worse */
  }
}

/** The link to load: the fallback when this session already needed it. */
export function routed(url: string, store = sessionStore()): string {
  if (!prefersFallback(store)) return url;
  return fallbackFor(url) ?? url;
}

/** The fallback to try after `url` failed to load, or null when there is none:
 *  a downloaded copy, a link that is already the fallback, a server without a proxy. */
export function nextAfterFailure(url: string): string | null {
  if (url.startsWith("blob:")) return null;
  return fallbackFor(url);
}

/** The player's next move after a media error on `src`: the other host first
 *  (once per track), then a freshly signed link (once), then give up. The
 *  fresh link already comes routed, so this never loops. */
export type ErrorStep = { kind: "fallback"; url: string } | { kind: "refresh" } | { kind: "fail" };

export function nextStep(src: string | null, fallbackTried: boolean, refreshed: boolean): ErrorStep {
  const url = !fallbackTried && src ? nextAfterFailure(src) : null;
  if (url) return { kind: "fallback", url };
  return refreshed ? { kind: "fail" } : { kind: "refresh" };
}

/** The parts of a media element a source switch touches. */
export interface SwitchableMedia {
  src: string;
  currentTime: number;
  load(): void;
  play(): Promise<void>;
  addEventListener(type: "canplay", listener: () => void, options: { once: true }): void;
}

/** Loads `url` and carries on from where the element was. A failure before
 *  anything played leaves the position to whoever set up the first load (a
 *  resume point), which is why 0 is not restored. */
export function switchSource(media: SwitchableMedia, url: string, onPlaying?: () => void): void {
  const at = media.currentTime;
  media.src = url;
  media.load();
  media.addEventListener(
    "canplay",
    () => {
      if (at > 0) media.currentTime = at;
      onPlaying?.();
      void media.play().catch(() => {});
    },
    { once: true }
  );
}

/** A fetch of stored audio that takes the fallback once if the first host
 *  cannot be reached or answers with a block page. */
export async function fetchWithFallback(url: string, init?: RequestInit): Promise<Response> {
  let res: Response | null = null;
  let error: unknown = null;
  try {
    res = await fetch(url, init);
    if (res.ok) return res;
  } catch (e) {
    if (init?.signal?.aborted) throw e;
    error = e;
  }
  const next = nextAfterFailure(url);
  if (!next) {
    if (res) return res;
    throw error;
  }
  const retry = await fetch(next, init);
  if (retry.ok) rememberFallback();
  return retry;
}

/** For tests. */
export function forgetFallbacks(): void {
  known.clear();
}
