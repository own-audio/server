// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { defaultFamilyName } from "./familyName";

describe("defaultFamilyName", () => {
  it("is the surname, as on the server", () => {
    expect(defaultFamilyName("Release Owner")).toBe("Owner");
    expect(defaultFamilyName("  Jana  Nováková ")).toBe("Nováková");
    expect(defaultFamilyName("Kornel")).toBe("Kornel");
    expect(defaultFamilyName("")).toBe("");
  });
});
