// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { formatDeviceCode } from "./deviceAuth";

describe("formatDeviceCode", () => {
  it("shows a typed code the way the TV shows it", () => {
    expect(formatDeviceCode("bkqxjgfb")).toBe("BKQX-JGFB");
    expect(formatDeviceCode("BKQX-JGFB")).toBe("BKQX-JGFB");
    expect(formatDeviceCode("bkqx jgfb")).toBe("BKQX-JGFB");
  });

  it("groups as you type rather than only once the code is complete", () => {
    expect(formatDeviceCode("bk")).toBe("BK");
    expect(formatDeviceCode("bkqx")).toBe("BKQX");
    expect(formatDeviceCode("bkqxj")).toBe("BKQX-J");
  });

  it("stops at eight characters, so a pasted URL cannot smuggle more in", () => {
    expect(formatDeviceCode("BKQXJGFBZZZZ")).toBe("BKQX-JGFB");
  });

  /**
   * Look-alike folding (O for 0, I and L for 1, S for 5) belongs to the server
   * alone — two copies of that rule can drift, and the cost of drift is a
   * correctly-read code being rejected. This test exists to keep it that way.
   */
  it("does not fold look-alike characters itself", () => {
    expect(formatDeviceCode("OIL5")).toBe("OIL5");
  });
});
