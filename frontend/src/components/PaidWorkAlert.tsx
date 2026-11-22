// SPDX-License-Identifier: AGPL-3.0-or-later
import { Link } from "react-router-dom";
import { refusalMessage, type PaidWorkRefusal } from "../lib/paidWork";
import { useT } from "../i18n";

/** A refused or failed order, in the catalog's words; a want of credit links to Storage & credit. */
export default function PaidWorkAlert({
  refusal,
  fallback,
  currency,
  className,
}: {
  refusal: PaidWorkRefusal;
  fallback: string;
  currency?: string;
  className?: string;
}) {
  const { t } = useT();
  return (
    <div role="alert" className={className ?? "mt-4 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error"}>
      <p>{refusalMessage(refusal, fallback, currency)}</p>
      {refusal.kind === "insufficient_credit" && (
        <Link to="/billing" className="mt-1 inline-block font-medium underline underline-offset-2 hover:no-underline">
          {t("billing.page.title")}
        </Link>
      )}
    </div>
  );
}
