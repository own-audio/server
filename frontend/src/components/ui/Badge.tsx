// SPDX-License-Identifier: AGPL-3.0-or-later
import { Users, Heart, Languages } from "lucide-react";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/* Content-state badges: "family" (shared), "favorite", and "translatable" (a
   podcast that publishes transcripts). Private content is the unmarked default
   and never gets a badge or a lock (repo CLAUDE.md §7). */
export function FamilyBadge({ className }: { className?: string }) {
  const { t } = useT();
  return (
    <span
      title={t("common.badge.family")}
      className={cn("inline-flex h-6 w-6 items-center justify-center rounded-pill bg-bg/85 text-fg backdrop-blur", className)}
    >
      <Users className="h-3.5 w-3.5" />
    </span>
  );
}

export function FavoriteBadge({ className }: { className?: string }) {
  const { t } = useT();
  return (
    <span
      title={t("common.badge.favorite")}
      className={cn("inline-flex h-6 w-6 items-center justify-center rounded-pill bg-bg/85 text-music backdrop-blur", className)}
    >
      <Heart className="h-3.5 w-3.5 fill-current" />
    </span>
  );
}

export function Pill({ children, tone = "neutral", className }: { children: React.ReactNode; tone?: "neutral" | "accent" | "success" | "warning" | "error"; className?: string }) {
  const tones = {
    neutral: "bg-bg-alt text-muted",
    // The plain --accent/--success are fill colours: on their own tints they
    // land near 3.6:1, under AA for 11px text. The darker -text steps exist
    // for exactly this.
    accent: "bg-accent/12 text-accent-text",
    success: "bg-success/12 text-success-text",
    warning: "bg-warning/12 text-warning",
    error: "bg-error/12 text-error",
  };
  return (
    <span className={cn("inline-flex items-center rounded-pill px-2 py-0.5 text-[11px] font-medium", tones[tone], className)}>
      {children}
    </span>
  );
}

export function TranslatableBadge({ className }: { className?: string }) {
  const { t } = useT();
  return (
    <span
      title={t("common.badge.translatable")}
      className={cn("inline-flex h-6 w-6 items-center justify-center rounded-pill bg-bg/85 text-accent-text backdrop-blur", className)}
    >
      <Languages className="h-3.5 w-3.5" aria-hidden />
      <span className="sr-only">{t("common.badge.translatable")}</span>
    </span>
  );
}
