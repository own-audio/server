// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, it, expect, vi, beforeEach, afterEach } from "vitest";

/* The contract rules worth a test (client guide §7): a span reports audio
   consumed rather than wall clock, retries reuse the same idempotency key, and
   a failed flush keeps the batch instead of dropping listening time. */

const reportSessions = vi.fn();
vi.mock("../api/playback", () => ({ reportSessions: (s: unknown[]) => reportSessions(s) }));

let mod: typeof import("./sessionReporter");

beforeEach(async () => {
  vi.resetModules();
  reportSessions.mockReset().mockResolvedValue({ recorded: 1, received: 1 });
  mod = await import("./sessionReporter");
});

afterEach(() => {
  vi.useRealTimers();
});

describe("sessionReporter", () => {
  it("reports audio consumed, not wall clock", async () => {
    vi.useFakeTimers();
    const span = mod.openSpan({ mediaKind: "music", itemId: "t1", startPosition: 100 });
    // Two minutes of wall clock, but only 40 s of audio at 2x.
    vi.advanceTimersByTime(120_000);
    mod.closeSpan(span, 140, 2);
    await mod.flush();

    expect(reportSessions).toHaveBeenCalledOnce();
    const [sent] = reportSessions.mock.calls[0] as [Array<Record<string, unknown>>];
    expect(sent[0].seconds_listened).toBe(40);
    expect(sent[0].playback_speed).toBe(2);
    expect(sent[0].device_kind).toBe("web");
  });

  it("drops spans too short to be listening", async () => {
    const span = mod.openSpan({ mediaKind: "music", itemId: "t1", startPosition: 10 });
    mod.closeSpan(span, 11, 1);
    await mod.flush();
    expect(reportSessions).not.toHaveBeenCalled();
  });

  it("never reports a negative span after seeking backwards", async () => {
    const span = mod.openSpan({ mediaKind: "audiobook", itemId: "b1", startPosition: 500 });
    mod.closeSpan(span, 20, 1);
    await mod.flush();
    expect(reportSessions).not.toHaveBeenCalled();
  });

  it("gives every span its own idempotency key", async () => {
    for (const start of [0, 100]) {
      const span = mod.openSpan({ mediaKind: "podcast", itemId: "e1", startPosition: start });
      mod.closeSpan(span, start + 60, 1);
    }
    await mod.flush();
    const [sent] = reportSessions.mock.calls[0] as [Array<Record<string, unknown>>];
    expect(sent).toHaveLength(2);
    expect(sent[0].client_session_id).not.toBe(sent[1].client_session_id);
  });

  it("keeps the batch when a flush fails, and reuses the same keys on retry", async () => {
    const span = mod.openSpan({ mediaKind: "music", itemId: "t1", startPosition: 0 });
    mod.closeSpan(span, 60, 1);

    reportSessions.mockRejectedValueOnce(new Error("offline"));
    await mod.flush();
    expect(mod.pendingCount()).toBe(1);

    await mod.flush();
    expect(reportSessions).toHaveBeenCalledTimes(2);
    const first = (reportSessions.mock.calls[0] as [Array<Record<string, unknown>>])[0][0];
    const second = (reportSessions.mock.calls[1] as [Array<Record<string, unknown>>])[0][0];
    expect(second.client_session_id).toBe(first.client_session_id);
    expect(mod.pendingCount()).toBe(0);
  });
});

describe("ended_reason", () => {
  it("carries the reason through to the batch", async () => {
    const span = mod.openSpan({ mediaKind: "music", itemId: "t1", startPosition: 0 });
    mod.closeSpan(span, 200, 1, "skipped");
    await mod.flush();
    expect(reportSessions.mock.calls[0][0][0].ended_reason).toBe("skipped");
  });

  /* A skip five seconds in and a skip near the end are both `skipped`; the
     server weighs them from the position. The client must not pre-judge, or
     that curve can never be retuned without shipping every client again. */
  it("sends the same reason whatever the position", async () => {
    const early = mod.openSpan({ mediaKind: "music", itemId: "t1", startPosition: 0 });
    mod.closeSpan(early, 5, 1, "skipped");
    const late = mod.openSpan({ mediaKind: "music", itemId: "t2", startPosition: 0 });
    mod.closeSpan(late, 230, 1, "skipped");
    await mod.flush();

    const batch = reportSessions.mock.calls[0][0];
    expect(batch.map((s: { ended_reason?: string }) => s.ended_reason)).toEqual([
      "skipped",
      "skipped",
    ]);
    expect(batch.map((s: { seconds_listened: number }) => s.seconds_listened)).toEqual([5, 230]);
  });

  /* Omitting it must stay legal: an older build, or a span whose cause is
     genuinely unknown, still reports its listening time. */
  it("is absent when not given rather than guessed", async () => {
    const span = mod.openSpan({ mediaKind: "music", itemId: "t1", startPosition: 0 });
    mod.closeSpan(span, 200, 1);
    await mod.flush();
    expect(reportSessions.mock.calls[0][0][0].ended_reason).toBeUndefined();
  });
});
