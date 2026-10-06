// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type {
  AudiobookCollection,
  AudiobookSeries,
  CollectionBookEntry,
} from "./types";

// ── Collections ─────────────────────────────────────────────────────────────

export async function listCollections(): Promise<AudiobookCollection[]> {
  const { data } = await api.get<AudiobookCollection[]>(
    "/audiobooks/organize/collections"
  );
  return data;
}

export async function getCollection(id: string): Promise<AudiobookCollection> {
  const { data } = await api.get<AudiobookCollection>(
    `/audiobooks/organize/collections/${id}`
  );
  return data;
}

export interface CreateCollectionRequest {
  name: string;
  description?: string;
  is_public?: boolean;
}

export async function createCollection(
  req: CreateCollectionRequest
): Promise<AudiobookCollection> {
  const { data } = await api.post<AudiobookCollection>(
    "/audiobooks/organize/collections",
    req
  );
  return data;
}

export async function updateCollection(
  id: string,
  req: CreateCollectionRequest
): Promise<AudiobookCollection> {
  const { data } = await api.put<AudiobookCollection>(
    `/audiobooks/organize/collections/${id}`,
    req
  );
  return data;
}

export async function deleteCollection(id: string): Promise<void> {
  await api.delete(`/audiobooks/organize/collections/${id}`);
}

export async function listCollectionBooks(
  id: string
): Promise<CollectionBookEntry[]> {
  const { data } = await api.get<CollectionBookEntry[]>(
    `/audiobooks/organize/collections/${id}/books`
  );
  return data;
}

export async function addBookToCollection(
  collectionId: string,
  bookId: string
): Promise<void> {
  await api.post(`/audiobooks/organize/collections/${collectionId}/books`, {
    book_id: bookId,
  });
}

export async function removeBookFromCollection(
  collectionId: string,
  bookId: string
): Promise<void> {
  await api.delete(
    `/audiobooks/organize/collections/${collectionId}/books/${bookId}`
  );
}

// ── Series ──────────────────────────────────────────────────────────────────

export async function listSeries(): Promise<AudiobookSeries[]> {
  const { data } = await api.get<AudiobookSeries[]>(
    "/audiobooks/organize/series"
  );
  return data;
}

export async function getSeries(id: string): Promise<AudiobookSeries> {
  const { data } = await api.get<AudiobookSeries>(
    `/audiobooks/organize/series/${id}`
  );
  return data;
}

export interface CreateSeriesRequest {
  name: string;
  description?: string;
}

export async function createSeries(
  req: CreateSeriesRequest
): Promise<AudiobookSeries> {
  const { data } = await api.post<AudiobookSeries>(
    "/audiobooks/organize/series",
    req
  );
  return data;
}

export async function updateSeries(
  id: string,
  req: CreateSeriesRequest
): Promise<AudiobookSeries> {
  const { data } = await api.put<AudiobookSeries>(
    `/audiobooks/organize/series/${id}`,
    req
  );
  return data;
}

export async function deleteSeries(id: string): Promise<void> {
  await api.delete(`/audiobooks/organize/series/${id}`);
}

export async function addBookToSeries(
  seriesId: string,
  bookId: string,
  position: number
): Promise<void> {
  await api.post(`/audiobooks/organize/series/${seriesId}/books`, {
    book_id: bookId,
    position,
  });
}

export async function removeBookFromSeries(
  seriesId: string,
  bookId: string
): Promise<void> {
  await api.delete(`/audiobooks/organize/series/${seriesId}/books/${bookId}`);
}

// ── Favorites ───────────────────────────────────────────────────────────────

export async function listFavorites(): Promise<CollectionBookEntry[]> {
  const { data } = await api.get<CollectionBookEntry[]>(
    "/audiobooks/organize/favorites"
  );
  return data;
}

export async function addFavorite(bookId: string): Promise<void> {
  await api.post(`/audiobooks/organize/favorites/${bookId}`);
}

export async function removeFavorite(bookId: string): Promise<void> {
  await api.delete(`/audiobooks/organize/favorites/${bookId}`);
}

export async function checkFavorite(
  bookId: string
): Promise<{ is_favorite: boolean }> {
  const { data } = await api.get<{ is_favorite: boolean }>(
    `/audiobooks/organize/favorites/${bookId}/check`
  );
  return data;
}
