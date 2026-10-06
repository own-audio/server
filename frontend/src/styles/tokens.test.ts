// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import { resolve } from "node:path";

/* Guards the hand-mirrored copy of audio2-android-book/docs/design-tokens.json
   the way the Android and Swift mirrors do. Only the parts the web app is
   meant to share are compared: neutrals, radii, motion. The accent and the
   cloud colours deliberately follow the brand brief instead (purple core,
   text-safe gold/sky/red) until design-tokens.json is updated to match —
   see docs/web-app-parity-plan.md Q1. */

const css = readFileSync(resolve(__dirname, "tokens.css"), "utf8");
const tokensPath = resolve(__dirname, "../../../../audio2-android-book/docs/design-tokens.json");

function block(selector: string): string {
  const start = css.indexOf(selector);
  if (start < 0) throw new Error(`no ${selector} block`);
  const open = css.indexOf("{", start);
  let depth = 0;
  for (let i = open; i < css.length; i++) {
    if (css[i] === "{") depth++;
    if (css[i] === "}") depth--;
    if (depth === 0) return css.slice(open, i);
  }
  throw new Error("unterminated block");
}

function value(blockCss: string, name: string): string {
  const m = blockCss.match(new RegExp(`--${name}:\\s*([^;]+);`));
  if (!m) throw new Error(`--${name} missing`);
  return m[1].trim().toLowerCase();
}

const light = block(":root {");
const dark = block(':root[data-theme="dark"]');
const system = block(':root:not([data-theme="light"])');

const neutrals: Record<string, string> = { bg: "bg", "bg-alt": "bgAlt", card: "card", fg: "fg", muted: "muted", border: "border" };

describe("tokens.css", () => {
  it("dark blocks agree with each other", () => {
    for (const name of [...Object.keys(neutrals), "accent", "book", "podcast", "music"]) {
      expect(value(dark, name)).toBe(value(system, name));
    }
  });

  it("uses the brand core purple as the accent", () => {
    expect(value(light, "accent")).toBe("#6e44ff");
    expect(value(dark, "accent")).toBe("#9b7bff");
  });

  const hasTokens = existsSync(tokensPath);
  it.skipIf(!hasTokens)("neutrals match design-tokens.json", () => {
    const tokens = JSON.parse(readFileSync(tokensPath, "utf8"));
    for (const [cssName, jsonName] of Object.entries(neutrals)) {
      expect(value(light, cssName)).toBe(tokens.color[jsonName].light.toLowerCase());
      expect(value(dark, cssName)).toBe(tokens.color[jsonName].dark.toLowerCase());
    }
  });

  it.skipIf(!hasTokens)("radii and motion match design-tokens.json", () => {
    const tokens = JSON.parse(readFileSync(tokensPath, "utf8"));
    const theme = block("@theme inline");
    expect(value(theme, "radius-card")).toBe(`${tokens.radius.card.$value}px`);
    expect(value(theme, "radius-cover")).toBe(`${tokens.radius.cover.$value}px`);
    expect(value(theme, "radius-player")).toBe(`${tokens.radius.miniPlayer.$value}px`);
    expect(value(theme, "radius-sheet")).toBe(`${tokens.radius.sheetTop.$value}px`);
    expect(value(theme, "radius-pill")).toBe(`${tokens.radius.pill.$value}px`);
    expect(value(theme, "duration-cover")).toBe(`${tokens.motion.coverCrossfadeMs.$value}ms`);
    expect(value(theme, "duration-screen")).toBe(`${tokens.motion.screenTransitionMs.$value}ms`);
  });
});
