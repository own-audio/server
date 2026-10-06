// SPDX-License-Identifier: AGPL-3.0-or-later
/* eslint-disable react-refresh/only-export-components -- Radix roots are components; the rule cannot tell from a re-export */
import * as D from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../lib/cn";
import { useSectionStyle } from "../../lib/sectionTheme";
import { useT } from "../../i18n";

export const Dialog = D.Root;
export const DialogTrigger = D.Trigger;
export const DialogClose = D.Close;

interface DialogContentProps {
  title: string;
  description?: string;
  children: ReactNode;
  footer?: ReactNode;
  /** "sheet" slides up from the bottom on narrow screens. */
  variant?: "center" | "sheet";
  className?: string;
}

export function DialogContent({ title, description, children, footer, variant = "center", className }: DialogContentProps) {
  const { t } = useT();
  const sectionStyle = useSectionStyle();
  return (
    <D.Portal>
      <D.Overlay className="fixed inset-0 z-50 bg-overlay data-[state=open]:animate-fade-in data-[state=closed]:animate-fade-out" />
      <D.Content
        style={sectionStyle}
        className={cn(
          "fixed z-50 flex max-h-[90vh] w-full flex-col bg-card text-fg shadow-pop outline-none",
          variant === "sheet"
            ? cn(
                "bottom-0 left-0 rounded-t-sheet pb-[env(safe-area-inset-bottom)] sm:bottom-auto sm:left-1/2 sm:top-1/2 sm:max-w-lg sm:-translate-x-1/2 sm:-translate-y-1/2 sm:rounded-sheet sm:pb-0",
                "max-sm:data-[state=open]:animate-sheet-in max-sm:data-[state=closed]:animate-sheet-out",
                "sm:data-[state=open]:animate-pop-in sm:data-[state=closed]:animate-pop-out"
              )
            : "left-1/2 top-1/2 max-w-lg -translate-x-1/2 -translate-y-1/2 rounded-sheet data-[state=open]:animate-pop-in data-[state=closed]:animate-pop-out",
          className
        )}
      >
        <div className="flex items-start justify-between gap-4 px-6 pt-6">
          <div className="min-w-0">
            <D.Title className="text-lg font-semibold tracking-tight">{title}</D.Title>
            {description ? (
              <D.Description className="mt-1 text-sm text-muted">{description}</D.Description>
            ) : (
              <D.Description className="sr-only">{title}</D.Description>
            )}
          </div>
          <D.Close
            aria-label={t("common.action.close")}
            className="-mr-2 -mt-2 inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-pill text-muted hover:bg-bg-alt hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent pointer-coarse:h-10 pointer-coarse:w-10"
          >
            <X className="h-4 w-4" />
          </D.Close>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-6 py-5">{children}</div>
        {footer && <div className="flex justify-end gap-2 border-t border-border px-6 py-4">{footer}</div>}
      </D.Content>
    </D.Portal>
  );
}
