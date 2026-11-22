// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { coverActions } from "./jobCover";
import { offersBookTranslation } from "./features";
import { NO_FEATURES } from "../api/server";

describe("cover actions on a finished narration", () => {
  it("offers a new cover while some are left", () => {
    expect(coverActions({ cover_mode: "auto", cover_regenerations_left: 5, source_has_cover: false }).regenerate).toBe(true);
    expect(coverActions({ cover_mode: "auto", cover_regenerations_left: 0, source_has_cover: false }).regenerate).toBe(false);
  });

  it("offers the book's own cover only when the file has one, and it is not already on", () => {
    expect(coverActions({ cover_mode: "auto", cover_regenerations_left: 1, source_has_cover: true }).useBook).toBe(true);
    expect(coverActions({ cover_mode: "book", cover_regenerations_left: 1, source_has_cover: true }).useBook).toBe(false);
    expect(coverActions({ cover_mode: "auto", cover_regenerations_left: 1, source_has_cover: false }).useBook).toBe(false);
    expect(coverActions({ cover_mode: "auto", cover_regenerations_left: 1, source_has_cover: null }).useBook).toBe(false);
  });

  it("offers removing only a cover that is there", () => {
    expect(coverActions({ cover_mode: "custom", cover_regenerations_left: 1, source_has_cover: null }).remove).toBe(true);
    expect(coverActions({ cover_mode: "none", cover_regenerations_left: 1, source_has_cover: null }).remove).toBe(false);
  });
});

describe("Translate a book is offered", () => {
  it("on narration and translation together until the server has its own flag", () => {
    expect(offersBookTranslation({ ...NO_FEATURES, narration: true, translation: true })).toBe(true);
    expect(offersBookTranslation({ ...NO_FEATURES, narration: true, translation: false })).toBe(false);
    expect(offersBookTranslation({ ...NO_FEATURES, narration: false, translation: true })).toBe(false);
    expect(offersBookTranslation(NO_FEATURES)).toBe(false);
  });

  it("by the flag alone once the server sends it", () => {
    expect(offersBookTranslation({ ...NO_FEATURES, narration: true, translation: true, book_translation: false })).toBe(false);
    expect(offersBookTranslation({ ...NO_FEATURES, narration: true, translation: false, book_translation: true })).toBe(true);
  });
});
