// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { useI18n } from "../i18n";
import { formatAmount, formatPrice, toAmountGrid } from "../api/billing";
import type { GenerationQuote } from "../api/types";
import QuoteStep from "../pages/generate/steps/QuoteStep";
import { chargedMicro, quotedMicro, quoteLines } from "./price";

useI18n.setState({ locale: "en" });

const quote = (extra: Partial<GenerationQuote> = {}): GenerationQuote => ({
  char_count: 12_000,
  translation_cost_cents: 2,
  tts_cost_cents: 2,
  internal_cost_cents: 0,
  quoted_price_cents: 10,
  currency: "USD",
  ...extra,
});

const precise = quote({
  translation_price_micro: 24_000,
  narration_price_micro: 18_000,
  preparation_price_micro: 3_000,
  quoted_price_micro: 45_000,
});

describe("formatAmount", () => {
  it("always shows four decimals", () => {
    expect(formatAmount(9_898_300, "USD")).toBe("$9.8983");
    expect(formatAmount(48_200, "USD")).toBe("$0.0482");
    expect(formatAmount(-1_700, "USD")).toBe("-$0.0017");
    expect(formatAmount(5_000_000, "USD")).toBe("$5.0000");
    expect(formatAmount(0, "USD")).toBe("$0.0000");
  });

  it("rounds off-grid micro half to even", () => {
    expect(formatAmount(150, "USD")).toBe("$0.0002");
    expect(formatAmount(250, "USD")).toBe("$0.0002");
    expect(formatAmount(350, "USD")).toBe("$0.0004");
    expect(formatAmount(149, "USD")).toBe("$0.0001");
    expect(formatAmount(251, "USD")).toBe("$0.0003");
    expect(formatAmount(-150, "USD")).toBe("-$0.0002");
    expect(formatAmount(-250, "USD")).toBe("-$0.0002");
    expect(formatAmount(-350, "USD")).toBe("-$0.0004");
    expect(formatAmount(-50, "USD")).toBe("$0.0000");
    expect(formatAmount(123_456_789_050, "USD")).toBe("$123,456.7890");
  });

  it("snaps to the 100-micro grid", () => {
    expect(toAmountGrid(1_666)).toBe(1_700);
    expect(toAmountGrid(-1_650)).toBe(-1_600);
    expect(toAmountGrid(50)).toBe(0);
  });
});

describe("formatPrice", () => {
  it("keeps the currency's two decimals for list prices", () => {
    expect(formatPrice(50_000, "USD")).toBe("$0.05");
    expect(formatPrice(10_000_000, "USD")).toBe("$10.00");
  });
});

describe("quoteLines", () => {
  it("adds up to the total on a server with micro prices", () => {
    const l = quoteLines(precise);
    expect(l.translation + l.narration + l.preparation + l.cover).toBe(l.total);
    expect(l.total).toBe(45_000);
  });

  it("falls back to the cents fields on an older server", () => {
    expect(quoteLines(quote())).toEqual({ translation: 20_000, narration: 20_000, preparation: 0, cover: 0, total: 100_000 });
  });

  it("renders the three lines and their sum in the confirm step", () => {
    const html = renderToStaticMarkup(<QuoteStep quote={precise} isLoading={false} />);
    for (const s of ["Translation", "$0.0240", "Narration (TTS)", "$0.0180", "Text preparation", "$0.0030", "$0.0450", "Free"]) {
      expect(html).toContain(s);
    }
  });

  it("hides empty translation and preparation lines", () => {
    const html = renderToStaticMarkup(
      <QuoteStep quote={quote({ narration_price_micro: 18_000, quoted_price_micro: 18_000, translation_price_micro: 0, preparation_price_micro: 0 })} isLoading={false} />
    );
    expect(html).not.toContain("Translation");
    expect(html).not.toContain("Text preparation");
    expect(html).toContain("$0.0180");
  });
});

describe("quotedMicro / chargedMicro", () => {
  it("prefers the micro fields", () => {
    expect(quotedMicro({ quoted_price_cents: 5, quoted_price_micro: 42_000 })).toBe(42_000);
    expect(chargedMicro({ charged_price_cents: 5, charged_price_micro: 41_000 })).toBe(41_000);
  });

  it("falls back to cents, keeping null for not yet charged", () => {
    expect(quotedMicro({ quoted_price_cents: 5 })).toBe(50_000);
    expect(chargedMicro({ charged_price_cents: 3 })).toBe(30_000);
    expect(chargedMicro({ charged_price_cents: null, charged_price_micro: null })).toBeNull();
  });
});
