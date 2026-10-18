// SPDX-License-Identifier: AGPL-3.0-or-later
// Episode summaries and questions (hosted edition; `features.summaries`), on /api/v2/summaries.
import api, { apiBaseUrl } from "./client";

/** The v2 API beside v1: same origin, same session. */
const v2 = { baseURL: apiBaseUrl.replace(/\/api\/v1\/?$/, "/api/v2") };

export type SummaryLength = "short" | "medium" | "long";

export interface EpisodeSummary {
  episode_id: string;
  language: string;
  length: SummaryLength;
  text: string;
  charged_micro: number;
  created_at: string;
}

export interface AskTurn {
  question: string;
  answer: string;
}

/** A turn as the page shows it: with the price the family paid for the answer. */
export interface AskedTurn extends AskTurn {
  charged_micro: number;
}

/** The household's summary in this language and length, or null when none has been made. */
export async function getSummary(episodeId: string, language: string, length: SummaryLength): Promise<EpisodeSummary | null> {
  try {
    const { data } = await api.get<EpisodeSummary>(`/summaries/episodes/${episodeId}/summary`, { ...v2, params: { language, length } });
    return data;
  } catch (err) {
    if ((err as { response?: { status?: number } })?.response?.status === 404) return null;
    throw err;
  }
}

/** Made now and paid from the family's credit, or the one already made. */
export async function createSummary(episodeId: string, language: string, length: SummaryLength): Promise<EpisodeSummary> {
  const { data } = await api.post<EpisodeSummary>(`/summaries/episodes/${episodeId}/summary`, { language, length }, v2);
  return data;
}

/** An answer from the transcript, paid; earlier turns go along for context. */
export async function askEpisode(episodeId: string, question: string, history: AskTurn[]): Promise<AskedTurn> {
  const { data } = await api.post<{ answer: string; charged_micro: number }>(
    `/summaries/episodes/${episodeId}/ask`,
    { question, history: history.map(({ question, answer }) => ({ question, answer })) },
    v2,
  );
  return { question, answer: data.answer, charged_micro: data.charged_micro };
}

/** A price in the ledger's dollars; these are fractions of a cent, so three decimals below 10 ¢. */
export function formatPrice(micro: number, locale: string): string {
  const usd = micro / 1_000_000;
  const digits = usd < 0.1 ? 3 : 2;
  return new Intl.NumberFormat(locale, { style: "currency", currency: "USD", minimumFractionDigits: digits, maximumFractionDigits: digits }).format(usd);
}
