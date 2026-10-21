// SPDX-License-Identifier: AGPL-3.0-or-later
// The console against a large library, in a real browser: how long pages take,
// what they download, what fails, and whether the browser stalls. Read-only
// apart from playing a few songs.
//
//   BASE_URL=http://nas:8099 UI_TEST_EMAIL=… UI_TEST_PASSWORD=… \
//     node e2e/library-scale.mjs [out-dir]
//
// Writes report.json and report.md to out-dir (default ./e2e-report).
import { chromium } from "playwright";
import AxeBuilder from "@axe-core/playwright";
import { mkdirSync, writeFileSync } from "node:fs";

const BASE = (process.env.BASE_URL ?? "http://localhost:8080").replace(/\/$/, "");
const OUT = process.argv[2] ?? "e2e-report";
mkdirSync(OUT, { recursive: true });

const report = { base: BASE, started: new Date().toISOString(), library: {}, pages: [], flows: [], problems: [], axe: [] };
const problem = (where, what) => report.problems.push({ where, what: String(what).slice(0, 500) });

async function api(path, token, init = {}) {
  const res = await fetch(`${BASE}/api/v1${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}), ...init.headers },
  });
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status}`);
  return res.json();
}

const login = await api("/auth/login", null, {
  method: "POST",
  body: JSON.stringify({ email: process.env.UI_TEST_EMAIL, password: process.env.UI_TEST_PASSWORD }),
});
const tracks = await api("/music/tracks", login.token);
const albums = await api("/music/albums", login.token);
report.library = { tracks: tracks.length, albums: albums.length };

const browser = await chromium.launch();
const context = await browser.newContext({ locale: "en-US", viewport: { width: 1440, height: 900 } });
// The console keeps its session in localStorage (store/authStore.ts).
await context.addInitScript(
  ([state]) => {
    localStorage.setItem("audio2-auth", JSON.stringify({ state, version: 0 }));
    window.__longTasks = [];
    try {
      new PerformanceObserver((list) => {
        for (const e of list.getEntries()) window.__longTasks.push({ start: Math.round(e.startTime), ms: Math.round(e.duration) });
      }).observe({ type: "longtask", buffered: true });
    } catch {}
  },
  [{ token: login.token, refreshToken: login.refresh_token, user: login.user }],
);
let page;
let cdp;
let requests = [];
// A fresh tab for every check: a tab that crashed (a huge page can) must not
// take the checks after it down too.
async function freshPage() {
  if (page && !page.isClosed()) await page.close().catch(() => {});
  page = await context.newPage();
  cdp = await context.newCDPSession(page);
  await cdp.send("Performance.enable");
  page.on("crash", () => problem(page.url().slice(BASE.length), "the tab crashed"));
  page.on("response", onResponse);
  page.on("console", (m) => m.type() === "error" && problem(page.url().slice(BASE.length), `console: ${m.text()}`));
  page.on("pageerror", (e) => problem(page.url().slice(BASE.length), `page error: ${e.message}`));
}
async function onResponse(res) {
  const req = res.request();
  const url = req.url();
  if (!url.startsWith(BASE)) return;
  const timing = req.timing();
  let bytes = Number(res.headers()["content-length"] ?? 0);
  // Only JSON is read to count it: an audio stream has no end to wait for.
  if (!bytes && /json/.test(res.headers()["content-type"] ?? "")) bytes = (await res.body().catch(() => Buffer.alloc(0))).length;
  requests.push({ url: url.slice(BASE.length), status: res.status(), bytes, ms: Math.round(timing.responseEnd) });
  if (res.status() >= 400 && !/\/avatar(\?|$)/.test(url)) problem(page.url().slice(BASE.length), `HTTP ${res.status()} ${req.method()} ${url.slice(BASE.length)}`);
}
await freshPage();

const heapMb = async () => {
  const { metrics } = await cdp.send("Performance.getMetrics");
  return Math.round((metrics.find((m) => m.name === "JSHeapUsedSize")?.value ?? 0) / 1048576);
};
const longTasks = () => page.evaluate(() => window.__longTasks.splice(0));
const settle = () => page.waitForLoadState("networkidle", { timeout: 60_000 }).catch(() => problem(page.url(), "network not idle after 60 s"));

async function measure(name, path) {
  requests = [];
  const t0 = Date.now();
  const status = (await page.goto(BASE + path, { waitUntil: "domcontentloaded" }))?.status();
  await settle();
  const ms = Date.now() - t0;
  const api = requests.filter((r) => r.url.startsWith("/api/"));
  const biggest = [...api].sort((a, b) => b.bytes - a.bytes)[0];
  const slowest = [...api].sort((a, b) => b.ms - a.ms)[0];
  const tasks = await longTasks();
  const row = {
    name, path, status, ms,
    apiCalls: api.length,
    apiKb: Math.round(api.reduce((s, r) => s + r.bytes, 0) / 1024),
    biggest: biggest && `${biggest.url.split("?")[0]} ${Math.round(biggest.bytes / 1024)} KB`,
    slowest: slowest && `${slowest.url.split("?")[0]} ${slowest.ms} ms`,
    longTaskMs: tasks.reduce((s, t) => s + t.ms, 0),
    longestTaskMs: Math.max(0, ...tasks.map((t) => t.ms)),
    heapMb: await heapMb(),
  };
  report.pages.push(row);
  console.log(`${name.padEnd(22)} ${String(ms).padStart(6)} ms  ${row.apiKb} KB  long tasks ${row.longTaskMs} ms`);
  return row;
}

async function flow(name, fn) {
  await freshPage();
  const t0 = Date.now();
  try {
    const detail = await fn();
    report.flows.push({ name, ok: true, ms: Date.now() - t0, ...detail });
    console.log(`flow ${name}: ok ${Date.now() - t0} ms`, detail ?? "");
  } catch (e) {
    report.flows.push({ name, ok: false, ms: Date.now() - t0, error: String(e.message ?? e).slice(0, 300) });
    console.log(`flow ${name}: FAILED`, e.message);
  }
}

// ── Pages, each loaded fresh ────────────────────────────────────────────────
const firstAlbum = albums.find((a) => a.track_count >= 8) ?? albums[0];
const albumPath = `/music/albums/${encodeURIComponent(`${firstAlbum.artist}␟${firstAlbum.album}${firstAlbum.id ? `␟${firstAlbum.id}` : ""}`)}`;
const PAGES = [
  ["Home", "/"],
  ["Music: songs", "/music"],
  ["Music: albums", "/music/albums"],
  ["Music: one album", albumPath],
  ["Music: playlists", "/music/playlists"],
  ["Music: duplicates", "/music/duplicates"],
  ["Music: identify", "/music/identify"],
  ["Organize", "/music/organize"],
  ["Audiobooks", "/audiobooks"],
  ["Podcasts", "/podcasts"],
  ["Stats", "/stats"],
  ["Family", "/family"],
  ["Just me", "/private"],
  ["Trash", "/trash"],
  ["Settings", "/settings"],
  ["Playback settings", "/settings/playback"],
];
for (const [name, path] of PAGES) {
  try {
    await measure(name, path);
  } catch (e) {
    problem(path, `${name}: ${e.message}`);
    await freshPage();
  }
}

// ── Accessibility, on the heaviest pages ─────────────────────────────────────
for (const path of ["/", "/music", "/music/albums", albumPath, "/music/duplicates", "/music/organize", "/audiobooks", "/family", "/settings", "/settings/playback", "/stats"]) {
  try {
    await freshPage();
    await page.goto(BASE + path);
    await settle();
    const res = await new AxeBuilder({ page }).withTags(["wcag2a", "wcag2aa"]).analyze();
    report.axe.push({ path, violations: res.violations.map((v) => ({ id: v.id, impact: v.impact, nodes: v.nodes.length, help: v.help })) });
  } catch (e) {
    problem(path, `axe: ${e.message}`);
  }
}

// ── Flows ───────────────────────────────────────────────────────────────────
await flow("scroll the whole song list", async () => {
  await page.goto(BASE + "/music");
  await settle();
  const before = await heapMb();
  await longTasks();
  const t0 = Date.now();
  let steps = 0;
  for (; steps < 400; steps++) {
    const done = await page.evaluate(() => {
      const els = [...document.querySelectorAll("*")].filter((e) => e.scrollHeight > e.clientHeight + 200 && getComputedStyle(e).overflowY !== "visible");
      const el = els.sort((a, b) => b.scrollHeight - a.scrollHeight)[0] ?? document.scrollingElement;
      el.scrollTop += el.clientHeight;
      return el.scrollTop + el.clientHeight >= el.scrollHeight - 2;
    });
    await page.waitForTimeout(30);
    // The list grows as its end comes near: at the end, give the next part a moment.
    if (done) {
      const grew = await page.waitForFunction(() => document.querySelectorAll("main .group").length > window.__rows, null, { timeout: 1500 }).then(() => true, () => false);
      if (!grew) break;
    }
    await page.evaluate(() => (window.__rows = document.querySelectorAll("main .group").length));
  }
  const tasks = await longTasks();
  return {
    screens: steps, scrollMs: Date.now() - t0, heapBeforeMb: before, heapAfterMb: await heapMb(),
    rowsInDom: await page.evaluate(() => document.querySelectorAll("main .group").length),
    longTaskMs: tasks.reduce((s, t) => s + t.ms, 0),
  };
});

await flow("play an album, skip, open Settings at once", async () => {
  await page.goto(BASE + albumPath);
  await settle();
  await page.getByRole("button", { name: /^Play$/ }).first().click();
  await page.waitForFunction(() => [...document.querySelectorAll("audio")].some((a) => a.currentTime > 0.5), null, { timeout: 30_000 });
  await longTasks();
  const next = page.getByRole("button", { name: /^Next$/ }).first();
  const t0 = Date.now();
  await next.click();
  await page.locator("aside button[aria-label]").last().click();
  await page.getByRole("menuitem", { name: /^Settings$/ }).click();
  await page.getByRole("heading", { name: /^Settings$/ }).first().waitFor({ timeout: 30_000 });
  const settingsMs = Date.now() - t0;
  await page.waitForFunction(() => [...document.querySelectorAll("audio")].some((a) => !a.paused && a.currentTime > 0.5), null, { timeout: 30_000 });
  const tasks = await longTasks();
  return { settingsShownAfterMs: settingsMs, nextSongPlayingAfterMs: Date.now() - t0, longTaskMs: tasks.reduce((s, t) => s + t.ms, 0), longestTaskMs: Math.max(0, ...tasks.map((t) => t.ms)) };
});

await flow("WMA song plays in the browser", async () => {
  // The track list does not say a song's format; WMA_TRACK_ID names one.
  const wma = tracks.find((t) => t.id === process.env.WMA_TRACK_ID);
  if (!wma) return { skipped: "set WMA_TRACK_ID to a WMA song's id" };
  await page.goto(BASE + "/music");
  await settle();
  const res = await page.evaluate(async ([base, token, id]) => {
    const r = await fetch(`${base}/api/v2/playback-info`, {
      method: "POST",
      headers: { "content-type": "application/json", authorization: `Bearer ${token}` },
      body: JSON.stringify({ kind: "track", id, capabilities: { direct: ["audio/mpeg"], transcode: ["mp3"] } }),
    });
    const info = await r.json();
    const audio = new Audio(info.url);
    audio.muted = true;
    await audio.play();
    await new Promise((ok) => setTimeout(ok, 3000));
    return { decision: info.decision, playedSecs: audio.currentTime };
  }, [BASE, login.token, wma.id]);
  if (!(res.playedSecs > 0.5)) throw new Error(`did not play: ${JSON.stringify(res)}`);
  return { title: wma.title, ...res };
});

await flow("albums with the same name are told apart", async () => {
  const seen = new Map();
  const pairs = [];
  for (const a of albums) {
    const k = `${a.artist}␟${a.album}`;
    if (seen.has(k)) pairs.push([seen.get(k), a]);
    else seen.set(k, a);
  }
  if (!pairs.length) return { pairs: 0 };
  const [a] = pairs[0];
  await page.goto(BASE + "/music/albums");
  await settle();
  const path = `/music/albums/${encodeURIComponent(`${a.artist}␟${a.album}${a.id ? `␟${a.id}` : ""}`)}`;
  await page.goto(BASE + path);
  await settle();
  // The album's column lists its own songs, not those of its namesake.
  // The album's own column is the last one; the first lists the library.
  const listed = await page.locator("section").last().locator('[role="listitem"]').count();
  if (listed !== a.track_count) throw new Error(`${a.artist} — ${a.album}: ${listed} songs listed, the album has ${a.track_count}`);
  return { pairs: pairs.length, example: `${a.artist} — ${a.album}`, songsListed: listed };
});

await flow("every console address answers 200", async () => {
  const bad = [];
  for (const [, path] of PAGES) {
    const res = await fetch(BASE + path);
    if (res.status !== 200) bad.push(`${path} ${res.status}`);
  }
  if (bad.length) throw new Error(bad.join(", "));
  return { checked: PAGES.length };
});

await flow("a WMA album plays in the console's own player", async () => {
  const wma = tracks.find((t) => t.id === process.env.WMA_TRACK_ID);
  if (!wma) return { skipped: "set WMA_TRACK_ID" };
  const album = albums.find((a) => wma.album_id ? a.id === wma.album_id : a.album === wma.album);
  if (!album) throw new Error(`no album for ${wma.title}`);
  await page.goto(BASE + `/music/albums/${encodeURIComponent(`${album.artist}\u241F${album.album}${album.id ? `\u241F${album.id}` : ""}`)}`);
  await settle();
  const played = [];
  page.on("request", (r) => /playback-info|\/stream\b|\/media\//.test(r.url()) && played.push(r.url().slice(BASE.length, BASE.length + 40)));
  await page.getByRole("button", { name: /^Play$/ }).first().click();
  await page.waitForFunction(() => [...document.querySelectorAll("audio")].some((a) => !a.paused && a.currentTime > 1), null, { timeout: 30_000 });
  const info = await page.evaluate(() => {
    const a = [...document.querySelectorAll("audio")].find((x) => !x.paused);
    return { secs: Math.round(a.currentTime * 10) / 10, duration: Math.round(a.duration) };
  });
  return { album: `${album.artist} — ${album.album}`, ...info, requests: played.slice(0, 3) };
});

await flow("a profile picture shows after uploading it", async () => {
  await page.goto(BASE + "/settings");
  await settle();
  const png = Buffer.from(
    "iVBORw0KGgoAAAANSUhEUgAAABAAAAAQCAIAAACQkWg2AAAAFklEQVR4nGM4YRNFEmIY1TCqYfhqAACAeF4QeTHioQAAAABJRU5ErkJggg==",
    "base64",
  );
  await page.locator('input[type="file"][accept="image/*"]').first().setInputFiles({ name: "ui-test.png", mimeType: "image/png", buffer: png });
  const preview = await page.locator('img[src^="blob:"]').count();
  await page.getByRole("button", { name: /^Save$/ }).first().click();
  await page.waitForTimeout(2500);
  await page.reload();
  await settle();
  const shown = await page.locator('aside img[src^="blob:"], main img[src^="blob:"]').count();
  if (!preview || !shown) throw new Error(`preview ${preview}, shown after reload ${shown}`);
  return { previewShown: preview > 0, picturesAfterReload: shown };
});

await flow("songs on screen: first visit, then reopened", async () => {
  const firstRow = async () => {
    const t0 = Date.now();
    await page.goto(BASE + "/music", { waitUntil: "commit" });
    await page.locator('main [role="listitem"]').first().waitFor({ timeout: 30_000 });
    return Date.now() - t0;
  };
  // A real first visit: no copy of the library on the device yet.
  await page.goto(BASE + "/settings");
  await page.evaluate(() => new Promise((ok) => {
    const req = indexedDB.open("own-audio-offline");
    req.onsuccess = () => {
      const tx = req.result.transaction("meta", "readwrite");
      tx.objectStore("meta").delete(IDBKeyRange.bound("library:", "library:\uffff"));
      tx.oncomplete = () => { req.result.close(); ok(); };
    };
    req.onerror = () => ok();
  }));
  const first = await firstRow();
  // The device's copy is written a moment after the answer arrives.
  await page.waitForTimeout(3000);
  const again = await firstRow();
  return { firstVisitMs: first, reopenedMs: again };
});

for (const [label, pick] of [
  ["a book of several files plays", (files) => files.length > 1],
  // The file list does not name the format; BOOK_M4B names a one-file M4B book by title.
  ["a one-file M4B book plays", (files, b) => files.length === 1 && b.title.includes(process.env.BOOK_M4B ?? "\u0000")],
]) {
  await flow(label, async () => {
    const books = await api("/audiobooks", login.token);
    if (!books.length) return { skipped: "no audiobooks visible to this account" };
    let book = null;
    for (const b of books.slice(0, 60)) {
      const files = await api(`/audiobooks/${b.id}/files`, login.token);
      if (pick(files, b)) { book = { ...b, files: files.length }; break; }
    }
    if (!book) return { skipped: "no such book" };
    await page.goto(BASE + `/audiobooks/${book.id}`);
    await settle();
    await page.getByRole("button", { name: /^(Play|Resume|Continue)/ }).first().click();
    await page.waitForFunction(() => [...document.querySelectorAll("audio")].some((a) => !a.paused && a.currentTime > 1), null, { timeout: 30_000 });
    const at = await page.evaluate(() => Math.round([...document.querySelectorAll("audio")].find((a) => !a.paused).currentTime));
    return { book: `${book.author} — ${book.title}`.slice(0, 80), files: book.files, playingAtSecs: at };
  });
}

await flow("search", async () => {
  await page.goto(BASE + "/music");
  await settle();
  const box = page.getByPlaceholder(/Filter|Search/).first();
  const t0 = Date.now();
  await box.fill("love");
  await page.waitForTimeout(1500);
  return { typedToSettledMs: Date.now() - t0, longTaskMs: (await longTasks()).reduce((s, t) => s + t.ms, 0) };
});

await page.screenshot({ path: `${OUT}/last.png` }).catch(() => {});
await browser.close();
report.finished = new Date().toISOString();
writeFileSync(`${OUT}/report.json`, JSON.stringify(report, null, 2));

const md = [
  `# Console against ${report.library.tracks} songs, ${report.library.albums} albums`,
  "",
  `${BASE}, ${report.started}`,
  "",
  "| Page | Load | API calls | API KB | Biggest | Slowest | Long tasks | Heap MB |",
  "|---|---|---|---|---|---|---|---|",
  ...report.pages.map((p) => `| ${p.name} | ${p.ms} ms | ${p.apiCalls} | ${p.apiKb} | ${p.biggest ?? ""} | ${p.slowest ?? ""} | ${p.longTaskMs} ms (max ${p.longestTaskMs}) | ${p.heapMb} |`),
  "",
  "## Flows",
  "",
  ...report.flows.map((f) => `- **${f.name}**: ${f.ok ? "ok" : "FAILED"} — ${JSON.stringify({ ...f, name: undefined, ok: undefined })}`),
  "",
  "## Accessibility (axe, WCAG 2 A/AA)",
  "",
  ...report.axe.map((a) => `- ${a.path}: ${a.violations.length ? a.violations.map((v) => `${v.id} (${v.impact}, ${v.nodes})`).join(", ") : "none"}`),
  "",
  `## Problems (${report.problems.length})`,
  "",
  ...[...new Set(report.problems.map((p) => `${p.what} — on ${p.where}`))].map((p) => `- ${p}`),
];
writeFileSync(`${OUT}/report.md`, md.join("\n") + "\n");
console.log(`\nreport: ${OUT}/report.md`);
