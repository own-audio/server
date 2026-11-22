// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { TIERS } from "./voiceTiers.fixture";
import { effectiveTier, sampleUrl, showsTierPicker, tierName, tierOfVoice, voiceForTier } from "./voiceTiers";

describe("voice tiers", () => {
  it("builds the website's sample link unless the server sends one", () => {
    expect(sampleUrl(TIERS[0], "cs")).toBe("https://www.own.audio/samples/voices/cs-chirp3-hd.mp3");
    expect(sampleUrl(TIERS[1], "en-US")).toBe("https://www.own.audio/samples/voices/en-gemini-flash.mp3");
    expect(sampleUrl(TIERS[2], "cs")).toBe("https://www.own.audio/samples/voices/cs-gemini-flash-lite.mp3");
    expect(sampleUrl(TIERS[2], "en")).toBe("https://cdn.example/en-lite.mp3");
  });

  it("shows the name without the Preview suffix, which is a badge", () => {
    expect(tierName(TIERS[1])).toBe("Gemini 3.8 Flash TTS");
    expect(tierName(TIERS[0])).toBe("Chirp 3: HD");
  });

  it("sends no tier to a server without tiers", () => {
    expect(effectiveTier([], "gemini_flash", "en")).toBeUndefined();
    expect(showsTierPicker([])).toBe(false);
    expect(showsTierPicker(TIERS.slice(0, 1))).toBe(false);
    expect(showsTierPicker(TIERS)).toBe(true);
  });

  it("starts on the default tier and keeps a chosen one while it reads the language", () => {
    expect(effectiveTier(TIERS, null, "en")).toBe("chirp3_hd");
    expect(effectiveTier(TIERS, "gemini_flash", "en")).toBe("gemini_flash");
    // No Slovak Gemini voices: back to Chirp.
    expect(effectiveTier(TIERS, "gemini_flash", "sk")).toBe("chirp3_hd");
    expect(effectiveTier(TIERS, "unknown", "en")).toBe("chirp3_hd");
  });

  it("reads a voice's tier from its id and drops a voice of another tier", () => {
    expect(tierOfVoice("gemini_flash:cs-CZ:Aoede")).toBe("gemini_flash");
    expect(tierOfVoice("gemini_flash_lite:en-US:Charon")).toBe("gemini_flash_lite");
    expect(tierOfVoice("cs-CZ-Chirp3-HD-Aoede")).toBe("chirp3_hd");
    expect(voiceForTier("gemini_flash:cs-CZ:Aoede", "chirp3_hd")).toBeNull();
    expect(voiceForTier("gemini_flash:cs-CZ:Aoede", "gemini_flash")).toBe("gemini_flash:cs-CZ:Aoede");
    expect(voiceForTier("cs-CZ-Chirp3-HD-Aoede", undefined)).toBe("cs-CZ-Chirp3-HD-Aoede");
  });
});
