// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";

/*
 * A structural guard, not a behavioural test — and it exists because of a real
 * bug rather than a hypothetical one.
 *
 * The player renders two different views (the bar, and the full Now Playing
 * screen). When the `<audio>` element was rendered inside each of them, opening
 * Now Playing unmounted it and mounted a fresh one. `src` is set imperatively
 * in an effect keyed on the track, and the track hadn't changed — so nothing
 * restored it and playback stopped dead with `currentTime` back at 0.
 *
 * Types can't see this and a build can't either. What keeps it fixed is that
 * exactly one `<audio>` exists, above the branch, in a position React never
 * moves it from.
 */

const source = readFileSync(resolve(__dirname, "PlayerBar.tsx"), "utf8");

describe("PlayerBar audio element", () => {
  it("renders exactly one <audio>", () => {
    expect(source.match(/<audio\b/g) ?? []).toHaveLength(1);
  });

  it("keeps it outside the expanded/compact branch, so switching views can't remount it", () => {
    const audioAt = source.indexOf("<audio");
    const expandedAt = source.indexOf("const expandedView");
    const compactAt = source.indexOf("const compactView");

    expect(expandedAt).toBeGreaterThan(-1);
    expect(compactAt).toBeGreaterThan(-1);
    // The element is declared after both views are built — i.e. in the shared
    // return, not inside either one of them.
    expect(audioAt).toBeGreaterThan(expandedAt);
    expect(audioAt).toBeGreaterThan(compactAt);
  });

  it("still swaps the views on `expanded`", () => {
    expect(source).toMatch(/\{expanded \? expandedView : compactView\}/);
  });
});
