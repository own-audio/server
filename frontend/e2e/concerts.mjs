// SPDX-License-Identifier: AGPL-3.0-or-later
// Concerts in the console, in a real browser (docs/concerts-plan.md C2): a whole concert as
// one track with a chapter per song. Uploads a generated concert (ffmpeg tones, four marked
// songs) to the account, then: the Concerts tab lists it, the player names the song playing,
// next goes to the next song, the songs list jumps to one, the owner renames one, and the
// concert opens where it was left after a reload. Leaves the concert in the library.
//
//   BASE_URL=http://localhost:8080 UI_TEST_EMAIL=… UI_TEST_PASSWORD=… node e2e/concerts.mjs [out-dir]
import { chromium } from "playwright";
import { execFileSync } from "node:child_process";
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = (process.env.BASE_URL ?? "http://localhost:8080").replace(/\/$/, "");
const OUT = process.argv[2] ?? "e2e-concerts";
mkdirSync(OUT, { recursive: true });
const failures = [];
const check = (what, ok, detail = "") => {
  console.log(`${ok ? "  ok  " : "  FAIL"} ${what}${ok || !detail ? "" : ` — ${detail}`}`);
  if (!ok) failures.push(what);
};

async function api(path, token, init = {}, version = "v1") {
  const res = await fetch(`${BASE}/api/${version}${path}`, {
    ...init,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}), ...init.headers },
  });
  if (!res.ok) throw new Error(`${path}: HTTP ${res.status} ${await res.text()}`);
  return res.status === 204 ? null : res.json();
}

// Four songs of 20 s each, marked as ffmpeg writes chapters.
function makeConcert() {
  const dir = mkdtempSync(join(tmpdir(), "concert-"));
  const marks = ["Opening", "Second Song", "Third Song", "Encore"];
  writeFileSync(join(dir, "meta.txt"), ";FFMETADATA1\n" + marks.map((t, i) => `[CHAPTER]\nTIMEBASE=1/1000\nSTART=${i * 20000}\nEND=${(i + 1) * 20000}\ntitle=${t}\n`).join(""));
  const file = join(dir, "concert.m4a");
  execFileSync("ffmpeg", ["-v", "error", "-f", "lavfi", "-i", "sine=frequency=330:sample_rate=44100", "-i", join(dir, "meta.txt"),
    "-map", "0:a", "-map_metadata", "1", "-map_chapters", "1", "-c:a", "aac", "-b:a", "64k", "-t", "80", file]);
  return readFileSync(file);
}

const login = await api("/auth/login", null, { method: "POST", body: JSON.stringify({ email: process.env.UI_TEST_EMAIL, password: process.env.UI_TEST_PASSWORD }) });
const token = login.token;
const sfx = Date.now().toString(36);
const title = `UI Test Concert ${sfx}`;
const audio = makeConcert();
const slot = await api("/uploads", token, { method: "POST", body: JSON.stringify({ kind: "track", filename: `${sfx}.m4a`, content_type: "audio/mp4", size_bytes: audio.length }) }, "v2");
const put = await fetch(slot.url, { method: "PUT", body: audio, headers: slot.headers });
if (!put.ok) throw new Error(`upload: HTTP ${put.status}`);
const track = await api("/tracks", token, {
  method: "POST",
  body: JSON.stringify({ upload_id: slot.upload_id, filename: `${sfx}.m4a`, title, artist: "Test Tone Band", album: title, album_artist: "Test Tone Band", release_kind: "live" }),
}, "v2");
check("the concert uploaded with its four songs", track.chapter_count === 4, JSON.stringify(track).slice(0, 200));

const browser = await chromium.launch();
const context = await browser.newContext({ locale: "en-US", viewport: { width: 1440, height: 900 } });
await context.addInitScript(([state]) => localStorage.setItem("audio2-auth", JSON.stringify({ state, version: 0 })),
  [{ token: login.token, refreshToken: login.refresh_token, user: login.user }]);
const page = await context.newPage();
const errors = [];
page.on("pageerror", (e) => errors.push(e.message));
const shot = (name) => page.screenshot({ path: join(OUT, `${name}.png`) });
// The mini player's title line.
const playerTitle = () => page.locator("span.block.truncate.text-sm.font-semibold").first().textContent();
const waitTitle = async (text, ms = 15_000) => {
  const end = Date.now() + ms;
  while (Date.now() < end) { if ((await playerTitle().catch(() => ""))?.trim() === text) return true; await page.waitForTimeout(250); }
  return false;
};

try {
  await page.goto(`${BASE}/music`);
  await page.getByRole("button", { name: /^Concerts$/ }).or(page.getByRole("radio", { name: /^Concerts$/ })).first().click({ timeout: 30_000 });
  const card = page.getByText(title, { exact: true }).first();
  check("the Concerts tab lists the concert", await card.waitFor({ timeout: 20_000 }).then(() => true, () => false));
  await shot("01-concerts");
  await card.click();
  await page.getByRole("button", { name: /^Play$/ }).first().click();
  check("the player names the song playing", await waitTitle("Opening"), await playerTitle());
  await shot("02-playing");

  await page.getByRole("button", { name: /^Next$/ }).first().click();
  check("next is the next song of the concert", await waitTitle("Second Song"), await playerTitle());

  await page.getByRole("button", { name: /^Songs of the concert$/ }).first().click();
  await page.getByRole("button", { name: /Encore/ }).first().click();
  check("the songs list jumps to a song", await waitTitle("Encore"), await playerTitle());
  await shot("03-songs");

  await page.getByRole("button", { name: /^Songs of the concert$/ }).first().click();
  await page.getByRole("button", { name: /^Edit$/ }).first().click();
  const names = page.getByRole("textbox", { name: /^Song$/ });
  await names.nth(3).fill("Big Encore");
  await page.getByRole("button", { name: /^Save$/ }).first().click();
  check("the owner renames a song", await waitTitle("Big Encore"), await playerTitle());
  const saved = await api(`/tracks/${track.id}`, token, {}, "v2");
  check("…and the server keeps it", saved.chapters?.[3]?.title === "Big Encore", JSON.stringify(saved.chapters));
  await shot("04-edited");

  // Back in the second song, paused, then a reload: the concert opens there.
  await page.getByRole("button", { name: /^Songs of the concert$/ }).first().click();
  await page.getByRole("button", { name: /Second Song/ }).first().click();
  await page.waitForTimeout(4_000);
  await page.getByRole("button", { name: /^Pause$/ }).first().click();
  await page.waitForTimeout(1_500);
  const progress = await api(`/music/tracks/${track.id}/progress`, token);
  check("the position is saved", progress.position_secs >= 20 && progress.position_secs < 40, JSON.stringify(progress));
  await page.reload();
  await page.getByRole("button", { name: /^Concerts$/ }).or(page.getByRole("radio", { name: /^Concerts$/ })).first().click({ timeout: 30_000 });
  await page.getByText(title, { exact: true }).first().click();
  await page.getByRole("button", { name: /^Play$/ }).first().click();
  check("after a reload the concert opens where it was left", await waitTitle("Second Song"), await playerTitle());
  await shot("05-resumed");
} catch (e) {
  check("the flow ran to the end", false, e.message);
  await shot("error").catch(() => {});
}
check("no errors on the page", errors.length === 0, errors.join("; "));
await browser.close();
console.log(failures.length ? `FAIL: ${failures.length} failed` : "PASS");
process.exit(failures.length ? 1 : 0);
