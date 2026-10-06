// SPDX-License-Identifier: AGPL-3.0-or-later
import { reportSessions, type EndedReason, type SessionReport } from "../api/playback";

/**
 * Accumulates listening spans and flushes them in batches.
 *
 * Two rules from the contract (client guide §7) drive the shape here:
 *
 * - `seconds_listened` is **audio consumed**, not wall clock. A span that ran
 *   90 s at 1.5× consumed 90 s of audio position, which is what the player
 *   measures; the speed goes in its own field.
 * - `client_session_id` is what makes a retry safe. It is generated when the
 *   span closes and reused on every attempt, so a batch that reached the
 *   server before the response was lost is not counted twice. `recorded: 0`
 *   on a retry is success.
 *
 * Reporting explicitly switches the server's progress-derived sessions off for
 * this user, so a span that is dropped here is listening time that no longer
 * exists anywhere.
 */

const FLUSH_INTERVAL_MS = 60_000;
const MAX_BATCH = 500;
/** Below this, the span is a seek or a mis-tap rather than listening. */
const MIN_SPAN_SECONDS = 3;

let pending: SessionReport[] = [];
let timer: ReturnType<typeof setInterval> | null = null;
let flushing = false;

function newSessionId(): string {
  if (typeof crypto !== "undefined" && "randomUUID" in crypto) return crypto.randomUUID();
  // Older Safari: any unique string is fine, the server only compares it.
  return `${Date.now()}-${Math.random().toString(36).slice(2)}`;
}

export interface OpenSpan {
  mediaKind: SessionReport["media_kind"];
  itemId: string;
  partId?: string;
  startedAt: number;
  /** Audio position when the span opened, in seconds. */
  startPosition: number;
}

/** Open a span. Returns the handle to close it with. */
export function openSpan(span: Omit<OpenSpan, "startedAt">): OpenSpan {
  return { ...span, startedAt: Date.now() };
}

/**
 * Close a span and queue it for reporting. `endPosition` is the audio position
 * now; the difference is what was consumed. A span that went backwards (the
 * listener seeked back) reports zero rather than a negative, which the server
 * rejects outright.
 */
export function closeSpan(
  span: OpenSpan,
  endPosition: number,
  playbackSpeed: number,
  endedReason?: EndedReason
): void {
  const consumed = Math.round(endPosition - span.startPosition);
  if (consumed < MIN_SPAN_SECONDS) return;

  pending.push({
    media_kind: span.mediaKind,
    item_id: span.itemId,
    part_id: span.partId,
    started_at: new Date(span.startedAt).toISOString(),
    ended_at: new Date().toISOString(),
    seconds_listened: consumed,
    playback_speed: playbackSpeed,
    device_kind: "web",
    client_session_id: newSessionId(),
    ended_reason: endedReason,
  });

  if (pending.length >= MAX_BATCH) void flush();
}

/**
 * Send everything queued. Failures put the batch back so the next flush
 * retries it — the ids make that safe.
 */
export async function flush(): Promise<void> {
  if (flushing || pending.length === 0) return;
  flushing = true;
  const batch = pending.slice(0, MAX_BATCH);
  pending = pending.slice(batch.length);
  try {
    await reportSessions(batch);
  } catch {
    pending = [...batch, ...pending];
  } finally {
    flushing = false;
  }
}

/**
 * Last-chance flush on page close. `fetch` with `keepalive` is the only thing
 * that reliably survives unload; a normal request is cancelled with the page.
 */
export function flushOnUnload(token: string | null, baseUrl: string): void {
  if (pending.length === 0 || !token) return;
  const body = JSON.stringify({ sessions: pending });
  pending = [];
  try {
    void fetch(`${baseUrl}/playback/sessions`, {
      method: "POST",
      keepalive: true,
      headers: { "Content-Type": "application/json", Authorization: `Bearer ${token}` },
      body,
    });
  } catch {
    // the page is going away regardless
  }
}

export function startReporter(): () => void {
  if (timer) return () => {};
  timer = setInterval(() => void flush(), FLUSH_INTERVAL_MS);
  return () => {
    if (timer) clearInterval(timer);
    timer = null;
  };
}

/** Test seam. */
export function pendingCount(): number {
  return pending.length;
}
