// SPDX-License-Identifier: AGPL-3.0-or-later
/* eslint-disable react-refresh/only-export-components -- Radix provider is a component; the rule cannot tell from a re-export */
import * as TT from "@radix-ui/react-tooltip";
import type { ReactNode } from "react";

export const TooltipProvider = TT.Provider;

/**
 * Carries its own provider. Radix throws if a tooltip renders without one, and
 * the app-level provider only covers the signed-in shell — a tooltip on the
 * sign-in screen took the whole page down before this. Nested providers are
 * supported, so the shell-level one is harmless.
 */
export function Tooltip({
  label,
  children,
  side = "top",
}: {
  label: string;
  children: ReactNode;
  side?: "top" | "bottom" | "left" | "right";
}) {
  return (
    <TT.Provider delayDuration={400}>
      <TT.Root>
        <TT.Trigger asChild>{children}</TT.Trigger>
        <TT.Portal>
          <TT.Content
            side={side}
            sideOffset={6}
            className="z-[70] rounded-lg bg-fg px-2.5 py-1.5 text-xs font-medium text-bg shadow-pop"
          >
            {label}
          </TT.Content>
        </TT.Portal>
      </TT.Root>
    </TT.Provider>
  );
}
