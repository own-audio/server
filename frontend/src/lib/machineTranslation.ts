// SPDX-License-Identifier: AGPL-3.0-or-later
import type { BookOrigin, PodcastEpisodeTranslation, VoiceProfile } from "../api/types";
import { intlLocale, t, type Locale } from "../i18n";

/* Telling an original from a machine translation (`audio2/docs/creation-features-clients.md`
   §9.5). Only what the server says counts: an absent or unknown `origin`, or a translation row
   without `machine_translated`, gets no badge. */

export function isMachineTranslatedBook(book: { origin?: BookOrigin | null } | null | undefined): boolean {
  return book?.origin?.kind === "machine_translated";
}

/** Narrated by an AI voice in the book's own language — the "AI voice" badge. */
export function isAiVoiceBook(book: { origin?: BookOrigin | null } | null | undefined): boolean {
  return book?.origin?.kind === "narrated";
}

export function isMachineTranslatedEpisode(tr: Pick<PodcastEpisodeTranslation, "machine_translated"> | null | undefined): boolean {
  return tr?.machine_translated === true;
}

const baseCode = (code: string) => code.split(/[-_]/)[0].toLowerCase();

function displayName(code: string, locale: Locale): string | null {
  try {
    const name = new Intl.DisplayNames([locale], { type: "language" }).of(code);
    return name && name.toLowerCase() !== code.toLowerCase() ? name : null;
  } catch {
    return null;
  }
}

/** A language standing on its own ("Czech", "Čeština"), in the app's language. */
export function languageName(code: string, locale: Locale = intlLocale()): string {
  const name = displayName(code, locale);
  return name ? name.charAt(0).toLocaleUpperCase(locale) + name.slice(1) : code.toUpperCase();
}

/** A language as it reads after "from … to …" — the genitive in Czech and Slovak ("z češtiny do
 *  angličtiny"), the plain name in the other languages, whose sentences are built around it. */
export function languageInSentence(code: string, locale: Locale = intlLocale()): string {
  const base = baseCode(code);
  const name = displayName(base, locale);
  // The catalog knows the common ones; for another Czech or Slovak name, "-ina" → "-iny" is the
  // genitive of nearly every language (polština → polštiny, maďarčina → maďarčiny). Otherwise the
  // code, as the server does.
  const genitive = locale === "cs" || locale === "sk";
  const fallback = !name ? code : genitive ? (name.endsWith("ina") ? `${name.slice(0, -1)}y` : code) : name;
  return t("common.languageInSentence", { code: base, name: fallback });
}

/** A voice's name: from the voice list when it is there, else read off the id
 *  (`gemini_flash:cs-CZ:Kore`, `cs-CZ-Chirp3-HD-Achernar`). */
export function voiceName(voiceProfileId: string, voices: VoiceProfile[] = []): string {
  const known = voices.find((v) => v.id === voiceProfileId)?.display_name;
  if (known) return known;
  const parts = voiceProfileId.includes(":") ? voiceProfileId.split(":") : voiceProfileId.split("-");
  return parts[parts.length - 1] || voiceProfileId;
}

/** "Machine-translated by Google from Czech to English, read by Sarah." — or null when the book
 *  does not say all three. */
export function bookMachineTranslationLine(book: { language?: string | null; origin?: BookOrigin | null }): string | null {
  const o = book.origin;
  if (!isMachineTranslatedBook(book) || !o?.source_language || !book.language || !o.voice) return null;
  const params = { source: languageInSentence(o.source_language), target: languageInSentence(book.language), voice: o.voice };
  return o.translator === "google" ? t("audiobooks.detail.machineTranslated", params) : t("audiobooks.detail.machineTranslatedNeutral", params);
}

export function episodeMachineTranslationLine(tr: PodcastEpisodeTranslation, voices: VoiceProfile[] = []): string | null {
  if (!isMachineTranslatedEpisode(tr)) return null;
  const params = {
    source: languageInSentence(tr.source_language),
    target: languageInSentence(tr.target_language),
    voice: voiceName(tr.voice_profile_id, voices),
  };
  return tr.translator === "google" ? t("podcasts.translate.machineTranslated", params) : t("podcasts.translate.machineTranslatedNeutral", params);
}

/** "Read by the AI voice Tereza." — or null when the book is not an AI narration or the voice is unknown. */
export function bookAiVoiceLine(book: { origin?: BookOrigin | null }): string | null {
  const voice = book.origin?.voice;
  return isAiVoiceBook(book) && voice ? t("audiobooks.detail.aiVoice", { voice }) : null;
}
