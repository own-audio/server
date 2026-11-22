// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it } from "vitest";
import { useI18n } from "../i18n";
import type { PodcastEpisodeTranslation } from "../api/types";
import {
  bookMachineTranslationLine,
  episodeMachineTranslationLine,
  isMachineTranslatedBook,
  isMachineTranslatedEpisode,
  languageInSentence,
  voiceName,
} from "./machineTranslation";

afterEach(() => useI18n.setState({ locale: "en" }));
useI18n.setState({ locale: "en" });

const translated = { language: "en", origin: { kind: "machine_translated", source_language: "cs", translator: "google", voice: "Sarah" } };

const episode = (extra: Partial<PodcastEpisodeTranslation> = {}): PodcastEpisodeTranslation => ({
  id: "t1",
  episode_id: "e1",
  source_language: "en",
  target_language: "de",
  voice_profile_id: "gemini_flash:de-DE:Lena",
  status: "complete",
  char_count: 1,
  quoted_price_cents: 1,
  charged_price_cents: 1,
  currency: "USD",
  error: null,
  stream_url: "x",
  notice: "",
  ...extra,
});

describe("machine-translation badge", () => {
  it("marks a book only when its origin says so", () => {
    expect(isMachineTranslatedBook(translated)).toBe(true);
    expect(isMachineTranslatedBook({ origin: { kind: "narrated", voice: "Sarah" } })).toBe(false);
    expect(isMachineTranslatedBook({ origin: null })).toBe(false);
    expect(isMachineTranslatedBook({})).toBe(false);
    expect(isMachineTranslatedBook({ origin: { kind: "something_new" } })).toBe(false);
    expect(isMachineTranslatedBook(null)).toBe(false);
  });

  it("marks an episode translation only when the server flags it", () => {
    expect(isMachineTranslatedEpisode(episode({ machine_translated: true }))).toBe(true);
    expect(isMachineTranslatedEpisode(episode())).toBe(false);
    expect(isMachineTranslatedEpisode(episode({ machine_translated: false }))).toBe(false);
  });
});

describe("the detail line", () => {
  it("names Google and the languages in English", () => {
    expect(bookMachineTranslationLine(translated)).toBe("Machine-translated by Google from Czech to English, read by Sarah.");
  });

  it("puts the languages in the genitive in Czech, as the server does", () => {
    useI18n.setState({ locale: "cs" });
    expect(bookMachineTranslationLine(translated)).toBe("Strojově přeloženo Googlem z češtiny do angličtiny, čte Sarah.");
    expect(languageInSentence("de-DE")).toBe("němčiny");
    expect(languageInSentence("pt")).toBe("portugalštiny");
  });

  it("leaves the translator out when it is not Google", () => {
    expect(bookMachineTranslationLine({ ...translated, origin: { ...translated.origin, translator: "deepl" } })).toBe(
      "Machine-translated from Czech to English, read by Sarah."
    );
  });

  it("says nothing when the book does not tell all of it", () => {
    expect(bookMachineTranslationLine({ ...translated, language: null })).toBeNull();
    expect(bookMachineTranslationLine({ ...translated, origin: { ...translated.origin, voice: null } })).toBeNull();
    expect(bookMachineTranslationLine({ language: "en", origin: { kind: "narrated", voice: "Sarah" } })).toBeNull();
  });

  it("writes an episode's line from the translation and its voice", () => {
    expect(episodeMachineTranslationLine(episode({ machine_translated: true, translator: "google" }))).toBe(
      "Machine-translated by Google from English to German, read by Lena."
    );
    expect(episodeMachineTranslationLine(episode())).toBeNull();
  });

  it("reads a voice's name from the list, else from its id", () => {
    expect(voiceName("cs-CZ-Chirp3-HD-Achernar")).toBe("Achernar");
    expect(voiceName("gemini_flash_lite:cs-CZ:Kore")).toBe("Kore");
    expect(voiceName("x", [{ id: "x", display_name: "Sarah" } as never])).toBe("Sarah");
  });
});
