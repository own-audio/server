// SPDX-License-Identifier: AGPL-3.0-or-later
// The console's hosted-only pages against the hosted edition, in a real (headless) browser:
// sign-up, the confirmation mail and the welcome credit, Storage & credit, narrating a book up
// to its quote and translating an episode up to its price. Nothing is bought and no job is
// started, so nothing paid runs. Every browser console error and 4xx/5xx answer is reported.
//
// Needs a fresh hosted server with open registration, mail to Mailpit, and a feed whose
// episodes publish transcripts and which the server can reach:
//
//   BASE_URL=http://localhost:18263 MAILPIT_URL=http://localhost:18265 \
//   FEED_URL=http://localhost:18257/feed-local.xml MEDIA_DIR=/path/with/songs \
//     node e2e/release-smoke-hosted.mjs [out-dir]
import { chromium } from "playwright";
import { mkdirSync, mkdtempSync, readdirSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = (process.env.BASE_URL ?? "http://localhost:8080").replace(/\/$/, "");
const MAILPIT = (process.env.MAILPIT_URL ?? "").replace(/\/$/, "");
const FEED_URL = process.env.FEED_URL ?? "";
const MEDIA = process.env.MEDIA_DIR ?? "media";
const OUT = process.argv[2] ?? "e2e-release-hosted";
mkdirSync(OUT, { recursive: true });

const USER = { name: "Hosted Tester", email: `hosted-${Date.now().toString(36)}@example.com`, password: "release-smoke-hosted-1" };
const songs = readdirSync(join(MEDIA, "songs")).filter((f) => f.endsWith(".mp3")).sort().map((f) => join(MEDIA, "songs", f));

const results = [];
const problems = [];
let current = "start";
let shotN = 0;
let responses = 0;

const browser = await chromium.launch();
const context = await browser.newContext({ locale: "en-US", viewport: { width: 1440, height: 900 } });
const page = await context.newPage();
page.on("pageerror", (e) => problems.push({ step: current, kind: "js", what: e.message }));
page.on("console", (m) => m.type() === "error" && problems.push({ step: current, kind: "console", what: m.text().slice(0, 400) }));
page.on("response", (r) => {
  responses += 1;
  if (r.status() >= 400) problems.push({ step: current, kind: "http", what: `${r.status()} ${r.request().method()} ${r.url().replace(BASE, "")}` });
});
page.on("requestfailed", (r) => {
  const why = r.failure()?.errorText ?? "";
  if (!/ERR_ABORTED/.test(why)) problems.push({ step: current, kind: "network", what: `${why} ${r.method()} ${r.url().replace(BASE, "")}` });
});

const shot = async (name) => {
  shotN += 1;
  await page.screenshot({ path: join(OUT, `${String(shotN).padStart(2, "0")}-${name}.png`) }).catch(() => {});
};
async function step(name, fn) {
  current = name;
  try {
    const detail = await fn();
    results.push({ step: name, ok: true, detail: detail ?? "" });
    console.log(`  ok   ${name}${detail ? ` — ${detail}` : ""}`);
  } catch (e) {
    results.push({ step: name, ok: false, detail: e.message.split("\n")[0] });
    console.log(`  FAIL ${name} — ${e.message.split("\n")[0]}`);
    writeFileSync(join(OUT, `fail-${name.replace(/[^a-z0-9]+/gi, "-")}.aria.txt`), await page.locator("body").ariaSnapshot().catch(() => ""));
  }
  await shot(name.replace(/[^a-z0-9]+/gi, "-").toLowerCase());
}
const expect = (ok, what) => { if (!ok) throw new Error(what); };
const button = (name) => page.getByRole("button", { name }).first();
const tokenOf = () => page.evaluate(() => JSON.parse(localStorage.getItem("audio2-auth") ?? "{}")?.state?.token ?? null);
async function api(method, path, body) {
  const token = await tokenOf();
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text.slice(0, 200)}`);
  return text ? JSON.parse(text) : null;
}
async function waitFor(fn, ms = 20_000, every = 500) {
  const end = Date.now() + ms;
  while (Date.now() < end) {
    const v = await fn().catch(() => null);
    if (v) return v;
    await new Promise((r) => setTimeout(r, every));
  }
  return null;
}

await step("sign up", async () => {
  await page.goto(`${BASE}/auth/login`);
  await page.getByRole("button", { name: /Create (an )?account|Sign up/i }).last().click();
  await page.getByLabel("Your name").fill(USER.name);
  await page.getByLabel("Email").fill(USER.email);
  await page.getByLabel("Password", { exact: true }).fill(USER.password);
  const consent = page.getByRole("checkbox");
  for (const box of await consent.all()) if (!(await box.isChecked())) await box.check();
  await page.locator("form").getByRole("button", { name: /^Create account$/ }).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/auth"), { timeout: 20_000 });
  expect(await tokenOf(), "no session after sign-up");
});

if (MAILPIT) {
  await step("the confirmation mail, and confirming", async () => {
    const msg = await waitFor(async () => {
      const r = await (await fetch(`${MAILPIT}/api/v1/search?query=${encodeURIComponent(`to:${USER.email}`)}`)).json();
      return r.messages?.[0] ?? null;
    }, 30_000, 1000);
    expect(msg, "no confirmation mail in Mailpit");
    const full = await (await fetch(`${MAILPIT}/api/v1/message/${msg.ID}`)).json();
    const link = (full.Text + full.HTML).match(/https?:\/\/[^\s"'<>]+\/verify-email\?[^\s"'<>]+/)?.[0]?.replace(/&amp;/g, "&");
    expect(link, "no /verify-email link in the mail");
    await page.goto(link);
    await page.getByText(/confirmed|verified|thank/i).first().waitFor({ timeout: 15_000 });
    return `"${msg.Subject}"`;
  });
}

await step("storage & credit shows the amounts", async () => {
  await page.goto(`${BASE}/billing`);
  await page.getByRole("heading", { name: "Storage & credit" }).waitFor({ timeout: 15_000 });
  await page.getByText(/per GB per month/).first().waitFor();
  const text = await page.locator("main").innerText();
  const credit = text.match(/credit\s+(\$[\d.,]+)/i)?.[1];
  expect(credit, "no credit amount on the page");
  // The welcome credit comes with the confirmed address.
  if (MAILPIT) expect(credit !== "$0.00", `no welcome credit after confirming: ${credit}`);
  return `credit ${credit}`;
});

await step("storage & credit after an upload", async () => {
  await page.goto(`${BASE}/music/upload`);
  const chooser = page.waitForEvent("filechooser");
  await button(/^Choose files$/).click();
  await (await chooser).setFiles(songs.slice(0, 2));
  await waitFor(async () => ((await api("GET", "/api/v2/tracks")).items.length >= 2) || null, 60_000);
  await page.goto(`${BASE}/billing`);
  await page.getByText(/a month at [\d.,]+ [KMG]?B/).first().waitFor({ timeout: 15_000 });
  return (await page.getByText(/a month at/).first().innerText()).trim();
});

await step("narrate a book: up to the quote", async () => {
  const dir = mkdtempSync(join(tmpdir(), "narrate-"));
  const book = join(dir, "smoke-book.txt");
  writeFileSync(book, "Chapter One\n\n" + "The tone rose and fell over the quiet house. ".repeat(80) + "\n\nChapter Two\n\n" + "Nobody minded it at all. ".repeat(80));
  await page.goto(`${BASE}/generate`);
  const chooser = page.waitForEvent("filechooser");
  await button(/^Choose file$/).click();
  await (await chooser).setFiles(book);
  const visited = [];
  // Next through the wizard until the price shows, never pressing what starts the job.
  for (let i = 0; i < 6; i += 1) {
    const heading = (await page.getByRole("heading", { level: 2 }).first().innerText().catch(() => "")).trim();
    visited.push(heading);
    if (await page.getByText(/\$\d+\.\d\d/).first().isVisible().catch(() => false)) break;
    if (/voice/i.test(heading)) await page.getByRole("button", { name: /· (Male|Female) ·/ }).first().click();
    const next = button(/^Next$/);
    await next.waitFor({ timeout: 15_000 });
    await page.waitForFunction(() => [...document.querySelectorAll("button")].some((b) => b.textContent?.trim() === "Next" && !b.disabled), null, { timeout: 30_000 });
    await next.click();
    await page.waitForTimeout(800);
  }
  const price = await page.getByText(/\$\d+\.\d\d/).first().innerText({ timeout: 15_000 });
  return `${visited.join(" → ")} → ${price.trim()}`;
});

if (FEED_URL) {
  await step("translate an episode: up to the price", async () => {
    await api("POST", "/api/v1/podcasts/subscribe", { feed_url: FEED_URL });
    await page.goto(`${BASE}/translate`);
    await page.getByRole("button", { name: /Hosted Smoke Radio/ }).first().click({ timeout: 20_000 });
    await page.getByRole("button", { name: /^Episode Two/ }).first().click();
    const dialog = page.getByRole("dialog");
    await dialog.getByText(/\$\d+\.\d\d/).first().waitFor({ timeout: 20_000 });
    const price = (await dialog.getByText(/\$\d+\.\d\d/).first().innerText()).trim();
    const chars = (await dialog.getByText(/characters/).first().innerText()).trim();
    await dialog.getByRole("button", { name: /^Cancel$/ }).click();
    return `${price}, ${chars}`;
  });
}

await step("sign out", async () => {
  await page.getByRole("button", { name: USER.name }).last().click();
  await page.getByRole("menuitem", { name: /^Sign out$/ }).click();
  await page.waitForURL(/\/auth\/login/, { timeout: 15_000 });
});

await browser.close();
writeFileSync(join(OUT, "report.json"), JSON.stringify({ base: BASE, results, problems, responses }, null, 1));
console.log(`\n${results.filter((r) => !r.ok).length} of ${results.length} steps failed; ${responses} responses watched, ${problems.length} problems`);
for (const p of problems) console.log(`  [${p.step}] ${p.kind}: ${p.what}`);
process.exit(results.some((r) => !r.ok) || problems.length ? 1 : 0);
