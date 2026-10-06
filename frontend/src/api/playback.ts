// SPDX-License-Identifier: AGPL-3.0-or-later
import client from "./client";

export interface ProgressResponse {
  media_id: string;
  position_secs: number;
  completed: boolean;
  updated_at: string;
  file_id?: string;
}

export interface UserSettings {
  playback_speed: number;
  skip_intro_secs: number;
  skip_outro_secs: number;
  ab_skip_forward_secs: number;
  ab_skip_backward_secs: number;
  ab_playback_speed: number;
}

export async function getEpisodeProgress(
  episodeId: string
): Promise<ProgressResponse | null> {
  try {
    const res = await client.get<ProgressResponse>(
      `/playback/episodes/${episodeId}/progress`
    );
    return res.data;
  } catch (err: unknown) {
    // 404 means no progress saved yet — treat as null
    if ((err as { response?: { status?: number } })?.response?.status === 404) {
      return null;
    }
    return null;
  }
}

export async function saveEpisodeProgress(
  episodeId: string,
  positionSecs: number,
  completed: boolean
): Promise<void> {
  await client.put(`/playback/episodes/${episodeId}/progress`, {
    position_secs: positionSecs,
    completed,
    device_kind: "web",
  });
}

/** One row per book this listener has started. */
export interface BookProgressSummary {
  book_id: string;
  /** Measured from the start of the book — the server has already added the
      durations of the files before the one being tracked. Divide by the book's
      `total_duration_secs` directly; adding file offsets again double-counts. */
  position_secs: number;
  file_id: string;
  /** The raw position inside `file_id`, which is what playback seeks to. */
  file_position_secs: number;
  completed: boolean;
  updated_at: string;
}

/**
 * Every started book in one request.
 *
 * The shelf used to ask per book, which cost one round trip each and — worse —
 * answered a position scoped to whichever file was playing, with no way to turn
 * it into a percentage of the whole book. This endpoint does that sum in SQL.
 */
export async function listBookProgress(): Promise<BookProgressSummary[]> {
  const res = await client.get<BookProgressSummary[]>("/playback/books/progress");
  return res.data;
}

export async function getBookProgress(
  bookId: string
): Promise<ProgressResponse | null> {
  try {
    const res = await client.get<ProgressResponse>(`/playback/books/${bookId}/progress`);
    return res.data;
  } catch (err: unknown) {
    if ((err as { response?: { status?: number } })?.response?.status === 404) {
      return null;
    }
    return null;
  }
}

export async function saveBookProgress(
  bookId: string,
  fileId: string,
  positionSecs: number,
  completed: boolean
): Promise<void> {
  await client.put(`/playback/books/${bookId}/progress`, {
    file_id: fileId,
    position_secs: positionSecs,
    completed,
    device_kind: "web",
  });
}

/** "Start over" — clears the saved position entirely rather than writing 0 to whichever file
 *  was last playing, so the next play begins at the book's first file. */
export async function resetBookProgress(bookId: string): Promise<void> {
  await client.delete(`/playback/books/${bookId}/progress`);
}

// ── User settings ─────────────────────────────────────────────────────────

export async function getUserSettings(): Promise<UserSettings> {
  const res = await client.get<UserSettings>("/playback/settings");
  return res.data;
}

export async function updateAudiobookDefaults(
  ab_skip_forward_secs: number,
  ab_skip_backward_secs: number,
  ab_playback_speed: number
): Promise<void> {
  await client.put("/playback/settings/audiobook-defaults", {
    ab_skip_forward_secs,
    ab_skip_backward_secs,
    ab_playback_speed,
  });
}

// ── Bookmarks ─────────────────────────────────────────────────────────────

export interface BookmarkResponse {
  id: string;
  episode_id: string | null;
  book_id: string | null;
  file_id: string | null;
  position_secs: number;
  label: string | null;
  audio_url: string | null;
  created_at: string;
}

/** List all bookmarks for the current user. */
export async function listBookmarks(): Promise<BookmarkResponse[]> {
  const res = await client.get<BookmarkResponse[]>("/playback/bookmarks");
  return res.data;
}

/** List bookmarks for a specific audiobook. */
export async function listBookBookmarks(
  bookId: string
): Promise<BookmarkResponse[]> {
  const res = await client.get<BookmarkResponse[]>(
    `/playback/books/${bookId}/bookmarks`
  );
  return res.data;
}

/** Create a text bookmark. */
export async function createBookmark(params: {
  bookId?: string;
  episodeId?: string;
  fileId?: string;
  positionSecs: number;
  label?: string;
}): Promise<BookmarkResponse> {
  const res = await client.post<BookmarkResponse>("/playback/bookmarks", {
    book_id: params.bookId ?? null,
    episode_id: params.episodeId ?? null,
    file_id: params.fileId ?? null,
    position_secs: params.positionSecs,
    label: params.label ?? null,
  });
  return res.data;
}

/** Update a bookmark's label. */
export async function updateBookmark(
  id: string,
  label: string | null
): Promise<void> {
  await client.put(`/playback/bookmarks/${id}`, { label });
}

/** Delete a bookmark. */
export async function deleteBookmark(id: string): Promise<void> {
  await client.delete(`/playback/bookmarks/${id}`);
}

// ── Listening sessions ────────────────────────────────────────────────────

export interface SessionReport {
  media_kind: "audiobook" | "podcast" | "music";
  item_id: string;
  part_id?: string;
  started_at: string;
  ended_at: string;
  /** Audio actually consumed — not wall clock, and not scaled by speed. */
  seconds_listened: number;
  playback_speed?: number;
  device_kind: "web";
  /** Idempotency key: generated when the span closes, reused on every retry. */
  client_session_id: string;
  /**
   * Why playback stopped. Only the client can tell a skip from a pause or a
   * closed tab, and nothing server-side can recover the difference afterwards.
   *
   * Send the fact and the position, **not a judgement**: how much an early skip
   * counts against a track is derived server-side from `seconds_listened`, so
   * that curve can be retuned without shipping every client again. Omit it when
   * genuinely unknown — an unrecognised value is dropped, and a missing reason
   * costs a little signal where a wrong one teaches the model something false.
   */
  ended_reason?: EndedReason;
}

export type EndedReason = "completed" | "skipped" | "stopped" | "replaced";

export interface ReportSessionsResponse {
  recorded: number;
  received: number;
}

/**
 * Report listening spans. Batched (server caps at 500) and idempotent through
 * `client_session_id`, so `recorded: 0` on a retry means the batch was already
 * counted — success, not failure.
 *
 * Reporting explicitly turns off the server's progress-derived sessions for
 * this user, so once the web app calls this it must keep calling it.
 */
export async function reportSessions(sessions: SessionReport[]): Promise<ReportSessionsResponse> {
  const { data } = await client.post<ReportSessionsResponse>("/playback/sessions", { sessions });
  return data;
}

// ── Cross-device queue ────────────────────────────────────────────────────

export interface QueueItem {
  media_kind: "audiobook" | "podcast" | "music";
  item_id: string;
  part_id?: string;
}

export interface QueueResponse {
  items: QueueItem[];
  current_index: number;
  position_secs: number;
  updated_by_device: string | null;
  updated_by_device_label: string | null;
  updated_at: string;
}

/** An empty queue comes back as an empty list, never a 404. */
export async function getQueue(): Promise<QueueResponse> {
  const { data } = await client.get<QueueResponse>("/playback/queue");
  return data;
}

/** Last write wins; the returned `updated_at` is how you notice being overtaken. */
export async function putQueue(req: {
  items: QueueItem[];
  current_index: number;
  position_secs: number;
}): Promise<QueueResponse> {
  const { data } = await client.put<QueueResponse>("/playback/queue", {
    ...req,
    device_kind: "web",
  });
  return data;
}

// ── Bulk episode progress ─────────────────────────────────────────────────

/** The only bulk endpoint that exists: mark many episodes played or unplayed. */
export async function bulkSetEpisodesPlayed(episodeIds: string[], completed: boolean): Promise<number> {
  if (episodeIds.length === 0) return 0;
  const { data } = await client.post<{ updated: number }>("/playback/episodes/progress/bulk", {
    episode_ids: episodeIds,
    completed,
  });
  return data.updated;
}
