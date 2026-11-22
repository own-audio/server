// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { useI18n } from "../i18n";
import { coverChangeMessage, readRefusal, refusalMessage } from "./paidWork";

useI18n.setState({ locale: "en" });

const refused = (status: number, data: unknown = {}) => ({ response: { status, data } });

describe("paid-work refusals", () => {
  it("reads each code the server sends", () => {
    expect(readRefusal(refused(402, { code: "insufficient_credit", error: "x" }))).toEqual({ kind: "insufficient_credit" });
    expect(readRefusal(refused(409, { code: "daily_cap_reached", daily_cap_micro: 70_000_000 }))).toEqual({ kind: "daily_cap", limitMicro: 70_000_000 });
    expect(readRefusal(refused(409, { code: "too_many_jobs", running: 2, max_running: 3 }))).toEqual({ kind: "too_many_jobs", maxRunning: 3 });
    expect(readRefusal(refused(403, { code: "email_unverified" }))).toEqual({ kind: "email_unverified" });
    expect(readRefusal(refused(501, { code: "feature_unavailable" }))).toEqual({ kind: "feature_unavailable" });
  });

  it("goes by the code, not the status or the English text", () => {
    expect(readRefusal(refused(409, { code: "daily_cap_reached", error: "too many jobs" })).kind).toBe("daily_cap");
    expect(readRefusal(refused(403, { error: "Forbidden" }))).toEqual({ kind: "not_allowed" });
  });

  it("knows a withdrawn tier from any other 400", () => {
    expect(readRefusal(refused(400, { error: "voice_tier gemini_flash is not available on this server" })).kind).toBe("tier_withdrawn");
    expect(readRefusal(refused(400, { error: "unknown voice_tier: gemini_pro" })).kind).toBe("tier_withdrawn");
    expect(readRefusal(refused(400, { error: "File too large" }))).toEqual({ kind: "bad_request", message: "File too large" });
    expect(readRefusal(refused(422, { detail: "Same language twice" }))).toEqual({ kind: "bad_request", message: "Same language twice" });
  });

  it("treats a network error or a 5xx as unknown", () => {
    expect(readRefusal(new Error("Network Error")).kind).toBe("other");
    expect(readRefusal(refused(500)).kind).toBe("other");
    expect(readRefusal(refused(502, "<html>")).kind).toBe("other");
  });

  it("says each refusal in the catalog's words", () => {
    expect(refusalMessage({ kind: "insufficient_credit" }, "fb")).toBe(
      "There isn't enough credit in the family for this. Add credit in Storage & credit, then try again."
    );
    expect(refusalMessage({ kind: "email_unverified" }, "fb")).toBe("Confirm your email address first.");
    expect(refusalMessage({ kind: "not_allowed" }, "fb")).toBe("You can't order paid work in this family.");
    expect(refusalMessage({ kind: "tier_withdrawn" }, "fb")).toBe("That voice model is no longer offered. Choose another.");
    expect(refusalMessage({ kind: "bad_request", message: "File too large" }, "fb")).toBe("File too large");
    expect(refusalMessage({ kind: "bad_request", message: null }, "fb")).toBe("fb");
    expect(refusalMessage({ kind: "other" }, "Couldn't start narrating the book.")).toBe("Couldn't start narrating the book.");
  });

  it("never shows a placeholder for a figure the server did not send", () => {
    expect(refusalMessage({ kind: "daily_cap", limitMicro: null }, "fb")).toBe("Today's limit for paid work is reached. Try again tomorrow.");
    expect(refusalMessage(readRefusal(refused(409, { code: "daily_cap_reached" })), "fb")).not.toContain("—");
  });

  it("reads the figures as v2 problem members too", () => {
    const problem = { type: "about:blank", status: 409, code: "daily_cap_reached", detail: "x", daily_cap_micro: 70_000_000, spent_today_micro: 69_000_000 };
    expect(refusalMessage(readRefusal(refused(409, problem)), "fb")).toContain("($70.00)");
    expect(readRefusal(refused(409, { code: "too_many_jobs", running: 3, max_running: "3" }))).toEqual({ kind: "too_many_jobs", maxRunning: 3 });
    expect(readRefusal(refused(409, { code: "daily_cap_reached", daily_cap_micro: "nope" }))).toEqual({ kind: "daily_cap", limitMicro: null });
  });

  it("formats the daily cap the server sent, never a fixed one", () => {
    expect(refusalMessage({ kind: "daily_cap", limitMicro: 70_000_000 }, "fb")).toBe(
      "Today's limit for paid work ($70.00) is reached. Try again tomorrow."
    );
    expect(refusalMessage({ kind: "daily_cap", limitMicro: 12_500_000 }, "fb")).toContain("($12.50)");
  });

  it("counts the running jobs from the body", () => {
    expect(refusalMessage({ kind: "too_many_jobs", maxRunning: 2 }, "fb")).toBe(
      "2 narrations or translations are already being made for your family. Wait for one to finish."
    );
    expect(refusalMessage({ kind: "too_many_jobs", maxRunning: 1 }, "fb")).toContain("Wait for it to finish.");
  });

  it("reads in Czech too", () => {
    useI18n.setState({ locale: "cs" });
    try {
      expect(refusalMessage({ kind: "too_many_jobs", maxRunning: 2 }, "fb")).toBe(
        "Pro vaši rodinu se už vytvářejí 2 namlouvání nebo překlady. Počkejte, až jedno skončí."
      );
      expect(refusalMessage({ kind: "insufficient_credit" }, "fb")).toContain("Úložiště a kredit");
    } finally {
      useI18n.setState({ locale: "en" });
    }
  });
});

describe("cover change errors", () => {
  it("has a sentence for every code", () => {
    expect(coverChangeMessage(refused(409, { code: "not_finished" }))).toBe("The book isn't finished yet, or it was deleted.");
    expect(coverChangeMessage(refused(422, { code: "no_book_cover" }))).toBe("The book's file has no cover of its own.");
    expect(coverChangeMessage(refused(429, { code: "cover_limit_reached" }))).toBe("This book has used all its free covers.");
    expect(coverChangeMessage(refused(502, { code: "cover_failed" }))).toContain("Nothing was used up");
    expect(coverChangeMessage(refused(503, { code: "cover_unavailable" }))).toBe("Covers can't be made on this server right now.");
    expect(coverChangeMessage(refused(404, { error: "Not found" }))).toBe("This narration isn't yours, or it no longer exists.");
    expect(coverChangeMessage(refused(400, { error: "cover_prompt is required for custom" }))).toBe("cover_prompt is required for custom");
    expect(coverChangeMessage(new Error("Network Error"))).toBe("Couldn't change the cover.");
  });
});
