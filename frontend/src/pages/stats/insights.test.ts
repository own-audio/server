// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { bestMonth, bucketize, busiestDay, dayParts, devices, familySummary, longestStreak, topPerKind, weekdays } from "./insights";
import type { FamilyStatsEntry, HistoryEntry, TopItem } from "../../api/types";

const d = (day: string, seconds: number) => ({ day, seconds });

describe("longestStreak", () => {
  it("finds the longest run of consecutive days and ignores days under a minute", () => {
    const days = [d("2026-03-30", 600), d("2026-03-31", 600), d("2026-04-01", 600), d("2026-04-02", 30), d("2026-04-10", 900), d("2026-04-11", 900)];
    expect(longestStreak(days)).toEqual({ length: 3, from: "2026-03-30", to: "2026-04-01" });
  });

  it("crosses a daylight-saving change as one day", () => {
    expect(longestStreak([d("2026-03-28", 100), d("2026-03-29", 100), d("2026-03-30", 100)]).length).toBe(3);
  });

  it("is zero for no listening", () => {
    expect(longestStreak([]).length).toBe(0);
  });
});

describe("day figures", () => {
  const days = [d("2026-01-05", 100), d("2026-01-06", 5000), d("2026-02-02", 3000), d("2026-02-09", 3000)];

  it("picks the busiest day and the best month", () => {
    expect(busiestDay(days)?.day).toBe("2026-01-06");
    expect(bestMonth(days)).toEqual({ month: "2026-02", seconds: 6000 });
  });

  it("totals weekdays Monday first", () => {
    const { totals, favourite } = weekdays(days); // 5 Jan, 2 Feb and 9 Feb 2026 are Mondays
    expect(totals[0]).toBe(6100);
    expect(totals[1]).toBe(5000);
    expect(favourite).toBe("Monday"); // English: no browser, so the default locale
    expect(weekdays(days).favouriteIndex).toBe(0);
  });
});

const session = (started_at: string, seconds: number, device_kind = "web"): HistoryEntry => ({
  id: started_at,
  media_kind: "music",
  item_id: "x",
  part_id: null,
  title: null,
  started_at,
  ended_at: started_at,
  seconds,
  device_kind,
  source: "reported",
});

describe("sessions", () => {
  it("splits time into parts of the day and devices", () => {
    const local = (h: number) => new Date(2026, 5, 1, h).toISOString();
    const history = [session(local(8), 100, "ios"), session(local(20), 300, "web"), session(local(21), 100, "web")];
    expect(dayParts(history).favourite).toBe("Evening");
    expect(devices(history)).toEqual([
      { kind: "web", label: "the web", share: 0.8 },
      { kind: "ios", label: "iPhone", share: 0.2 },
    ]);
  });
});

describe("topPerKind", () => {
  it("keeps the first, most-listened item of each kind", () => {
    const t = (media_kind: string, item_id: string): TopItem => ({ media_kind, item_id, title: item_id, seconds: 1, sessions: 1 });
    expect(topPerKind([t("music", "a"), t("audiobook", "b"), t("music", "c")])).toEqual({ music: t("music", "a"), audiobook: t("audiobook", "b") });
  });
});

describe("familySummary", () => {
  it("counts private members without figures and names each kind's leader", () => {
    const entries: FamilyStatsEntry[] = [
      { user_id: "1", display_name: "Ada", display_label: null, hidden: false, total_seconds: 3000, by_kind: [{ media_kind: "music", seconds: 3000, sessions: 1 }] },
      { user_id: "2", display_name: "Bo", display_label: "Dad", hidden: false, total_seconds: 1000, by_kind: [{ media_kind: "audiobook", seconds: 1000, sessions: 1 }] },
      { user_id: "3", display_name: "Cy", display_label: null, hidden: true },
    ];
    const s = familySummary(entries);
    expect(s.totalSeconds).toBe(4000);
    expect(s.members.map((m) => m.name)).toEqual(["Ada", "Dad"]);
    expect(s.privateCount).toBe(1);
    expect(s.leaders).toEqual([
      { kind: "audiobook", name: "Dad", seconds: 1000 },
      { kind: "music", name: "Ada", seconds: 3000 },
    ]);
  });
});

describe("bucketize", () => {
  const rows = [
    { day: "2026-09-30", media_kind: "music", seconds: 600 },
    { day: "2026-09-30", media_kind: "audiobook", seconds: 300 },
    { day: "2026-09-24", media_kind: "podcast", seconds: 100 },
    { day: "2026-07-01", media_kind: "music", seconds: 50 },
  ];

  it("gives a week as seven days ending today", () => {
    const b = bucketize("week", "2026-09-30", rows, []);
    expect(b.map((x) => x.start)).toEqual(["2026-09-24", "2026-09-25", "2026-09-26", "2026-09-27", "2026-09-28", "2026-09-29", "2026-09-30"]);
    expect(b[6].values).toEqual({ music: 600, audiobook: 300 });
    expect(b[6].total).toBe(900);
    expect(b[0].values).toEqual({ podcast: 100 });
  });

  it("gives a quarter as 13 Monday weeks and a year as 12 months", () => {
    const q = bucketize("quarter", "2026-09-30", rows, []);
    expect(q).toHaveLength(13);
    expect(q[12]).toMatchObject({ start: "2026-09-28", end: "2026-10-04", total: 900 });
    const y = bucketize("year", "2026-09-30", rows, []);
    expect(y).toHaveLength(12);
    expect(y[0].start).toBe("2025-10-01");
    expect(y[11]).toMatchObject({ start: "2026-09-01", end: "2026-09-30", total: 1000 });
    expect(y[9].total).toBe(50);
  });

  it("falls back to one series when the server sends no split", () => {
    const b = bucketize("week", "2026-09-30", null, [{ day: "2026-09-30", seconds: 42 }]);
    expect(b[6].values).toEqual({ all: 42 });
  });
});
