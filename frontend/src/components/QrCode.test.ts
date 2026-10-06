// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, it, expect } from "vitest";
import jsQR from "jsqr";
import { renderToStaticMarkup } from "react-dom/server";
import { createElement } from "react";
import QrCode from "./QrCode";

/* Renders the component, rebuilds the module grid from the SVG, and reads it
   back with an independent decoder — the only check that means anything for a
   QR is whether a scanner can read it. Caught a hand-written encoder that
   produced unreadable codes. */

function decode(value: string): string | null {
  const svg = renderToStaticMarkup(createElement(QrCode, { value }));

  const viewBox = /viewBox="0 0 (\d+) \d+"/.exec(svg);
  if (!viewBox) return null;
  const total = Number(viewBox[1]);

  const dark = new Set<string>();
  for (const [, x, y] of svg.matchAll(/M(\d+),(\d+)h1v1h-1z/g)) dark.add(`${x},${y}`);

  // Blow each module up so the decoder has pixels to work with.
  const scale = 4;
  const size = total * scale;
  const rgba = new Uint8ClampedArray(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      const on = dark.has(`${Math.floor(px / scale)},${Math.floor(py / scale)}`);
      const i = (py * size + px) * 4;
      const v = on ? 0 : 255;
      rgba[i] = v;
      rgba[i + 1] = v;
      rgba[i + 2] = v;
      rgba[i + 3] = 255;
    }
  }

  return jsQR(rgba, size, size)?.data ?? null;
}

describe("QrCode", () => {
  it("encodes a join URL that a scanner can read back", () => {
    const url = "https://app.own.audio/join/7f3a9c2e";
    expect(decode(url)).toBe(url);
  });

  it("encodes a bare code", () => {
    expect(decode("KJ4T-9WQ2")).toBe("KJ4T-9WQ2");
  });

  it("grows a version when the content needs one", () => {
    const long = "https://app.own.audio/join/" + "a".repeat(80);
    expect(decode(long)).toBe(long);
  });

  it("still decodes a long payload", () => {
    const long = "https://app.own.audio/join/" + "b".repeat(200);
    expect(decode(long)).toBe(long);
  });
});
