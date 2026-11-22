// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";
import { AxiosError, AxiosHeaders } from "axios";

const get = vi.fn();
const post = vi.fn();
vi.mock("./client", () => ({
  default: { get: (...a: unknown[]) => get(...a), post: (...a: unknown[]) => post(...a) },
}));

const { createGenerationJob, getQuote, listVoiceProfiles, listVoiceTiers } = await import("./generation");
const { createTranslation, quoteTranslation } = await import("./podcastTranslate");

afterEach(() => {
  get.mockReset();
  post.mockReset();
});

const httpError = (status: number) =>
  new AxiosError("x", String(status), undefined, undefined, {
    status,
    statusText: "",
    headers: {},
    config: { headers: new AxiosHeaders() },
    data: {},
  });

describe("voice tier requests", () => {
  it("reads no tiers from a server without them", async () => {
    get.mockRejectedValueOnce(httpError(501));
    expect(await listVoiceTiers()).toEqual([]);
    get.mockRejectedValueOnce(httpError(404));
    expect(await listVoiceTiers()).toEqual([]);
    get.mockRejectedValueOnce(httpError(500));
    await expect(listVoiceTiers()).rejects.toBeInstanceOf(AxiosError);
    expect(get).toHaveBeenCalledWith("/audiobook-gen/voice-tiers");
  });

  it("asks for a tier's voices only when one is named", async () => {
    get.mockResolvedValue({ data: [] });
    await listVoiceProfiles();
    await listVoiceProfiles("gemini_flash");
    expect(get).toHaveBeenNthCalledWith(1, "/audiobook-gen/voices");
    expect(get).toHaveBeenNthCalledWith(2, "/audiobook-gen/voices", { params: { tier: "gemini_flash" } });
  });

  it("sends voice_tier in the narration quote, and nothing new without one", async () => {
    post.mockResolvedValue({ data: {} });
    await getQuote(1000, "en", "cs", "gemini_flash:cs-CZ:Aoede", "auto", "gemini_flash");
    await getQuote(1000, "en", "en", "en-US-Chirp3-HD-Aoede", "none");
    expect(post.mock.calls[0]).toEqual([
      "/audiobook-gen/quote",
      { char_count: 1000, source_language: "en", target_language: "cs", voice_profile_id: "gemini_flash:cs-CZ:Aoede", cover_mode: "auto", voice_tier: "gemini_flash" },
    ]);
    expect(post.mock.calls[1][1]).not.toHaveProperty("voice_tier");
  });

  it("sends voice_tier in the narration job form", async () => {
    post.mockResolvedValue({ data: { id: "j1" } });
    const base = {
      file: new File(["text"], "book.txt"),
      title: "Book",
      sourceLanguage: "en",
      targetLanguage: "en",
      voiceProfileId: "gemini_flash_lite:en-US:Aoede",
      outputMode: "single_m4b" as const,
      coverMode: "auto" as const,
      coverShowTitle: false,
    };
    await createGenerationJob({ ...base, voiceTier: "gemini_flash_lite" });
    await createGenerationJob({ ...base, voiceProfileId: "en-US-Chirp3-HD-Aoede" });
    const [path, form] = post.mock.calls[0] as [string, FormData];
    expect(path).toBe("/audiobook-gen/jobs");
    expect(form.get("voice_tier")).toBe("gemini_flash_lite");
    expect(form.get("voice_profile_id")).toBe("gemini_flash_lite:en-US:Aoede");
    expect((post.mock.calls[1][1] as FormData).has("voice_tier")).toBe(false);
  });

  it("sends voice_tier in the episode translation quote and request", async () => {
    post.mockResolvedValue({ data: {} });
    await quoteTranslation("e1", "cs", "gemini_flash:cs-CZ:Charon", undefined, "gemini_flash");
    await createTranslation("e1", "cs", "gemini_flash:cs-CZ:Charon", undefined, "gemini_flash");
    await createTranslation("e1", "cs", "cs-CZ-Chirp3-HD-Charon");
    expect(post.mock.calls[0]).toEqual([
      "/podcast-translate/episodes/e1/quote",
      { target_language: "cs", voice_profile_id: "gemini_flash:cs-CZ:Charon", source_language: undefined, voice_tier: "gemini_flash" },
    ]);
    expect(post.mock.calls[1][0]).toBe("/podcast-translate/episodes/e1");
    expect(post.mock.calls[1][1]).toMatchObject({ voice_tier: "gemini_flash" });
    expect(post.mock.calls[2][1]).not.toHaveProperty("voice_tier");
  });
});

describe("cover", () => {
  it("offers the book's own cover only for a file that can carry one", async () => {
    const { canHaveOwnCover } = await import("./generation");
    expect(canHaveOwnCover(new File([""], "Book.EPUB"))).toBe(true);
    expect(canHaveOwnCover(new File([""], "book.mobi"))).toBe(true);
    expect(canHaveOwnCover(new File([""], "book.txt"))).toBe(false);
    expect(canHaveOwnCover(null)).toBe(false);
  });
});
