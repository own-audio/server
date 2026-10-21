// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { chapterIndex, formatChapterTime, nextChapterStart, parseChapterTime, previousChapterStart } from "./chapters";

const concert = [
  { title: "Intro", startSecs: 0 },
  { title: "The Emptiness Machine", startSecs: 157 },
  { title: "Lying From You", startSecs: 377 },
];

describe("chapters", () => {
  it("knows the chapter playing", () => {
    expect(chapterIndex(concert, 0)).toBe(0);
    expect(chapterIndex(concert, 156.9)).toBe(0);
    expect(chapterIndex(concert, 156.97)).toBe(1);
    expect(chapterIndex([], 5)).toBeNull();
  });

  it("goes to the next song, then leaves the track", () => {
    expect(nextChapterStart(concert, 10)).toBe(157);
    expect(nextChapterStart(concert, 400)).toBeNull();
  });

  it("restarts a song a few seconds in, else goes back one", () => {
    expect(previousChapterStart(concert, 200)).toBe(157);
    expect(previousChapterStart(concert, 158)).toBe(0);
    expect(previousChapterStart(concert, 60)).toBeNull();
  });

  it("reads and writes times as people do", () => {
    expect(formatChapterTime(566)).toBe("9:26");
    expect(formatChapterTime(3723.9)).toBe("1:02:03");
    expect(parseChapterTime(" 1:02:03 ")).toBe(3723);
    expect(parseChapterTime("75")).toBe(75);
    for (const bad of ["", "abc", "1:75", "1::2", "-1:00", "1:2:3:4"]) expect(parseChapterTime(bad)).toBeNull();
  });
});
