// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import { batchHeaders } from "./trash";
import type { AudioBook, AudioBookFile, AudioBookChapter, Visibility } from "./types";

export async function listBooks(): Promise<AudioBook[]> {
  const { data } = await api.get<AudioBook[]>("/audiobooks");
  return data;
}

export interface CreateBookRequest {
  title: string;
  author?: string;
  narrator?: string;
  description?: string;
  source_url?: string;
}

export async function createBook(req: CreateBookRequest): Promise<AudioBook> {
  const { data } = await api.post<AudioBook>("/audiobooks", req);
  return data;
}

export interface UpdateBookRequest {
  title: string;
  author?: string;
  narrator?: string;
  description?: string;
}

export interface UploadManifestEntry {
  relative_path?: string;
  duration_secs?: number;
}

export interface UploadBookRequest {
  title?: string;
  author?: string;
  narrator?: string;
  description?: string;
  manifest: UploadManifestEntry[];
  files: File[];
  cover?: File | null;
}

export async function uploadBook(req: UploadBookRequest): Promise<AudioBook> {
  const formData = new FormData();

  if (req.title?.trim()) formData.append("title", req.title.trim());
  if (req.author?.trim()) formData.append("author", req.author.trim());
  if (req.narrator?.trim()) formData.append("narrator", req.narrator.trim());
  if (req.description?.trim()) formData.append("description", req.description.trim());
  formData.append("manifest", JSON.stringify(req.manifest));

  req.files.forEach((file) => {
    formData.append("files", file, file.name);
  });

  if (req.cover) {
    formData.append("cover", req.cover, req.cover.name);
  }

  const { data } = await api.post<AudioBook>("/audiobooks/upload", formData);
  return data;
}

export interface UploadBookFileRequest {
  position: number;
  relativePath?: string;
  durationSecs?: number;
  file: File;
}

export async function uploadBookFile(
  bookId: string,
  req: UploadBookFileRequest
): Promise<void> {
  const formData = new FormData();
  formData.append("position", String(req.position));
  if (req.relativePath) formData.append("relative_path", req.relativePath);
  if (req.durationSecs != null) formData.append("duration_secs", String(req.durationSecs));
  formData.append("file", req.file, req.file.name);
  await api.post(`/audiobooks/${bookId}/upload-file`, formData);
}

export interface FromUploadsFile {
  object_key: string;
  relative_path?: string;
  title?: string;
  duration_secs?: number;
}

export interface CreateBookFromUploadsRequest {
  title?: string;
  author?: string;
  narrator?: string;
  description?: string;
  visibility?: string;
  cover_object_key?: string;
  files: FromUploadsFile[];
}

/**
 * Create a book from objects already PUT straight to storage.
 *
 * Preferred over `uploadBook`/`uploadBookFile` in production: those send the
 * bytes through the API, which sits behind a proxy that caps request bodies at
 * 100 MB.
 */
export async function createBookFromUploads(
  req: CreateBookFromUploadsRequest
): Promise<AudioBook> {
  const { data } = await api.post<AudioBook>("/audiobooks/from-uploads", req);
  return data;
}

export async function uploadBookCover(bookId: string, file: File): Promise<void> {
  const formData = new FormData();
  formData.append("cover", file, file.name);
  await api.post(`/audiobooks/${bookId}/upload-cover`, formData);
}

export async function getBook(id: string): Promise<AudioBook> {
  const { data } = await api.get<AudioBook>(`/audiobooks/${id}`);
  return data;
}

export async function updateBook(
  id: string,
  req: UpdateBookRequest
): Promise<AudioBook> {
  const { data } = await api.put<AudioBook>(`/audiobooks/${id}`, req);
  return data;
}

// ── Identify (Google Books) ───────────────────────────────────────────────

export interface BookCandidate {
  volume_id: string;
  title: string;
  subtitle: string | null;
  author: string | null;
  publisher: string | null;
  published_year: number | null;
  description: string | null;
  isbn: string | null;
  page_count: number | null;
  categories: string[];
  /** Google's own image, already upgraded past the 128px thumbnail server-side.
   *  Can still 404 — a broken image here is normal, not an error. */
  cover_url: string | null;
  /** 0–100, computed by our backend. Google returns relevance order but no score. */
  score: number;
}

/** The book id scopes visibility only — the search runs on what is passed, so a
 *  book whose stored title is wrong can still be found. At least one of the two
 *  is required or the server answers 400. */
export async function searchBookMetadata(
  bookId: string,
  req: { title?: string; author?: string; limit?: number }
): Promise<BookCandidate[]> {
  const { data } = await api.post<BookCandidate[]>(`/audiobooks/${bookId}/metadata/search`, req);
  return data;
}

/** Which parts of a match to write. Omitted fields default to true server-side. */
export interface IdentifyFields {
  title?: boolean;
  author?: boolean;
  description?: boolean;
  publisher?: boolean;
  published_year?: boolean;
  isbn?: boolean;
  cover?: boolean;
}

/** Only the volume id travels — the server re-fetches the volume rather than
 *  trusting the candidate this client is holding. Unpicked fields keep their
 *  current value; nothing is cleared. */
export async function applyBookMetadata(
  bookId: string,
  req: { volume_id: string; fields?: IdentifyFields }
): Promise<AudioBook> {
  const { data } = await api.post<AudioBook>(`/audiobooks/${bookId}/metadata/apply`, req);
  return data;
}

/** Move a book between the private and family folders. Owner only. */
export async function setBookVisibility(id: string, visibility: Visibility): Promise<void> {
  await api.put(`/audiobooks/${id}/visibility`, { visibility });
}

/** Moves the book to the trash; `batch` groups one gesture — see `api/trash.ts`. */
export async function deleteBook(id: string, batch?: string): Promise<void> {
  await api.delete(`/audiobooks/${id}`, batchHeaders(batch));
}

export async function listBookFiles(id: string): Promise<AudioBookFile[]> {
  const { data } = await api.get<AudioBookFile[]>(`/audiobooks/${id}/files`);
  return data;
}

/**
 * Owner, or a family admin when the book is shared with their family — same rule as identifying
 * it. 400 on an empty title; the server has no way to turn a title back into the "Part N"
 * fallback other than typing that literally, so clearing the field client-side just reverts
 * rather than sending an empty string.
 */
export async function updateFileTitle(bookId: string, fileId: string, title: string): Promise<AudioBookFile> {
  const { data } = await api.put<AudioBookFile>(`/audiobooks/${bookId}/files/${fileId}`, { title });
  return data;
}

export async function listBookChapters(id: string): Promise<AudioBookChapter[]> {
  const { data } = await api.get<AudioBookChapter[]>(`/audiobooks/${id}/chapters`);
  return data;
}

export async function getBookFileStreamUrl(
  bookId: string,
  fileId: string
): Promise<string> {
  const { data } = await api.get<{ url: string }>(`/audiobooks/${bookId}/files/${fileId}/stream`);
  return data.url;
}

export async function reorderBookFiles(
  bookId: string,
  fileIds: string[]
): Promise<AudioBookFile[]> {
  const { data } = await api.put<AudioBookFile[]>(
    `/audiobooks/${bookId}/files/reorder`,
    { file_ids: fileIds }
  );
  return data;
}

/** Removes one file from a multi-file book — for a botched or duplicate upload, not a way to
 *  delete the whole book (that's `deleteBook`, which goes through the trash). Hard delete: the
 *  server doesn't put stray files through the 30-day trash. */
export async function deleteBookFile(bookId: string, fileId: string): Promise<void> {
  await api.delete(`/audiobooks/${bookId}/files/${fileId}`);
}
