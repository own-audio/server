/*
 * own.audio service worker — just enough for the app to open with no
 * connection. Music is not cached here: downloads live in IndexedDB and play
 * as blobs (see src/lib/offline/), because Safari's audio element often
 * refuses a whole-file response served from a cache when it asked for a range.
 *
 * The cache version and file list below are filled in at build time
 * (vite.config.ts) from the hashed file names of that build, so every deploy is
 * a new worker with a new cache, and the old one is dropped on activate.
 */

const CACHE = "own-audio-shell-__VERSION__";
const PRECACHE = __PRECACHE__;
const SHELL = "/";
/** On a connection that is up but going nowhere, stop waiting and use the copy. */
const NAVIGATION_TIMEOUT_MS = 4000;

/* The host answers a path it doesn't have with the app's HTML page and 200 (the
   single-page fallback), so a build file asked for while a deploy is switching
   over can come back as HTML. Cached under the file's name, that page broke the
   screen that loads the file ("'text/html' is not a valid JavaScript MIME type")
   until the next deploy. Only real files are kept. */
function isBuildFile(url) {
  return new URL(url, self.location.origin).pathname.startsWith("/assets/");
}

function keepable(url, res) {
  if (!res.ok) return false;
  return !(isBuildFile(url) && (res.headers.get("content-type") ?? "").includes("text/html"));
}

self.addEventListener("install", (event) => {
  event.waitUntil(
    caches
      .open(CACHE)
      .then((cache) =>
        Promise.all(
          PRECACHE.map((path) =>
            fetch(path, { cache: "reload" }).then((res) => {
              if (!keepable(path, res)) throw new Error(`not precached: ${path}`);
              return cache.put(path, res);
            })
          )
        )
      )
      .then(() => self.skipWaiting())
  );
});

self.addEventListener("activate", (event) => {
  event.waitUntil(
    caches
      .keys()
      .then((keys) => Promise.all(keys.filter((k) => k.startsWith("own-audio-shell-") && k !== CACHE).map((k) => caches.delete(k))))
      .then(() => self.clients.claim())
  );
});

function withTimeout(promise, ms) {
  return new Promise((resolve, reject) => {
    const timer = setTimeout(() => reject(new Error("timeout")), ms);
    promise.then(
      (v) => {
        clearTimeout(timer);
        resolve(v);
      },
      (e) => {
        clearTimeout(timer);
        reject(e);
      }
    );
  });
}

self.addEventListener("fetch", (event) => {
  const req = event.request;
  if (req.method !== "GET") return;
  const url = new URL(req.url);
  // The API (same-origin when the backend serves this console) and anything
  // on another origin — storage, covers — are never touched.
  if (url.origin !== self.location.origin) return;
  if (url.pathname.startsWith("/api/") || url.pathname === "/health") return;

  if (req.mode === "navigate") {
    // Network first, so a deploy shows up on the next open; every route is the
    // same single-page shell, which is what falls back when that fails.
    // A playlist's page carries that playlist's manifest, so it is kept under
    // its own path rather than overwriting the app's shell.
    const key = url.pathname.startsWith("/play/") ? url.pathname : SHELL;
    event.respondWith(
      withTimeout(fetch(req), NAVIGATION_TIMEOUT_MS)
        .then((res) => {
          if (res.ok && (res.headers.get("content-type") ?? "").includes("text/html")) {
            const copy = res.clone();
            void caches.open(CACHE).then((cache) => cache.put(key, copy));
          }
          return res;
        })
        .catch(() =>
          caches
            .match(key)
            .then((hit) => hit ?? caches.match(SHELL))
            .then((hit) => hit ?? Response.error())
        )
    );
    return;
  }

  // Hashed build files never change under one name: cache first. A copy that is
  // the HTML fallback, kept by an earlier worker, is dropped and fetched again.
  event.respondWith(
    caches.match(req).then((hit) => {
      if (hit && keepable(req.url, hit)) return hit;
      if (hit) void caches.open(CACHE).then((cache) => cache.delete(req));
      // Past the browser's HTTP cache too: a CDN can give a `.js` path hours of it,
      // and the HTML fallback it once answered with would come back from there.
      return fetch(req, { cache: "no-cache" })
        .then((res) => (keepable(req.url, res) ? res : fetch(req, { cache: "reload" })))
        .then((res) => {
          if (url.pathname.startsWith("/assets/") && keepable(req.url, res)) {
            const copy = res.clone();
            void caches.open(CACHE).then((cache) => cache.put(req, copy));
          }
          return res;
        });
    })
  );
});
