// SPDX-License-Identifier: AGPL-3.0-or-later
import { describe, expect, it } from "vitest";
import { renderToStaticMarkup } from "react-dom/server";
import { MemoryRouter } from "react-router-dom";
import { useI18n } from "../../i18n";
import LanguageStep from "./steps/LanguageStep";
import QuoteStep from "./steps/QuoteStep";
import PaidWorkAlert from "../../components/PaidWorkAlert";
import type { GenerationQuote } from "../../api/types";

useI18n.setState({ locale: "en" });

const LANGS = [
  { code: "en", label: "English" },
  { code: "cs", label: "Czech" },
];
const noop = () => {};

const languageStep = (fixed: boolean, translate: boolean, target = "cs") =>
  renderToStaticMarkup(
    <LanguageStep
      languages={LANGS}
      sourceLanguage="en"
      targetLanguage={target}
      translate={translate}
      fixed={fixed}
      onSourceLanguageChange={noop}
      onTargetLanguageChange={noop}
      onTranslateChange={noop}
    />
  );

const QUOTE: GenerationQuote = {
  char_count: 120_000,
  translation_cost_cents: 240,
  tts_cost_cents: 1800,
  cover_cost_cents: 0,
  internal_cost_cents: 2040,
  quoted_price_cents: 4080,
  currency: "USD",
};

describe("Translate a book", () => {
  it("asks for both languages with no checkbox", () => {
    const html = languageStep(true, true);
    expect(html).toContain("The book is in");
    expect(html).toContain("Translate into");
    expect(html).not.toContain('type="checkbox"');
    // The target list leaves the source out.
    expect(html.match(/<option value="en"/g)).toHaveLength(1);
  });

  it("says when the two languages are the same", () => {
    expect(languageStep(true, true, "en")).toContain("Pick a language other than the book");
  });

  it("keeps Narrate a book's checkbox and words", () => {
    const html = languageStep(false, false);
    expect(html).toContain("Source language");
    expect(html).toContain('type="checkbox"');
    expect(html).not.toContain("Translate into");
  });
});

describe("the confirm step", () => {
  it("shows the cap line, the credit and the personal-use note", () => {
    const html = renderToStaticMarkup(<QuoteStep quote={QUOTE} isLoading={false} coverMode="auto" creditMicro={12_340_000} />);
    expect(html).toContain("$40.80");
    expect(html).toContain("Free");
    expect(html).toContain("The price shown is the most you pay.");
    expect(html).toContain("Family credit");
    expect(html).toContain("$12.34");
    expect(html).toContain("meant for personal listening");
  });

  it("shows the server's own personal-use sentence when it sends one", () => {
    const html = renderToStaticMarkup(<QuoteStep quote={{ ...QUOTE, personal_use_notice: "Server sentence." }} isLoading={false} />);
    expect(html).toContain("Server sentence.");
    expect(html).not.toContain("meant for personal listening");
  });

  it("leaves the credit out when it is not known", () => {
    const html = renderToStaticMarkup(<QuoteStep quote={QUOTE} isLoading={false} />);
    expect(html).not.toContain("Family credit");
  });
});

describe("a refusal", () => {
  it("links to Storage & credit when credit is short", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <PaidWorkAlert refusal={{ kind: "insufficient_credit" }} fallback="fb" />
      </MemoryRouter>
    );
    expect(html).toContain('href="/billing"');
    expect(html).toContain("Storage &amp; credit");
    expect(html).toContain('role="alert"');
  });

  it("has no link for any other refusal", () => {
    const html = renderToStaticMarkup(
      <MemoryRouter>
        <PaidWorkAlert refusal={{ kind: "too_many_jobs", maxRunning: 2 }} fallback="fb" />
      </MemoryRouter>
    );
    expect(html).not.toContain("/billing");
  });
});
