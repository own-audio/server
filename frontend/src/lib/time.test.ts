// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, it, expect } from "vitest";
import { formatClock, formatUntil } from "./time";

describe("formatClock", () => {
  it("drops the hour under an hour", () => {
    expect(formatClock(75)).toBe("1:15");
  });
  it("pads minutes once there are hours", () => {
    expect(formatClock(3725)).toBe("1:02:05");
  });
  it("survives NaN from an audio element with no metadata yet", () => {
    expect(formatClock(NaN)).toBe("0:00");
  });
});

describe("formatUntil", () => {
  it("counts forward, not backward", () => {
    expect(formatUntil(new Date(Date.now() + 6 * 86_400_000).toISOString())).toBe("in 6 days");
  });
  it("says expired rather than rendering a negative", () => {
    expect(formatUntil(new Date(Date.now() - 60_000).toISOString())).toBe("expired");
  });
});
