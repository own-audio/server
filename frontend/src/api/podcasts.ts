// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type {
  PodcastCategory,
  PodcastFeed,
  PodcastEpisode,
  PodcastSearchResult,
  Visibility,
} from "./types";

export async function listFeeds(): Promise<PodcastFeed[]> {
  const { data } = await api.get<PodcastFeed[]>("/podcasts");
  return data;
}

export async function subscribeFeed(feed_url: string, visibility: Visibility = "private"): Promise<PodcastFeed> {
  const { data } = await api.post<PodcastFeed>("/podcasts/subscribe", { feed_url, visibility });
  return data;
}

/**
 * Search the self-hosted Podcast Index catalogue. Nothing about the query
 * leaves the deployment — it used to be proxied to a third-party host.
 *
 * `language` is a subtag (`en`, not `en-US`); omitted means no filter.
 */
export async function searchPodcasts(
  query: string,
  language?: string
): Promise<PodcastSearchResult[]> {
  const { data } = await api.post<PodcastSearchResult[]>("/podcasts/search", {
    q: query,
    ...(language ? { language } : {}),
  });
  return data;
}

/** The catalogue's categories, with how many live shows each holds. */
export async function listCategories(): Promise<PodcastCategory[]> {
  const { data } = await api.get<PodcastCategory[]>("/podcasts/discover/categories");
  return data;
}

/** Best shows in one category. Already filtered against what the household
    follows, server-side, so the list never offers back what you have. */
export async function browseCategory(
  category: string,
  language?: string,
  offset = 0
): Promise<PodcastSearchResult[]> {
  const { data } = await api.get<PodcastSearchResult[]>("/podcasts/discover/browse", {
    params: { category, offset, ...(language ? { language } : {}) },
  });
  return data;
}

/**
 * Shows like this one. Empty (not an error) when the feed has no
 * catalogue entry — the catalogue is a weekly snapshot and may simply
 * not know a new show.
 *
 * Carries no listening history in either direction: the catalogue answers
 * "what is like this feed", and it is asked because the user opened the
 * feed, not because anything watched them.
 */
export async function similarFeeds(feedId: string): Promise<PodcastSearchResult[]> {
  const { data } = await api.get<PodcastSearchResult[]>(`/podcasts/${feedId}/similar`);
  return data;
}

export async function getFeed(id: string): Promise<PodcastFeed> {
  const { data } = await api.get<PodcastFeed>(`/podcasts/${id}`);
  return data;
}

export async function deleteFeed(id: string): Promise<void> {
  await api.delete(`/podcasts/${id}`);
}

export async function setFeedVisibility(id: string, visibility: Visibility): Promise<void> {
  await api.put(`/podcasts/${id}/visibility`, { visibility });
}

export async function refreshFeed(id: string): Promise<void> {
  await api.post(`/podcasts/${id}/refresh`);
}

export async function syncImages(id: string): Promise<PodcastFeed> {
  const { data } = await api.post<PodcastFeed>(`/podcasts/${id}/sync-images`);
  return data;
}

export async function listEpisodes(
  feedId: string,
  limit = 50,
  offset = 0,
  q?: string
): Promise<PodcastEpisode[]> {
  const { data } = await api.get<PodcastEpisode[]>(`/podcasts/${feedId}/episodes`, {
    // Searched server-side (title and description, accents ignored), so it reaches
    // episodes the first page never loaded.
    params: q ? { limit, offset, q } : { limit, offset },
  });
  return data;
}

/** The server stores every episode published from now on. */
export async function setAutoStore(feedId: string, enabled: boolean): Promise<PodcastFeed> {
  const { data } = await api.put<PodcastFeed>(`/podcasts/${feedId}/auto-store`, { enabled });
  return data;
}

export interface StoreAllResult {
  episodes: number;
  /** Rough, from durations. */
  estimated_bytes: number;
}

/** Stores the back catalogue on the server — all, or the newest `latest` — so a paid feed
    stays in the library after it ends. `preview` only counts and sizes. */
export async function storeAll(feedId: string, preview: boolean, latest?: number): Promise<StoreAllResult> {
  const { data } = await api.post<StoreAllResult>(`/podcasts/${feedId}/store-all`, { preview, latest });
  return data;
}

export async function downloadEpisode(
  feedId: string,
  epId: string
): Promise<PodcastEpisode> {
  const { data } = await api.post<PodcastEpisode>(
    `/podcasts/${feedId}/episodes/${epId}/download`
  );
  return data;
}

export async function getStreamUrl(
  feedId: string,
  epId: string
): Promise<string> {
  const { data } = await api.get<{ url: string; expires_in_secs: number }>(
    `/podcasts/${feedId}/episodes/${epId}/stream`
  );
  return data.url;
}
