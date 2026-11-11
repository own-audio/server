import { describe, expect, it } from "vitest";
import type { LedgerEntry } from "../api/billing";
import { recentTopup } from "./topup";

const entry = (entry_type: string, created_at: string, amount_micro = 5_000_000): LedgerEntry => ({
  id: created_at, entry_type, amount_micro, storage_bytes: null, charge_date: null, note: null, created_at, external_ref: null,
});
const back = new Date("2026-11-11T18:00:00Z").getTime();

describe("recentTopup", () => {
  it("finds the top-up made just before the return", () => {
    expect(recentTopup([entry("topup", "2026-11-11T17:59:30Z")], back)?.created_at).toBe("2026-11-11T17:59:30Z");
  });
  it("arrives after the return too — the webhook can lag the redirect", () => {
    expect(recentTopup([entry("topup", "2026-11-11T18:00:05Z")], back)).toBeDefined();
  });
  it("ignores older top-ups and other entries", () => {
    expect(recentTopup([entry("topup", "2026-11-11T16:00:00Z"), entry("welcome_grant", "2026-11-11T17:59:00Z")], back)).toBeUndefined();
  });
  it("takes the newest when there are two", () => {
    const got = recentTopup([entry("topup", "2026-11-11T17:40:00Z", 1), entry("topup", "2026-11-11T17:59:00Z", 2)], back);
    expect(got?.amount_micro).toBe(2);
  });
});
