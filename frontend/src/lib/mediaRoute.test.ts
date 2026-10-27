// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  fallbackFor,
  fetchWithFallback,
  forgetFallbacks,
  nextAfterFailure,
  nextStep,
  prefersFallback,
  proxyRouteOf,
  rememberFallback,
  routed,
  streamUrlFrom,
  switchSource,
  type RouteStore,
  type SwitchableMedia,
} from "./mediaRoute";

const R2 = "https://bucket.r2.cloudflarestorage.com/a/b.mp3?X-Amz-Signature=x";
const PROXY = "https://media.example.org/a/b.mp3?e=1&s=y";
const V2_ORIG = "https://own.example.org/api/v2/stream?i=track:1&f=orig&b=0&o=0&u=2&e=3&k=1&s=sig";
const V2_MP3 = "https://own.example.org/api/v2/stream?i=track:1&f=mp3&b=128&o=0&u=2&e=3&k=1&s=sig";

function memoryStore(): RouteStore & { data: Map<string, string> } {
  const data = new Map<string, string>();
  return { data, getItem: (k) => data.get(k) ?? null, setItem: (k, v) => void data.set(k, v) };
}

const throwingStore: RouteStore = {
  getItem: () => {
    throw new Error("blocked");
  },
  setItem: () => {
    throw new Error("blocked");
  },
};

beforeEach(() => forgetFallbacks());

describe("proxyRouteOf", () => {
  it("adds route=proxy to a v2 link to the original", () => {
    expect(proxyRouteOf(V2_ORIG)).toBe(`${V2_ORIG}&route=proxy`);
  });

  it("works on a link relative to the console", () => {
    expect(proxyRouteOf("/api/v2/stream?i=x&f=orig&s=1")).toBe("/api/v2/stream?i=x&f=orig&s=1&route=proxy");
  });

  it("leaves a transcode alone: the server streams it itself", () => {
    expect(proxyRouteOf(V2_MP3)).toBeNull();
  });

  it("leaves a link that already names a route alone", () => {
    expect(proxyRouteOf(`${V2_ORIG}&route=proxy`)).toBeNull();
  });

  it("knows nothing about other links", () => {
    expect(proxyRouteOf(R2)).toBeNull();
    expect(proxyRouteOf("blob:https://own.example.org/123")).toBeNull();
  });
});

describe("the session's choice", () => {
  it("uses the store's link until the fallback was needed", () => {
    const store = memoryStore();
    expect(streamUrlFrom(R2, PROXY, store)).toBe(R2);
    expect(prefersFallback(store)).toBe(false);
    rememberFallback(store);
    expect(prefersFallback(store)).toBe(true);
    expect(streamUrlFrom(R2, PROXY, store)).toBe(PROXY);
    expect(routed(V2_ORIG, store)).toBe(`${V2_ORIG}&route=proxy`);
  });

  it("keeps the store's link when there is no fallback, even after choosing it", () => {
    const store = memoryStore();
    rememberFallback(store);
    expect(streamUrlFrom(R2, undefined, store)).toBe(R2);
    expect(routed(V2_MP3, store)).toBe(V2_MP3);
  });

  it("survives storage that throws", () => {
    expect(() => rememberFallback(throwingStore)).not.toThrow();
    expect(prefersFallback(throwingStore)).toBe(false);
    expect(streamUrlFrom(R2, PROXY, throwingStore)).toBe(R2);
  });

  it("survives no storage at all", () => {
    expect(() => rememberFallback(null)).not.toThrow();
    expect(prefersFallback(null)).toBe(false);
  });
});

describe("nextAfterFailure", () => {
  it("names the server's fallback for a link it gave one for", () => {
    streamUrlFrom(R2, PROXY, memoryStore());
    expect(fallbackFor(R2)).toBe(PROXY);
    expect(nextAfterFailure(R2)).toBe(PROXY);
  });

  it("has nothing after the fallback itself failed", () => {
    streamUrlFrom(R2, PROXY, memoryStore());
    expect(nextAfterFailure(PROXY)).toBeNull();
    expect(nextAfterFailure(`${V2_ORIG}&route=proxy`)).toBeNull();
  });

  it("has nothing for a downloaded copy or a server without a proxy", () => {
    expect(nextAfterFailure("blob:https://own.example.org/123")).toBeNull();
    expect(nextAfterFailure(R2)).toBeNull();
  });
});

describe("nextStep (the player after a media error)", () => {
  it("tries the other host first, then a fresh link, then gives up", () => {
    streamUrlFrom(R2, PROXY, memoryStore());
    expect(nextStep(R2, false, false)).toEqual({ kind: "fallback", url: PROXY });
    expect(nextStep(PROXY, true, false)).toEqual({ kind: "refresh" });
    expect(nextStep(R2, true, true)).toEqual({ kind: "fail" });
  });

  it("never offers the fallback twice for one track", () => {
    streamUrlFrom(R2, PROXY, memoryStore());
    // A fresh link after the fallback failed is the store's again; trying the proxy once more would loop.
    expect(nextStep(R2, true, true)).toEqual({ kind: "fail" });
  });

  it("refreshes as before when there is no fallback", () => {
    expect(nextStep(R2, false, false)).toEqual({ kind: "refresh" });
    expect(nextStep(null, false, false)).toEqual({ kind: "refresh" });
  });
});

describe("switchSource", () => {
  function fakeMedia(currentTime: number) {
    let onCanPlay: (() => void) | null = null;
    const media: SwitchableMedia & { loads: number; plays: number; canplay(): void } = {
      src: R2,
      currentTime,
      loads: 0,
      plays: 0,
      load() {
        this.loads++;
        this.currentTime = 0;
      },
      play() {
        this.plays++;
        return Promise.resolve();
      },
      addEventListener(_type, listener) {
        onCanPlay = listener;
      },
      canplay() {
        onCanPlay?.();
      },
    };
    return media;
  }

  it("loads the new link and keeps playing at the same position", () => {
    const media = fakeMedia(73.5);
    const playing = vi.fn();
    switchSource(media, PROXY, playing);
    expect(media.src).toBe(PROXY);
    expect(media.loads).toBe(1);
    expect(playing).not.toHaveBeenCalled();
    media.canplay();
    expect(media.currentTime).toBe(73.5);
    expect(media.plays).toBe(1);
    expect(playing).toHaveBeenCalledOnce();
  });

  it("leaves the position alone when nothing had played yet (the resume point is set elsewhere)", () => {
    const media = fakeMedia(0);
    switchSource(media, PROXY);
    media.currentTime = 1200; // the first load's own canplay handler resuming
    media.canplay();
    expect(media.currentTime).toBe(1200);
  });
});

describe("fetchWithFallback", () => {
  let session: ReturnType<typeof memoryStore>;
  beforeEach(() => {
    session = memoryStore();
    vi.stubGlobal("sessionStorage", session);
  });
  afterEach(() => vi.unstubAllGlobals());

  it("returns the first answer when it works", async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response("ok"));
    vi.stubGlobal("fetch", fetchMock);
    streamUrlFrom(R2, PROXY);
    const res = await fetchWithFallback(R2);
    expect(res.ok).toBe(true);
    expect(fetchMock).toHaveBeenCalledTimes(1);
    expect(prefersFallback()).toBe(false);
  });

  it("takes the fallback when the store's host is unreachable, and remembers it", async () => {
    const fetchMock = vi.fn().mockRejectedValueOnce(new TypeError("Failed to fetch")).mockResolvedValueOnce(new Response("ok"));
    vi.stubGlobal("fetch", fetchMock);
    streamUrlFrom(R2, PROXY);
    const res = await fetchWithFallback(R2);
    expect(res.ok).toBe(true);
    expect(fetchMock.mock.calls.map((c) => c[0])).toEqual([R2, PROXY]);
    expect(prefersFallback()).toBe(true);
  });

  it("takes the fallback after a block page", async () => {
    const fetchMock = vi.fn().mockResolvedValueOnce(new Response("blocked", { status: 403 })).mockResolvedValueOnce(new Response("ok"));
    vi.stubGlobal("fetch", fetchMock);
    const res = await fetchWithFallback(V2_ORIG);
    expect(res.ok).toBe(true);
    expect(fetchMock.mock.calls[1][0]).toBe(`${V2_ORIG}&route=proxy`);
  });

  it("tries the fallback once and hands back its failure", async () => {
    const fetchMock = vi.fn().mockRejectedValueOnce(new TypeError("Failed to fetch")).mockResolvedValueOnce(new Response("no", { status: 502 }));
    vi.stubGlobal("fetch", fetchMock);
    streamUrlFrom(R2, PROXY);
    const res = await fetchWithFallback(R2);
    expect(res.status).toBe(502);
    expect(fetchMock).toHaveBeenCalledTimes(2);
    expect(prefersFallback()).toBe(false);
  });

  it("rethrows when there is no fallback", async () => {
    vi.stubGlobal("fetch", vi.fn().mockRejectedValue(new TypeError("Failed to fetch")));
    await expect(fetchWithFallback(R2)).rejects.toThrow("Failed to fetch");
  });

  it("does not retry a download that was cancelled", async () => {
    const controller = new AbortController();
    const fetchMock = vi.fn().mockImplementation(() => {
      controller.abort();
      return Promise.reject(new DOMException("aborted", "AbortError"));
    });
    vi.stubGlobal("fetch", fetchMock);
    streamUrlFrom(R2, PROXY);
    await expect(fetchWithFallback(R2, { signal: controller.signal })).rejects.toThrow("aborted");
    expect(fetchMock).toHaveBeenCalledTimes(1);
  });
});
