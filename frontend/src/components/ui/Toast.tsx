// SPDX-License-Identifier: AGPL-3.0-or-later
import * as T from "@radix-ui/react-toast";
import { X } from "lucide-react";
import { useToasts } from "../../lib/toast";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

export function Toaster() {
  const { items, dismiss } = useToasts();
  const { t: tr } = useT();
  return (
    <T.Provider duration={5000} swipeDirection="right">
      {items.map((t) => (
        <T.Root
          key={t.id}
          onOpenChange={(open) => !open && dismiss(t.id)}
          className={cn(
            "flex items-start gap-3 rounded-card border border-border bg-card p-4 pr-3 shadow-pop",
            "data-[state=open]:animate-pop-in data-[swipe=end]:animate-out"
          )}
        >
          <span
            className={cn(
              "mt-1.5 h-2 w-2 shrink-0 rounded-pill",
              t.tone === "success" ? "bg-success" : t.tone === "error" ? "bg-error" : "bg-accent"
            )}
          />
          <div className="min-w-0 flex-1">
            <T.Title className="text-sm font-medium text-fg">{t.title}</T.Title>
            {t.description && <T.Description className="mt-0.5 text-xs text-muted">{t.description}</T.Description>}
          </div>
          {t.action && (
            <T.Action
              altText={t.action.label}
              onClick={t.action.onClick}
              className="shrink-0 rounded-pill px-2 py-0.5 text-sm font-medium text-accent hover:bg-bg-alt"
            >
              {t.action.label}
            </T.Action>
          )}
          <T.Close aria-label={tr("common.action.dismiss")} className="rounded-pill p-1 text-muted hover:text-fg">
            <X className="h-3.5 w-3.5" />
          </T.Close>
        </T.Root>
      ))}
      <T.Viewport className="fixed bottom-24 right-4 z-[60] flex w-80 max-w-[calc(100vw-2rem)] flex-col gap-2 outline-none" />
    </T.Provider>
  );
}
