// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, it, expect, beforeEach, beforeAll, vi } from "vitest";
import {
  getTranslationProgress,
  listTranslationsInProgress,
  rememberTranslation,
  saveTranslationPosition,
} from "./translationProgress";

/*
 * A translation's position lives here because the server has nowhere for it
 * (guide §11a). What matters is that it is kept per translation, never mixed
 * with the episode's own position, and that a finished one stops being offered.
 */
describe("translation progress", () => {
  // These tests run in node, where there is no localStorage. A few lines of stub keep the
  // project free of a DOM test environment it needs nowhere else.
  beforeAll(() => {
    const store = new Map<string, string>();
    globalThis.localStorage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: (i: number) => [...store.keys()][i] ?? null,
      get length() {
        return store.size;
      },
    } as Storage;
  });

  beforeEach(() => localStorage.clear());

  it("remembers what a translation is, then where it got to", () => {
    rememberTranslation({
      translationId: "t1",
      episodeId: "e1",
      episodeTitle: "An episode",
      showTitle: "A show",
      targetLanguage: "cs",
      durationSecs: 780,
    });

    saveTranslationPosition("t1", 120, false);

    expect(getTranslationProgress("t1")).toMatchObject({ positionSecs: 120, episodeTitle: "An episode", targetLanguage: "cs" });
  });

  it("ignores a position for a translation nobody remembered", () => {
    saveTranslationPosition("unknown", 90, false);

    expect(getTranslationProgress("unknown")).toBeUndefined();
  });

  it("offers what is unfinished, newest first, and drops what is done", () => {
    // Saves inside one millisecond would otherwise tie, and the order under test is the
    // whole point of the list.
    let now = 1_000;
    vi.spyOn(Date, "now").mockImplementation(() => (now += 1_000));

    rememberTranslation({ translationId: "old", episodeId: "e1", episodeTitle: "Older", showTitle: null, targetLanguage: "de", durationSecs: 600 });
    saveTranslationPosition("old", 60, false);
    rememberTranslation({ translationId: "new", episodeId: "e2", episodeTitle: "Newer", showTitle: null, targetLanguage: "cs", durationSecs: 600 });
    saveTranslationPosition("new", 30, false);
    rememberTranslation({ translationId: "done", episodeId: "e3", episodeTitle: "Finished", showTitle: null, targetLanguage: "cs", durationSecs: 600 });
    saveTranslationPosition("done", 600, true);

    expect(listTranslationsInProgress().map((t) => t.translationId)).toEqual(["new", "old"]);
  });
});
