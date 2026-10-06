// SPDX-License-Identifier: AGPL-3.0-or-later
import type { GenerationQuote } from "../../../api/types";
import { formatCents } from "../../../lib/format";
import { useT } from "../../../i18n";

export default function QuoteStep({
  quote,
  isLoading,
}: {
  quote: GenerationQuote | undefined;
  isLoading: boolean;
}) {
  const { t, locale } = useT();
  return (
    <div>
      <h2 className="mb-1 text-lg font-semibold">{t("generate.quote.title")}</h2>
      <p className="mb-5 text-sm text-muted">{t("generate.quote.intro")}</p>

      {isLoading && <p className="text-sm text-muted">{t("generate.quote.calculating")}</p>}

      {quote && (
        <div className="rounded-xl border border-border bg-card p-5">
          <dl className="space-y-2 text-sm">
            <div className="flex justify-between">
              <dt className="text-muted">{t("generate.quote.characters")}</dt>
              <dd className="tabular-nums">{quote.char_count.toLocaleString(locale)}</dd>
            </div>
            {quote.translation_cost_cents > 0 && (
              <div className="flex justify-between">
                <dt className="text-muted">{t("generate.quote.translation")}</dt>
                <dd className="tabular-nums">{formatCents(quote.translation_cost_cents, quote.currency)}</dd>
              </div>
            )}
            <div className="flex justify-between">
              <dt className="text-muted">{t("generate.quote.narration")}</dt>
              <dd className="tabular-nums">{formatCents(quote.tts_cost_cents, quote.currency)}</dd>
            </div>
          </dl>
          <div className="mt-4 flex justify-between border-t border-border pt-4 text-base font-semibold">
            <span>{t("generate.quote.total")}</span>
            <span className="tabular-nums text-accent">
              {formatCents(quote.quoted_price_cents, quote.currency)}
            </span>
          </div>
          <p className="mt-3 text-xs text-muted">{t("generate.quote.finalNote")}</p>
        </div>
      )}
    </div>
  );
}
