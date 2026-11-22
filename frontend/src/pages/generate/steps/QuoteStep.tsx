// SPDX-License-Identifier: AGPL-3.0-or-later
import type { CoverMode } from "../../../api/generation";
import type { GenerationQuote, VoiceTier } from "../../../api/types";
import { formatCents } from "../../../lib/format";
import { formatMicro } from "../../../api/billing";
import { useT } from "../../../i18n";
import QuoteTierRow from "../QuoteTierRow";

export default function QuoteStep({
  quote,
  isLoading,
  tiers = [],
  coverMode = "auto",
  creditMicro,
  creditCurrency,
}: {
  quote: GenerationQuote | undefined;
  isLoading: boolean;
  tiers?: VoiceTier[];
  coverMode?: CoverMode;
  /** The family's credit, for information; the server decides whether it is enough. */
  creditMicro?: number;
  creditCurrency?: string;
}) {
  const { t, locale } = useT();
  const coverCents = quote?.cover_cost_cents ?? 0;
  return (
    <div>
      <h2 className="mb-1 text-lg font-semibold">{t("generate.quote.title")}</h2>
      <p className="mb-5 text-sm text-muted">{t("generate.quote.intro")}</p>

      {isLoading && <p className="text-sm text-muted">{t("generate.quote.calculating")}</p>}

      {quote && (
        <div className="rounded-xl border border-border bg-card p-5">
          <dl className="space-y-2 text-sm">
            <QuoteTierRow tiers={tiers} tierId={quote.voice_tier} preview={quote.voice_tier_preview} />
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
            {coverMode !== "none" && (
              <div className="flex justify-between">
                <dt className="text-muted">{t("generate.quote.cover")}</dt>
                {/* A server that still prices the cover says so in the quote; it is never hidden. */}
                <dd className="tabular-nums">
                  {coverCents > 0 ? formatCents(coverCents, quote.currency) : t("generate.quote.free")}
                </dd>
              </div>
            )}
          </dl>
          <div className="mt-4 flex justify-between border-t border-border pt-4 text-base font-semibold">
            <span>{t("generate.quote.total")}</span>
            <span className="tabular-nums text-accent">
              {formatCents(quote.quoted_price_cents, quote.currency)}
            </span>
          </div>
          <p className="mt-3 text-xs text-muted">{t("generate.tier.priceNote")}</p>
          {creditMicro != null && (
            <div className="mt-3 flex justify-between text-sm">
              <span className="text-muted">{t("generate.quote.credit")}</span>
              <span className="tabular-nums">{formatMicro(creditMicro, creditCurrency ?? quote.currency)}</span>
            </div>
          )}
        </div>
      )}
      {/* The server's sentence when it sends one (verbatim, in the request's language); the
          catalog's, which is the same text, from an older server. */}
      {quote && <p className="mt-4 text-xs leading-relaxed text-muted">{quote.personal_use_notice ?? t("generate.personalUse.long")}</p>}
    </div>
  );
}
