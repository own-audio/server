// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";
import { intlLocale, t, useI18n } from ".";

afterEach(() => useI18n.getState().setPreference("en"));

describe("i18n", () => {
  it("fetches a language that is not bundled before switching to it", async () => {
    useI18n.getState().setPreference("de");
    expect(useI18n.getState().preference).toBe("de");
    await vi.waitFor(() => expect(intlLocale()).toBe("de"));
    expect(t("common.action.cancel")).toBe("Abbrechen");
  });

  it("uses each language's own plural categories", async () => {
    useI18n.getState().setPreference("fr");
    await vi.waitFor(() => expect(intlLocale()).toBe("fr"));
    expect(t("audiobooks.bookCount", { count: 1 })).toBe("1 livre");
    expect(t("audiobooks.bookCount", { count: 0 })).toBe("0 livre");
    // French counts round millions with "de".
    expect(t("audiobooks.bookCount", { count: 1_000_000 })).toMatch(/de livres$/);
    useI18n.getState().setPreference("sk");
    await vi.waitFor(() => expect(intlLocale()).toBe("sk"));
    expect(t("audiobooks.bookCount", { count: 3 })).not.toBe(t("audiobooks.bookCount", { count: 5 }));
  });

  it("keeps only the latest choice when catalogs load out of order", async () => {
    useI18n.getState().setPreference("it");
    useI18n.getState().setPreference("es");
    await vi.waitFor(() => expect(intlLocale()).toBe("es"));
    await new Promise((r) => setTimeout(r, 20));
    expect(intlLocale()).toBe("es");
  });
});
