// SPDX-License-Identifier: AGPL-3.0-or-later
import { afterEach, describe, expect, it, vi } from "vitest";
import { AxiosError, AxiosHeaders } from "axios";

const get = vi.fn();
const post = vi.fn();
vi.mock("./client", () => ({
  default: { get: (...a: unknown[]) => get(...a), post: (...a: unknown[]) => post(...a) },
}));

const { createGenerationJob, getQuote, listVoiceProfiles, listVoiceTiers, setJobCover } = await import("./generation");
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
    post.mockResolvedValue({ data: {}, status: 201 });
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

describe("translate a book", () => {
  it("sends both languages in the quote and the job", async () => {
    post.mockResolvedValue({ data: { id: "j1" } });
    await getQuote(5000, "en", "cs", "cs-CZ-Chirp3-HD-Aoede", "book", "chirp3_hd");
    expect(post.mock.calls[0][1]).toMatchObject({ source_language: "en", target_language: "cs", cover_mode: "book" });
    await createGenerationJob({
      file: new File(["x"], "book.epub"),
      title: "Kniha",
      sourceLanguage: "en",
      targetLanguage: "cs",
      voiceProfileId: "cs-CZ-Chirp3-HD-Aoede",
      outputMode: "single_m4b",
      coverMode: "book",
      coverShowTitle: false,
    });
    const form = post.mock.calls[1][1] as FormData;
    expect(form.get("source_language")).toBe("en");
    expect(form.get("target_language")).toBe("cs");
    expect(form.get("cover_mode")).toBe("book");
    expect(form.has("cover_show_title")).toBe(false);
  });
});

describe("cover on a finished narration", () => {
  it("posts each mode with only the fields it uses", async () => {
    post.mockResolvedValue({ data: { id: "j1" } });
    await setJobCover("j1", { coverMode: "auto", coverShowTitle: true });
    await setJobCover("j1", { coverMode: "custom", coverPrompt: "  A red fox  ", coverShowTitle: false });
    await setJobCover("j1", { coverMode: "book", coverShowTitle: true });
    await setJobCover("j1", { coverMode: "none" });
    expect(post.mock.calls[0]).toEqual(["/audiobook-gen/jobs/j1/cover", { cover_mode: "auto", cover_show_title: true }]);
    expect(post.mock.calls[1][1]).toEqual({ cover_mode: "custom", cover_prompt: "A red fox", cover_show_title: false });
    // The title overlay is for a generated cover only.
    expect(post.mock.calls[2][1]).toEqual({ cover_mode: "book", cover_show_title: false });
    expect(post.mock.calls[3][1]).toEqual({ cover_mode: "none", cover_show_title: false });
  });
});

describe("episode translation already made", () => {
  it("tells a 200 (made before, not charged again) from a 201", async () => {
    post.mockResolvedValueOnce({ data: { id: "t1" }, status: 201 });
    post.mockResolvedValueOnce({ data: { id: "t0" }, status: 200 });
    expect((await createTranslation("e1", "cs", "v")).alreadyMade).toBe(false);
    const again = await createTranslation("e1", "cs", "v");
    expect(again).toEqual({ translation: { id: "t0" }, alreadyMade: true });
  });
});
