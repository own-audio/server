// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { Check, Play, Square } from "lucide-react";
import type { VoiceTier, VoiceTierId } from "../../api/types";
import { Pill } from "../../components/ui";
import { cn } from "../../lib/cn";
import { formatCents } from "../../lib/format";
import { sampleUrl, tierLanguage, tierName } from "../../lib/voiceTiers";
import { usePlayerStore } from "../../store/playerStore";
import { useT } from "../../i18n";

/** Tier prices come without a currency; the hosted service bills in dollars. */
const TIER_CURRENCY = "USD";

/**
 * One card per voice tier, with its price per hour in the chosen language and a sample to hear.
 * The parent decides whether to show it at all (`showsTierPicker`): a server without tiers gets
 * the voice list alone, as before.
 */
export default function VoiceTierPicker({
  tiers,
  language,
  value,
  onChange,
}: {
  tiers: VoiceTier[];
  language: string;
  value: VoiceTierId | undefined;
  onChange: (id: VoiceTierId) => void;
}) {
  const { t } = useT();
  const sample = useSamplePlayer();

  return (
    <div>
      <p id="voice-tier-label" className="mb-2 text-sm font-medium text-fg">{t("generate.tier.title")}</p>
      <div role="radiogroup" aria-labelledby="voice-tier-label" className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {tiers.map((tier) => {
          const lang = tierLanguage(tier, language);
          const selected = tier.id === value;
          const name = tierName(tier);
          const url = sampleUrl(tier, language);
          const playing = sample.playing === url;
          return (
            <div
              key={tier.id}
              className={cn(
                "flex flex-col rounded-xl border transition",
                selected ? "border-accent bg-accent/6" : "border-border bg-card",
                lang ? (selected ? "" : "hover:border-accent") : "opacity-60"
              )}
            >
              <button
                type="button"
                role="radio"
                aria-checked={selected}
                disabled={!lang}
                onClick={() => onChange(tier.id)}
                className="flex flex-1 flex-col justify-start rounded-xl p-4 text-left focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent disabled:cursor-not-allowed"
              >
                <span className="flex items-start justify-between gap-2">
                  <span className="font-semibold">{name}</span>
                  {selected && <Check className="mt-0.5 h-4 w-4 shrink-0 text-accent-text" aria-hidden />}
                </span>
                {tier.preview && (
                  <span className="mt-1.5 block" title={t("generate.tier.previewHint")}>
                    <Pill tone="warning">{t("generate.tier.preview")}</Pill>
                    <span className="sr-only">{t("generate.tier.previewHint")}</span>
                  </span>
                )}
                <span className="mt-2 block text-sm tabular-nums text-muted">
                  {lang
                    ? t("generate.tier.perHour", { price: formatCents(lang.price_per_hour_cents, TIER_CURRENCY) })
                    : t("generate.tier.unavailable")}
                </span>
              </button>
              {lang && (
                <button
                  type="button"
                  onClick={() => sample.toggle(url)}
                  aria-pressed={playing}
                  aria-label={playing ? t("generate.tier.stopLabel", { name }) : t("generate.tier.playLabel", { name })}
                  className="mx-4 mb-4 inline-flex items-center gap-1.5 self-start rounded-pill border border-border px-3 py-1 text-xs font-medium text-fg hover:border-accent focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-accent"
                >
                  {playing ? <Square className="h-3 w-3 fill-current" aria-hidden /> : <Play className="h-3 w-3 fill-current" aria-hidden />}
                  {playing ? t("generate.tier.stop") : t("generate.tier.play")}
                </button>
              )}
            </div>
          );
        })}
      </div>
      {sample.failed && (
        <p role="status" className="mt-2 text-xs text-error">{t("generate.tier.sampleError")}</p>
      )}
      <p className="mt-2 text-xs text-muted">{t("generate.tier.priceNote")}</p>
    </div>
  );
}

/** One sample at a time, through one audio element, and the family's player paused while it plays. */
function useSamplePlayer() {
  const audio = useRef<HTMLAudioElement | null>(null);
  const [playing, setPlaying] = useState<string | null>(null);
  const [failed, setFailed] = useState(false);

  useEffect(
    () => () => {
      audio.current?.pause();
      audio.current = null;
    },
    []
  );

  function element(): HTMLAudioElement {
    if (!audio.current) {
      const el = new Audio();
      el.addEventListener("ended", () => setPlaying(null));
      audio.current = el;
    }
    return audio.current;
  }

  function toggle(url: string) {
    const el = element();
    if (playing === url) {
      el.pause();
      setPlaying(null);
      return;
    }
    setFailed(false);
    usePlayerStore.getState().setPlaying(false);
    el.src = url;
    setPlaying(url);
    el.play().catch((err: unknown) => {
      // A pause() while loading rejects with AbortError; that is a stop, not a failure.
      if ((err as Error)?.name === "AbortError") return;
      setPlaying((p) => (p === url ? null : p));
      setFailed(true);
    });
  }

  return { playing, failed, toggle };
}
