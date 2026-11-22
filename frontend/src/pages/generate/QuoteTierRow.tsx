// SPDX-License-Identifier: AGPL-3.0-or-later
import type { VoiceTier, VoiceTierId } from "../../api/types";
import { Pill } from "../../components/ui";
import { tierName } from "../../lib/voiceTiers";
import { useT } from "../../i18n";

/** The quote's voice model, as the server priced it; nothing on a server without tiers. */
export default function QuoteTierRow({
  tiers,
  tierId,
  preview,
}: {
  tiers: VoiceTier[];
  tierId: VoiceTierId | undefined;
  preview: boolean | undefined;
}) {
  const { t } = useT();
  if (!tierId || tiers.length === 0) return null;
  const tier = tiers.find((x) => x.id === tierId);
  return (
    <div className="flex items-center justify-between gap-3">
      <dt className="text-muted">{t("generate.quote.voiceModel")}</dt>
      <dd className="flex items-center gap-2 text-right">
        <span>{tier ? tierName(tier) : tierId}</span>
        {(preview ?? tier?.preview) && <Pill tone="warning">{t("generate.tier.preview")}</Pill>}
      </dd>
    </div>
  );
}
