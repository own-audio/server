// SPDX-License-Identifier: AGPL-3.0-or-later
// Episode summaries and questions (hosted edition; `features.summaries`), on /api/v2/summaries.
import api, { apiBaseUrl } from "./client";

/** The v2 API beside v1: same origin, same session. */
const v2 = { baseURL: apiBaseUrl.replace(/\/api\/v1\/?$/, "/api/v2") };

export interface EpisodeSummary {
  episode_id: string;
  language: string;
  text: string;
  charged_micro: number;
  created_at: string;
}

export interface AskTurn {
  question: string;
  answer: string;
}

/** The household's summary in this language, or null when none has been made. */
export async function getSummary(episodeId: string, language: string): Promise<EpisodeSummary | null> {
  try {
    const { data } = await api.get<EpisodeSummary>(`/summaries/episodes/${episodeId}/summary`, { ...v2, params: { language } });
    return data;
  } catch (err) {
    if ((err as { response?: { status?: number } })?.response?.status === 404) return null;
    throw err;
  }
}

/** Made now and paid from the family's credit, or the one already made. */
export async function createSummary(episodeId: string, language: string): Promise<EpisodeSummary> {
  const { data } = await api.post<EpisodeSummary>(`/summaries/episodes/${episodeId}/summary`, { language }, v2);
  return data;
}

/** An answer from the transcript, paid; earlier turns go along for context. */
export async function askEpisode(episodeId: string, question: string, history: AskTurn[]): Promise<string> {
  const { data } = await api.post<{ answer: string }>(`/summaries/episodes/${episodeId}/ask`, { question, history }, v2);
  return data.answer;
}
