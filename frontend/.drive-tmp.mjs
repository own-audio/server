import { chromium } from "playwright";
const BASE = process.env.BASE ?? "http://localhost:5175";
const SHOTS = "/private/tmp/claude-501/-Users-kornelko-vscode-audio2-www/1ed88fd5-625f-4cfa-ac67-8beeeb4a6f9e/scratchpad/shots";

const b = await chromium.launch();
const page = await b.newPage({ viewport: { width: 1400, height: 950 } });
const errors = [];
page.on("console", (m) => { if (m.type() === "error") errors.push(m.text().slice(0, 160)); });
page.on("pageerror", (e) => errors.push("pageerror: " + String(e).slice(0, 160)));

async function shot(name) { await page.screenshot({ path: `${SHOTS}/${name}.png` }); console.log("  shot:", name); }

// ── sign in ────────────────────────────────────────────────
await page.goto(`${BASE}/auth/login`, { waitUntil: "networkidle" });
await page.fill('input[type="email"]', "admin@audio2.local");
await page.fill('input[type="password"]', "admin");
await page.click('button[type="submit"]');
await page.waitForURL((u) => !u.pathname.startsWith("/auth"), { timeout: 20000 });
console.log("signed in ->", page.url());

// ── 1. Discovery: the add dialog's empty state ───────────────
await page.goto(`${BASE}/podcasts/add`, { waitUntil: "networkidle" });
await page.waitForTimeout(1500);
const chips = await page.locator("button:has-text('technology'), button:has-text('society')").count();
console.log(`category chips visible: ${chips > 0}`);
await shot("1-discover-categories");

// ── 2. Browse one category ─────────────────────────────────
const tech = page.locator("button", { hasText: /^technology/ }).first();
if (await tech.count()) {
  await tech.click();
  await page.waitForTimeout(2500);
  const rows = await page.locator("li:has(button:has-text('Follow'))").count();
  console.log(`browse technology -> ${rows} result rows`);
  await shot("2-browse-technology");
}

// ── 3. Search still works ──────────────────────────────────
await page.fill('input[aria-label="Search podcasts"]', "planet money");
await page.waitForTimeout(2500);
const first = await page.locator("li p.font-medium").first().textContent().catch(() => null);
console.log("search first result:", JSON.stringify(first));
await shot("3-search");

// ── 4. "You might also like" on a feed ─────────────────────
await page.goto(`${BASE}/podcasts`, { waitUntil: "networkidle" });
await page.waitForTimeout(1200);
await page.locator("[class*=cursor-pointer], button, a").filter({ hasText: /Darknet|99%|Radiolab|Planet/ }).first().click().catch(() => {});
await page.waitForTimeout(3000);
const heading = page.locator("h2:has-text('You might also like')");
const has = await heading.count();
console.log("'You might also like' section present:", has > 0);
if (has) { await heading.scrollIntoViewIfNeeded(); await page.waitForTimeout(600); }
await shot("4-feed-similar");

// ── 5. The switch in settings ──────────────────────────────
await page.goto(`${BASE}/settings`, { waitUntil: "networkidle" });
await page.waitForTimeout(1200);
const sw = page.locator("label:has-text('Suggest things based on what I listen to') input[type=checkbox]");
console.log("switch present:", await sw.count() > 0, "| checked:", await sw.isChecked().catch(() => "n/a"));
await page.locator("h2:has-text('Recommendations')").scrollIntoViewIfNeeded().catch(() => {});
await page.waitForTimeout(400);
await shot("5-settings-switch");

console.log(errors.length ? "CONSOLE ERRORS:\n  " + errors.slice(0, 6).join("\n  ") : "no console errors");
await b.close();
