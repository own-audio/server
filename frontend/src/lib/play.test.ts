// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { fileResumePosition } from "./play";

describe("fileResumePosition", () => {
  it("uses a position inside the file as it is", () => {
    expect(fileResumePosition(420, 3001, 1100)).toBe(420);
  });

  it("reads an old book-wide position by taking the file's offset off", () => {
    // Saved by the console before 1.0.0-alpha.5: 3001 s of earlier files + 217 s in this one.
    expect(fileResumePosition(3218, 3001, 1100)).toBe(217);
  });

  it("starts the file from the beginning when neither reading fits it", () => {
    expect(fileResumePosition(9000, 3001, 1100)).toBe(0);
  });

  it("leaves the first file alone, where both readings agree", () => {
    expect(fileResumePosition(300, 0, 1280)).toBe(300);
  });

  it("never goes negative", () => {
    expect(fileResumePosition(-5, 0, 100)).toBe(0);
  });
});
