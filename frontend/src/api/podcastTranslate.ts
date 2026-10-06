// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { PodcastEpisodeTranslation, PodcastTranslationQuote, RecentPodcastTranslation } from "./types";

export async function quoteTranslation(
  episodeId: string,
  targetLanguage: string,
  voiceProfileId: string,
  sourceLanguage?: string
): Promise<PodcastTranslationQuote> {
  const { data } = await api.post<PodcastTranslationQuote>(`/podcast-translate/episodes/${episodeId}/quote`, {
    target_language: targetLanguage,
    voice_profile_id: voiceProfileId,
    source_language: sourceLanguage,
  });
  return data;
}

export async function createTranslation(
  episodeId: string,
  targetLanguage: string,
  voiceProfileId: string,
  sourceLanguage?: string
): Promise<PodcastEpisodeTranslation> {
  const { data } = await api.post<PodcastEpisodeTranslation>(`/podcast-translate/episodes/${episodeId}`, {
    target_language: targetLanguage,
    voice_profile_id: voiceProfileId,
    source_language: sourceLanguage,
  });
  return data;
}

export async function listTranslations(episodeId: string): Promise<PodcastEpisodeTranslation[]> {
  const { data } = await api.get<PodcastEpisodeTranslation[]>(`/podcast-translate/episodes/${episodeId}`);
  return data;
}

/** Every translation of a show's episodes, from one request. */
export async function listFeedTranslations(feedId: string): Promise<PodcastEpisodeTranslation[]> {
  const { data } = await api.get<PodcastEpisodeTranslation[]>(`/podcast-translate/feeds/${feedId}`);
  return data;
}

/** The family's latest translations across shows, with episode and show names. */
export async function listRecentTranslations(): Promise<RecentPodcastTranslation[]> {
  const { data } = await api.get<RecentPodcastTranslation[]>("/podcast-translate/recent");
  return data;
}
