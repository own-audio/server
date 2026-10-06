// SPDX-License-Identifier: AGPL-3.0-or-later
import { Users } from "lucide-react";
import type { Visibility } from "../../api/types";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/**
 * Who can see an item. Two states only, and no security framing: "private"
 * means "not shared with the family", not locked — so no padlock, and the
 * wording never promises protection.
 */
export function VisibilityField({
  value,
  onChange,
  disabled,
  hint,
}: {
  value: Visibility;
  onChange: (v: Visibility) => void;
  disabled?: boolean;
  hint?: string;
}) {
  const { t } = useT();
  return (
    <div>
      <p className="mb-1.5 text-[13px] font-medium">{t("library.visibility.title")}</p>
      <div className="flex gap-1.5">
        {(["private", "family"] as const).map((v) => (
          <button
            key={v}
            type="button"
            disabled={disabled}
            onClick={() => onChange(v)}
            className={cn(
              "flex flex-1 items-center gap-2 rounded-card border px-3 py-2 text-left text-sm transition-colors",
              v === value ? "border-accent bg-accent/8 text-fg" : "border-border text-muted hover:text-fg",
              disabled && "pointer-events-none opacity-50"
            )}
          >
            {v === "family" && <Users className="h-4 w-4 shrink-0" />}
            <span>
              <span className="block font-medium">{v === "private" ? t("library.visibility.private") : t("library.visibility.family")}</span>
              <span className="block text-xs text-muted">
                {v === "private" ? t("library.visibility.privateHint") : t("library.visibility.familyHint")}
              </span>
            </span>
          </button>
        ))}
      </div>
      {hint && <p className="mt-1.5 text-xs text-muted">{hint}</p>}
      {disabled && <p className="mt-1.5 text-xs text-muted">{t("library.visibility.ownerOnly")}</p>}
    </div>
  );
}
