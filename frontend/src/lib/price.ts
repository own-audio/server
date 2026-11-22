// SPDX-License-Identifier: AGPL-3.0-or-later
import type { GenerationQuote, PodcastTranslationQuote } from "../api/types";

const CENT = 10_000;

/**
 * The lines of a paid-work quote, in micro-units. A server that sends the `_price_micro` fields
 * gives prices that add up to the total exactly; an older one only has the providers' costs in
 * cents and a marked-up total, so those are shown as they were.
 */
export interface QuoteLines {
  translation: number;
  narration: number;
  preparation: number;
  cover: number;
  total: number;
}

export function quoteLines(quote: GenerationQuote | PodcastTranslationQuote): QuoteLines {
  if (quote.quoted_price_micro != null) {
    return {
      translation: quote.translation_price_micro ?? 0,
      narration: quote.narration_price_micro ?? 0,
      preparation: quote.preparation_price_micro ?? 0,
      cover: 0,
      total: quote.quoted_price_micro,
    };
  }
  return {
    translation: quote.translation_cost_cents * CENT,
    narration: quote.tts_cost_cents * CENT,
    preparation: 0,
    cover: ("cover_cost_cents" in quote ? quote.cover_cost_cents ?? 0 : 0) * CENT,
    total: quote.quoted_price_cents * CENT,
  };
}

export function quotedMicro(x: { quoted_price_micro?: number; quoted_price_cents: number }): number {
  return x.quoted_price_micro ?? x.quoted_price_cents * CENT;
}

/** Null until charged, from either field. */
export function chargedMicro(x: { charged_price_micro?: number | null; charged_price_cents: number | null }): number | null {
  if (x.charged_price_micro != null) return x.charged_price_micro;
  return x.charged_price_cents == null ? null : x.charged_price_cents * CENT;
}
