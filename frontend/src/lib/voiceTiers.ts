// SPDX-License-Identifier: AGPL-3.0-or-later
import type { VoiceTier, VoiceTierId, VoiceTierLanguage } from "../api/types";

const SAMPLE_BASE = "https://www.own.audio/samples/voices";

const base = (language: string) => language.slice(0, 2).toLowerCase();

/** The tier's price and sample for a language, or none when the tier has no voice in it. */
export function tierLanguage(tier: VoiceTier, language: string): VoiceTierLanguage | undefined {
  return tier.languages.find((l) => base(l.language) === base(language));
}

/** The server's sample link when it sends one, else the website's file of the same name. */
export function sampleUrl(tier: VoiceTier, language: string): string {
  const own = tierLanguage(tier, language)?.sample_url;
  if (own) return own;
  return `${SAMPLE_BASE}/${base(language)}-${tier.id.replace(/_/g, "-")}.mp3`;
}

/** The server's label without its "(Preview)": the card shows that as a badge. */
export function tierName(tier: VoiceTier): string {
  return tier.label.replace(/\s*\(Preview\)\s*$/i, "");
}

export function defaultTier(tiers: VoiceTier[]): VoiceTier | undefined {
  return tiers.find((t) => t.default) ?? tiers[0];
}

/**
 * The tier a request goes out with. None on a server without tiers, so nothing new is sent to it;
 * the default when the chosen tier is gone or reads no voice in this language.
 */
export function effectiveTier(tiers: VoiceTier[], chosen: VoiceTierId | null, language: string): VoiceTierId | undefined {
  if (tiers.length === 0) return undefined;
  const picked = tiers.find((t) => t.id === chosen);
  if (picked && tierLanguage(picked, language)) return picked.id;
  return defaultTier(tiers)?.id;
}

/** A picker is worth showing only when there is more than one tier to choose from. */
export const showsTierPicker = (tiers: VoiceTier[]) => tiers.length > 1;

/** The tier a voice belongs to, read from its id as the server reads it: a Gemini voice is
 *  `<tier>:<locale>:<voice>`, every other voice is Chirp's. */
export function tierOfVoice(voiceId: string): VoiceTierId {
  const [head] = voiceId.split(":");
  if (voiceId.includes(":") && (head === "gemini_flash" || head === "gemini_flash_lite")) return head;
  return "chirp3_hd";
}

/** The picked voice while it still belongs to the tier in use; none once the tier moved away. */
export function voiceForTier(voiceId: string | null, tier: VoiceTierId | undefined): string | null {
  if (!voiceId) return null;
  if (!tier) return voiceId;
  return tierOfVoice(voiceId) === tier ? voiceId : null;
}
