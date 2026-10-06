// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { FamilyStatsEntry, HistoryEntry, StatsRange, StatsSummary, StatsVisibility } from "./types";

/** Minutes east of UTC. Without it the server buckets days in UTC and the
 *  streak looks wrong to anyone who isn't on UTC. */
function tzOffsetMinutes(): number {
  return -new Date().getTimezoneOffset();
}

export async function getStats(range: StatsRange): Promise<StatsSummary> {
  const { data } = await api.get<StatsSummary>("/stats/me", {
    params: { range, tz_offset_minutes: tzOffsetMinutes() },
  });
  return data;
}

export async function setStatsVisibility(stats_visibility: StatsVisibility): Promise<void> {
  await api.put("/stats/me/visibility", { stats_visibility });
}

/** Family admins only. Members who keep stats private come back `hidden: true`
 *  with no figures — render the row, omit the numbers. */
export async function getFamilyStats(range: StatsRange = "30d"): Promise<FamilyStatsEntry[]> {
  const { data } = await api.get<FamilyStatsEntry[]>("/stats/family", {
    params: { range, tz_offset_minutes: tzOffsetMinutes() },
  });
  return data;
}

export async function listHistory(limit = 50, offset = 0): Promise<HistoryEntry[]> {
  const { data } = await api.get<HistoryEntry[]>("/stats/me/history", { params: { limit, offset } });
  return data;
}

/** Sessions started since `since`, newest first, a page (the server's 200 cap)
 *  at a time, stopping after `maxPages` — enough for patterns, not an export. */
export async function listHistorySince(since: Date, maxPages = 5): Promise<HistoryEntry[]> {
  const out: HistoryEntry[] = [];
  for (let page = 0; page < maxPages; page++) {
    const rows = await listHistory(200, page * 200);
    const fresh = rows.filter((r) => new Date(r.started_at) >= since);
    out.push(...fresh);
    if (rows.length < 200 || fresh.length < rows.length) break;
  }
  return out;
}

/**
 * Family admins, and only for a member they manage — the server refuses
 * (403) for an adult with unrestricted access, so a parent cannot quietly
 * start watching another grown-up's listening.
 */
export async function setMemberStatsVisibility(
  userId: string,
  stats_visibility: StatsVisibility
): Promise<void> {
  await api.put(`/stats/family/members/${userId}/visibility`, { stats_visibility });
}
