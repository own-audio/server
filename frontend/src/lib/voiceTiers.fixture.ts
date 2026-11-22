// SPDX-License-Identifier: AGPL-3.0-or-later
import type { VoiceTier } from "../api/types";

const lang = (language: string, price: number, sample_url?: string) => ({
  language,
  price_per_hour_cents: price,
  price_per_million_chars_cents: price * 10,
  sample_url,
});

export const TIERS: VoiceTier[] = [
  { id: "chirp3_hd", label: "Chirp 3: HD", model: null, preview: false, default: true, languages: [lang("en", 380), lang("cs", 380), lang("sk", 380)] },
  { id: "gemini_flash", label: "Gemini 3.8 Flash TTS (Preview)", model: "gemini-3.8-flash-tts", preview: true, default: false, languages: [lang("en", 160), lang("cs", 190)] },
  { id: "gemini_flash_lite", label: "Gemini 3.8 Flash-Lite TTS (Preview)", model: "gemini-3.8-flash-lite-tts", preview: true, default: false, languages: [lang("en", 110, "https://cdn.example/en-lite.mp3"), lang("cs", 120)] },
];
