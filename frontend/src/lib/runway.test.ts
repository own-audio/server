// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { isLongRunway } from "./runway";

describe("isLongRunway", () => {
  it("is more than ten years of credit", () => {
    expect(isLongRunway(1666666)).toBe(true);
    expect(isLongRunway(3651)).toBe(true);
    expect(isLongRunway(3650)).toBe(false);
    expect(isLongRunway(12)).toBe(false);
    expect(isLongRunway(null)).toBe(false);
    expect(isLongRunway(undefined)).toBe(false);
  });
});
