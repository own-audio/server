// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type {
  AudiobookAuthor,
  BookAuthor,
  AudiobookTag,
} from "./types";

// ── Authors ─────────────────────────────────────────────────────────────────

export async function listAuthors(): Promise<AudiobookAuthor[]> {
  const { data } = await api.get<AudiobookAuthor[]>("/audiobooks/authors");
  return data;
}

export async function getAuthor(id: string): Promise<AudiobookAuthor> {
  const { data } = await api.get<AudiobookAuthor>(`/audiobooks/authors/${id}`);
  return data;
}

export interface CreateAuthorRequest {
  name: string;
  sort_name?: string;
  bio?: string;
}

export async function createAuthor(req: CreateAuthorRequest): Promise<AudiobookAuthor> {
  const { data } = await api.post<AudiobookAuthor>("/audiobooks/authors", req);
  return data;
}

export async function updateAuthor(
  id: string,
  req: CreateAuthorRequest
): Promise<AudiobookAuthor> {
  const { data } = await api.put<AudiobookAuthor>(`/audiobooks/authors/${id}`, req);
  return data;
}

export async function deleteAuthor(id: string): Promise<void> {
  await api.delete(`/audiobooks/authors/${id}`);
}

/** The user's own picture for this author — permanent, the automatic
 *  Wikimedia lookup never overwrites it once set. */
export async function uploadAuthorImage(id: string, file: File): Promise<void> {
  const formData = new FormData();
  formData.append("image", file, file.name);
  await api.post(`/audiobooks/authors/${id}/image`, formData);
}

/** Forgets this author's picture, user-set or fetched, so the automatic
 *  lookup runs fresh next time it's requested. */
export async function deleteAuthorImage(id: string): Promise<void> {
  await api.delete(`/audiobooks/authors/${id}/image`);
}

export async function getAuthorBooks(
  authorId: string
): Promise<
  Array<{
    id: string;
    title: string;
    author: string | null;
    cover_url: string | null;
    total_duration_secs: number | null;
    role: string;
  }>
> {
  const { data } = await api.get(`/audiobooks/authors/${authorId}/books`);
  return data;
}

// ── Book ↔ Author links ────────────────────────────────────────────────────

export async function listBookAuthors(bookId: string): Promise<BookAuthor[]> {
  const { data } = await api.get<BookAuthor[]>(`/audiobooks/authors/book/${bookId}`);
  return data;
}

export async function linkBookAuthor(
  bookId: string,
  authorId: string,
  role = "author"
): Promise<void> {
  await api.post(`/audiobooks/authors/book/${bookId}`, {
    author_id: authorId,
    role,
  });
}

export async function unlinkBookAuthor(
  bookId: string,
  authorId: string,
  role: string
): Promise<void> {
  await api.delete(`/audiobooks/authors/book/${bookId}/${authorId}/${role}`);
}

// ── Tags ────────────────────────────────────────────────────────────────────

export async function listAllTags(): Promise<AudiobookTag[]> {
  const { data } = await api.get<AudiobookTag[]>("/audiobooks/authors/tags");
  return data;
}

export async function listBookTags(bookId: string): Promise<AudiobookTag[]> {
  const { data } = await api.get<AudiobookTag[]>(`/audiobooks/authors/tags/book/${bookId}`);
  return data;
}

export async function setBookTags(bookId: string, tags: string[]): Promise<AudiobookTag[]> {
  const { data } = await api.put<AudiobookTag[]>(`/audiobooks/authors/tags/book/${bookId}`, {
    tags,
  });
  return data;
}
