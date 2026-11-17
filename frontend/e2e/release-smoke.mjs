// SPDX-License-Identifier: AGPL-3.0-or-later
// A release smoke test of the web console against a fresh server, in a real (headless)
// browser: the setup wizard, sign-in, music upload and playback, playlists, favourites, a
// podcast from a feed, an audiobook with its progress, a family invite and a second account
// joining through the mailed link, the trash, search, the settings pages and sign-out. Every
// browser console error and every 4xx/5xx answer is reported, step by step.
//
// Needs a server that has never been set up, with mail going to Mailpit and a feed the server
// can reach (the release checklist's compose stack does both):
//
//   BASE_URL=http://localhost:18251 MAILPIT_URL=http://localhost:18255 \
//   FEED_URL=http://feeds:18257/feed.xml HOST_MAP=feeds MEDIA_DIR=/path/with/songs-and-book \
//     node e2e/release-smoke.mjs [out-dir]
//
// MEDIA_DIR holds songs/*.mp3 (tagged, a few seconds long or more) and book/*.mp3 (an
// audiobook's chapters). Writes a screenshot per step and report.json to out-dir.
import { chromium } from "playwright";
import { mkdirSync, readdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const BASE = (process.env.BASE_URL ?? "http://localhost:8080").replace(/\/$/, "");
const MAILPIT = (process.env.MAILPIT_URL ?? "").replace(/\/$/, "");
const FEED_URL = process.env.FEED_URL ?? "";
const MEDIA = process.env.MEDIA_DIR ?? "media";
const OUT = process.argv[2] ?? "e2e-release";
mkdirSync(OUT, { recursive: true });

const OWNER = { name: "Release Owner", email: "owner@example.com", password: "release-smoke-owner-1" };
const MEMBER = { name: "Release Member", email: "member@example.com", password: "release-smoke-member-1" };
const songs = readdirSync(join(MEDIA, "songs")).filter((f) => f.endsWith(".mp3")).sort().map((f) => join(MEDIA, "songs", f));
const chapters = readdirSync(join(MEDIA, "book")).filter((f) => f.endsWith(".mp3")).sort().map((f) => join(MEDIA, "book", f));

const results = [];
const problems = [];
let current = "start";
let shotN = 0;
let responses = 0;

// HOST_MAP=feeds: the feed's host, as the server sees it in Docker, is this machine to the
// browser too, so episodes that stream from the feed play.
const browser = await chromium.launch({ args: process.env.HOST_MAP ? [`--host-resolver-rules=MAP ${process.env.HOST_MAP} 127.0.0.1`] : [] });
const context = await browser.newContext({ locale: "en-US", viewport: { width: 1440, height: 900 } });

function watch(page, who) {
  page.on("pageerror", (e) => problems.push({ step: current, who, kind: "js", what: e.message }));
  page.on("console", (m) => {
    if (m.type() === "error") problems.push({ step: current, who, kind: "console", what: m.text().slice(0, 400) });
  });
  page.on("response", (r) => {
    responses += 1;
    if (r.status() >= 400) problems.push({ step: current, who, kind: "http", what: `${r.status()} ${r.request().method()} ${r.url().replace(BASE, "")}` });
  });
  page.on("requestfailed", (r) => {
    const why = r.failure()?.errorText ?? "";
    // A media element that stops loading when playback moves on is not a failure.
    if (!/ERR_ABORTED/.test(why)) problems.push({ step: current, who, kind: "network", what: `${why} ${r.method()} ${r.url().replace(BASE, "")}` });
  });
}

const page = await context.newPage();
watch(page, "owner");

async function shot(p, name) {
  shotN += 1;
  await p.screenshot({ path: join(OUT, `${String(shotN).padStart(2, "0")}-${name}.png`) }).catch(() => {});
}

async function step(name, fn, p = page) {
  current = name;
  const started = Date.now();
  try {
    const detail = await fn();
    results.push({ step: name, ok: true, ms: Date.now() - started, detail: detail ?? "" });
    console.log(`  ok   ${name}${detail ? ` — ${detail}` : ""}`);
  } catch (e) {
    results.push({ step: name, ok: false, ms: Date.now() - started, detail: e.message.split("\n")[0] });
    console.log(`  FAIL ${name} — ${e.message.split("\n")[0]}`);
    writeFileSync(join(OUT, `fail-${name.replace(/[^a-z0-9]+/gi, "-")}.aria.txt`), await p.locator("body").ariaSnapshot().catch(() => ""));
  }
  await shot(p, name.replace(/[^a-z0-9]+/gi, "-").toLowerCase());
}

const expect = (ok, what) => { if (!ok) throw new Error(what); };
const button = (p, name) => p.getByRole("button", { name }).first();
const tokenOf = (p) => p.evaluate(() => JSON.parse(localStorage.getItem("audio2-auth") ?? "{}")?.state?.token ?? null);

async function api(p, method, path, body) {
  const token = await tokenOf(p);
  const res = await fetch(`${BASE}${path}`, {
    method,
    headers: { "content-type": "application/json", ...(token ? { authorization: `Bearer ${token}` } : {}) },
    body: body ? JSON.stringify(body) : undefined,
  });
  const text = await res.text();
  if (!res.ok) throw new Error(`${method} ${path}: ${res.status} ${text.slice(0, 200)}`);
  return text ? JSON.parse(text) : null;
}

// Whether the page's audio element really plays: its clock moves.
async function audioAdvances(p, ms = 3000) {
  const a = await p.evaluate(() => [...document.querySelectorAll("audio")].map((x) => x.currentTime));
  await p.waitForTimeout(ms);
  const b = await p.evaluate(() => [...document.querySelectorAll("audio")].map((x) => ({ t: x.currentTime, paused: x.paused, src: x.currentSrc.slice(0, 60) })));
  const moved = b.some((x, i) => x.t > (a[i] ?? 0) + 0.5);
  return { moved, state: JSON.stringify(b) };
}

async function waitFor(fn, ms = 20_000, every = 500) {
  const end = Date.now() + ms;
  let last;
  while (Date.now() < end) {
    last = await fn().catch((e) => { last = e; return null; });
    if (last) return last;
    await new Promise((r) => setTimeout(r, every));
  }
  return null;
}

// ── 1. first-time setup ─────────────────────────────────────────────────────
await step("setup wizard: server checks", async () => {
  await page.goto(`${BASE}/`);
  await page.waitForURL(/\/setup/, { timeout: 20_000 });
  await button(page, /^Continue$/).click({ timeout: 30_000 });
});

await step("setup wizard: first account", async () => {
  await page.getByLabel("Your name").fill(OWNER.name);
  await page.getByLabel("Email").fill(OWNER.email);
  await page.getByLabel("Password", { exact: true }).fill(OWNER.password);
  await button(page, /^Create account$/).click();
  await page.getByText(/library/i).first().waitFor({ timeout: 20_000 });
});

await step("setup wizard: library and apps", async () => {
  await button(page, /^Continue$/).click();
  await button(page, /^Open the library$/).waitFor({ timeout: 15_000 });
  await shot(page, "setup-apps");
  await button(page, /^Open the library$/).click();
  await page.waitForURL((u) => !u.pathname.startsWith("/setup"), { timeout: 15_000 });
  expect(await tokenOf(page), "no session after setup");
});

// ── 2. sign out and back in ─────────────────────────────────────────────────
async function signOut(p, who) {
  // The account menu is the button that carries the person's name and address.
  await p.getByRole("button", { name: who.name }).last().click();
  await p.getByRole("menuitem", { name: /^Sign out$/ }).click();
  await p.waitForURL(/\/auth\/login/, { timeout: 15_000 });
}
async function signIn(p, who) {
  if (!/\/auth\/login/.test(p.url())) await p.goto(`${BASE}/auth/login`);
  await p.getByLabel("Email").fill(who.email);
  await p.getByLabel("Password", { exact: true }).fill(who.password);
  await p.getByRole("button", { name: /^Sign in$/ }).last().click();
  await p.waitForURL((u) => !u.pathname.startsWith("/auth"), { timeout: 15_000 });
}

await step("sign out", async () => signOut(page, OWNER));
await step("sign in", async () => signIn(page, OWNER));

// ── 3. music ────────────────────────────────────────────────────────────────
await step("music: upload songs", async () => {
  await page.goto(`${BASE}/music/upload`);
  const chooser = page.waitForEvent("filechooser");
  await button(page, /^Choose files$/).click();
  await (await chooser).setFiles(songs);
  const tracks = await waitFor(async () => {
    const r = await api(page, "GET", "/api/v2/tracks?limit=500");
    return r.items.length >= songs.length ? r.items : null;
  }, 60_000);
  expect(tracks, "the songs did not arrive");
  return `${tracks.length} tracks on the server`;
});

await step("music: the library lists the songs", async () => {
  await page.goto(`${BASE}/music`);
  for (const n of [1, 2, 3, 4]) await page.getByRole("button", { name: `Play Release Tone ${n}` }).first().waitFor({ timeout: 15_000 });
});

await step("music: play a song", async () => {
  await page.getByRole("button", { name: "Play Release Tone 1" }).first().click();
  const r = await audioAdvances(page);
  expect(r.moved, `the audio element did not advance: ${r.state}`);
  return r.state;
});

await step("music: favourite from the player", async () => {
  await button(page, /^Expand$/).click();
  await button(page, /^Add to favorites$/).click();
  await button(page, /^Remove from favorites$/).waitFor({ timeout: 10_000 });
  const favs = await api(page, "GET", "/api/v2/me/favourites?kind=track");
  expect(favs.items.length === 1, `favourites: ${JSON.stringify(favs.items)}`);
  await button(page, /^Minimize$/).click();
  await button(page, /^Pause$/).click();
});

await step("music: a playlist of every song", async () => {
  await button(page, /^Add to playlist$/).click();
  await page.getByRole("dialog").getByRole("textbox", { name: "New playlist" }).fill("Smoke Mix");
  await button(page, /^Create and add$/).click();
  await page.getByRole("dialog").waitFor({ state: "detached", timeout: 10_000 }).catch(() => {});
  await page.getByRole("radio", { name: /^Playlists$/ }).click();
  await page.getByText("Smoke Mix").first().click();
  await page.getByRole("button", { name: "Play Release Tone 4" }).first().waitFor({ timeout: 10_000 });
  const pls = await api(page, "GET", "/api/v2/playlists");
  const mix = pls.items.find((x) => x.name === "Smoke Mix");
  expect(mix, "no Smoke Mix on the server");
  const entries = await api(page, "GET", `/api/v2/playlists/${mix.id}/entries`);
  expect(entries.items.length === 4, `entries: ${entries.items.length}`);
  return "4 entries";
});

await step("music: search", async () => {
  await button(page, /^Search$/).click();
  await page.getByPlaceholder(/Search audiobooks, podcasts, songs/).fill("Release Tone 3");
  await page.getByRole("dialog").getByText("Release Tone 3").first().waitFor({ timeout: 10_000 });
  await shot(page, "search-results");
  await page.keyboard.press("Escape");
});

// ── 4. podcasts ─────────────────────────────────────────────────────────────
if (FEED_URL) {
  await step("podcasts: follow a feed", async () => {
    await page.goto(`${BASE}/podcasts/add`);
    await page.getByRole("textbox", { name: "Or paste a feed URL" }).fill(FEED_URL);
    await button(page, /^Follow$/).click();
    await page.getByText("Smoke Test Radio").first().waitFor({ timeout: 30_000 });
    const shows = await api(page, "GET", "/api/v2/podcasts");
    const show = shows.items.find((x) => x.title === "Smoke Test Radio");
    expect(show, "the show is not on the server");
    const eps = await waitFor(async () => {
      const r = await api(page, "GET", `/api/v2/podcasts/${show.id}/episodes`);
      return r.items.length >= 2 ? r.items : null;
    });
    expect(eps, "episodes did not arrive");
    return `${eps.length} episodes`;
  });

  await step("podcasts: play an episode", async () => {
    await page.goto(`${BASE}/podcasts`);
    await page.getByText("Smoke Test Radio").first().click();
    await page.getByText("Episode Two").first().waitFor({ timeout: 15_000 });
    // An episode that is not on the server yet: the server fetches it from the feed first.
    await page.getByRole("listitem").filter({ hasText: "Episode Two" }).getByRole("button", { name: /^(Fetch and play|Play)$/ }).click();
    const r = await audioAdvances(page);
    expect(r.moved, `the episode did not play: ${r.state}`);
    await button(page, /^Pause$/).click();
    return r.state;
  });
}

// ── 5. an audiobook ─────────────────────────────────────────────────────────
await step("audiobooks: upload a book of chapters", async () => {
  await page.goto(`${BASE}/audiobooks/upload`);
  const chooser = page.waitForEvent("filechooser");
  await button(page, /^Choose files$/).click();
  await (await chooser).setFiles(chapters);
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("textbox", { name: "Title" }).fill("The Smoke Test Book");
  await dialog.getByRole("textbox", { name: "Author" }).fill("Ada Narrator");
  await dialog.getByRole("button", { name: /^Upload/ }).click();
  const book = await waitFor(async () => {
    const r = await api(page, "GET", "/api/v2/books");
    return r.items.find((b) => b.title === "The Smoke Test Book") ?? null;
  }, 60_000);
  expect(book, "the book did not arrive");
  const full = await api(page, "GET", `/api/v2/books/${book.id}`);
  expect(full.files?.length === chapters.length, `files: ${full.files?.length}`);
  return `${full.files.length} files`;
});

await step("audiobooks: play and keep the position", async () => {
  await page.goto(`${BASE}/audiobooks`);
  await page.getByText("The Smoke Test Book").first().click();
  await button(page, /^(Play|Resume|Listen)/).click();
  const r = await audioAdvances(page, 5000);
  expect(r.moved, `the book did not play: ${r.state}`);
  await button(page, /^Pause$/).click();
  await page.waitForTimeout(1500);
  const books = await api(page, "GET", "/api/v2/books");
  const id = books.items.find((b) => b.title === "The Smoke Test Book").id;
  const prog = await waitFor(async () => {
    const p = await api(page, "GET", `/api/v2/me/progress/book/${id}`).catch(() => null);
    return p && p.position_ms > 0 ? p : null;
  }, 15_000);
  expect(prog, "no progress saved for the book");
  await page.reload();
  await page.getByText("The Smoke Test Book").first().waitFor({ timeout: 15_000 });
  return `position ${prog.position_ms} ms`;
});

// ── 6. the family: an emailed invite and a second account ───────────────────
const member = await context.browser().newContext({ locale: "en-US", viewport: { width: 1440, height: 900 } });
const mpage = await member.newPage();
watch(mpage, "member");

await step("family: invite by email", async () => {
  await page.goto(`${BASE}/family`);
  await button(page, /^Invite$/).click();
  await page.getByRole("dialog").getByLabel("Their email").fill(MEMBER.email);
  await page.getByRole("dialog").getByRole("button", { name: /invite|Create|Send/i }).last().click();
  await page.getByText(/Share this with them|Invite sent|sent/i).first().waitFor({ timeout: 15_000 });
  await shot(page, "invite-created");
  await page.keyboard.press("Escape");
});

let joinLink = null;
if (MAILPIT) {
  await step("family: the invite arrives by mail", async () => {
    const msg = await waitFor(async () => {
      const r = await (await fetch(`${MAILPIT}/api/v1/search?query=${encodeURIComponent(`to:${MEMBER.email}`)}`)).json();
      return r.messages?.[0] ?? null;
    }, 30_000, 1000);
    expect(msg, "no invite mail in Mailpit");
    const full = await (await fetch(`${MAILPIT}/api/v1/message/${msg.ID}`)).json();
    joinLink = (full.Text + full.HTML).match(/https?:\/\/[^\s"'<>]+\/join\/[A-Za-z0-9_-]+/)?.[0] ?? null;
    expect(joinLink, "no /join/ link in the mail");
    return `"${msg.Subject}" → ${joinLink.replace(BASE, "")}`;
  });
}

await step("family: the second account joins", async () => {
  if (!joinLink) {
    const invites = await api(page, "GET", "/api/v2/family/invites");
    joinLink = `${BASE}/join/${invites.items[0].code}`;
  }
  current = "family: the second account joins";
  await mpage.goto(joinLink);
  const name = mpage.getByLabel("Your name");
  if (!(await name.isVisible().catch(() => false))) await mpage.getByRole("button", { name: /Create one|new here|Create/i }).first().click().catch(() => {});
  await mpage.getByLabel("Your name").fill(MEMBER.name);
  const email = mpage.getByLabel("Email");
  if (await email.isEditable().catch(() => false)) await email.fill(MEMBER.email);
  await mpage.getByLabel("Password", { exact: true }).fill(MEMBER.password);
  await mpage.locator("form").getByRole("button").last().click();
  await mpage.waitForURL((u) => !u.pathname.startsWith("/join"), { timeout: 20_000 });
  await shot(mpage, "member-joined");
  const fam = await api(mpage, "GET", "/api/v2/family");
  return `member in "${fam.name}"`;
}, mpage);

await step("family: the owner sees two members", async () => {
  await page.goto(`${BASE}/family`);
  await page.getByText(MEMBER.name).first().waitFor({ timeout: 15_000 });
});

await step("music: share every song with the family", async () => {
  await page.goto(`${BASE}/music`);
  await page.getByRole("radio", { name: /^Songs$/ }).click();
  await button(page, /^Share with family$/).click();
  const confirm = page.getByRole("alertdialog").or(page.getByRole("dialog"));
  if (await confirm.first().isVisible().catch(() => false)) await confirm.first().getByRole("button", { name: /Share/ }).last().click();
  const shared = await waitFor(async () => {
    const r = await api(page, "GET", "/api/v2/tracks");
    return r.items.every((t) => t.visibility === "family") ? r.items : null;
  }, 15_000);
  expect(shared, "the songs did not become family songs");
});

await step("family: the member sees the shared library", async () => {
  await mpage.goto(`${BASE}/music`);
  await mpage.getByRole("button", { name: "Play Release Tone 2" }).first().waitFor({ timeout: 15_000 });
  const tracks = await api(mpage, "GET", "/api/v2/tracks");
  expect(tracks.items.length === songs.length, `the member sees ${tracks.items.length} songs`);
  await shot(mpage, "member-music");
  return `${tracks.items.length} songs visible to the member`;
}, mpage);

// ── 7. trash ────────────────────────────────────────────────────────────────
await step("trash: move a song to the trash", async () => {
  await page.goto(`${BASE}/music`);
  await page.getByRole("radio", { name: /^Songs$/ }).click();
  const row = page.getByRole("listitem").filter({ has: page.getByRole("button", { name: "Play Release Tone 4" }) }).first();
  await row.getByRole("button", { name: "More" }).click();
  await page.getByRole("menuitem", { name: /^Move to Trash$/ }).click();
  const confirm = page.getByRole("alertdialog").or(page.getByRole("dialog"));
  if (await confirm.first().isVisible().catch(() => false)) await confirm.first().getByRole("button", { name: /Trash|Delete|Move/ }).last().click();
  await waitFor(async () => (await api(page, "GET", "/api/v2/tracks")).items.length === songs.length - 1 || null, 15_000);
});

await step("trash: restore it", async () => {
  await page.goto(`${BASE}/trash`);
  await page.getByText("Release Tone 4").first().waitFor({ timeout: 15_000 });
  await shot(page, "trash-list");
  await button(page, /^Restore/).click();
  const back = await waitFor(async () => (await api(page, "GET", "/api/v2/tracks")).items.length === songs.length || null, 15_000);
  expect(back, "the song did not come back");
});

// ── 8. settings, stats, sign-out ────────────────────────────────────────────
await step("settings: profile name", async () => {
  await page.goto(`${BASE}/settings`);
  await page.getByRole("textbox", { name: "Your name" }).fill("Release Owner 2");
  await page.getByRole("button", { name: /^Save$/ }).first().click();
  await waitFor(async () => (await api(page, "GET", "/api/v2/me")).display_name === "Release Owner 2" || null, 10_000);
  OWNER.name = "Release Owner 2";
});

await step("settings: two-factor set-up opens", async () => {
  await button(page, /^Set up$/).click();
  await page.getByText(/authenticator|QR|secret/i).first().waitFor({ timeout: 10_000 });
  await shot(page, "totp-setup");
  await page.keyboard.press("Escape");
  await page.getByRole("button", { name: /^Cancel$/ }).first().click({ timeout: 2000 }).catch(() => {});
});

await step("settings: devices and recent activity", async () => {
  await page.goto(`${BASE}/settings`);
  await page.getByRole("heading", { name: "Devices" }).scrollIntoViewIfNeeded();
  await page.getByRole("heading", { name: "Recent activity" }).waitFor();
  await page.getByRole("heading", { name: "Recent activity" }).scrollIntoViewIfNeeded();
});

await step("settings: change password", async () => {
  await page.goto(`${BASE}/settings`);
  await page.getByRole("textbox", { name: "Current password" }).fill(OWNER.password);
  await page.getByRole("textbox", { name: "New password" }).fill(`${OWNER.password}-2`);
  await page.getByRole("textbox", { name: "Confirm" }).fill(`${OWNER.password}-2`);
  await button(page, /^Change password$/).click();
  OWNER.password = `${OWNER.password}-2`;
  await page.waitForTimeout(2000);
});

await step("settings: playback page", async () => {
  await page.goto(`${BASE}/settings/playback`);
  await page.getByRole("heading").first().waitFor();
});

await step("stats page", async () => {
  await page.goto(`${BASE}/stats`);
  await page.getByRole("heading").first().waitFor();
  await page.waitForTimeout(1500);
});

await step("home page", async () => {
  await page.goto(`${BASE}/`);
  await page.getByRole("heading").first().waitFor();
  await page.waitForTimeout(1500);
});

await step("sign out (the owner)", async () => signOut(page, OWNER));
await step("sign in with the new password", async () => signIn(page, OWNER));
await step("sign out (the member)", async () => signOut(mpage, MEMBER), mpage);

await browser.close();
writeFileSync(join(OUT, "report.json"), JSON.stringify({ base: BASE, results, problems, responses }, null, 1));
console.log(`\n${results.filter((r) => !r.ok).length} of ${results.length} steps failed; ${responses} responses watched, ${problems.length} problems`);
for (const p of problems) console.log(`  [${p.step}] ${p.who} ${p.kind}: ${p.what}`);
process.exit(results.some((r) => !r.ok) || problems.length ? 1 : 0);
