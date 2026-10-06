// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import { cn } from "../../lib/cn";

export interface Segment<T extends string> { value: T; label: ReactNode; title?: string }

export function SegmentedControl<T extends string>({ value, onChange, segments, size = "md", className }: { value: T; onChange: (v: T) => void; segments: Segment<T>[]; size?: "sm" | "md"; className?: string }) {
  return (
    /* Wrap rather than scroll. Scrolling kept the layout intact but hid
       options past the edge with the scrollbar deliberately suppressed — the
       Music switcher's fifth item, Playlists, was simply unreachable in a
       narrow column, and nothing on screen said it was there. */
    <div role="tablist" className={cn("flex max-w-full flex-wrap gap-0.5 rounded-card bg-bg-alt p-0.5", className)}>
      {segments.map((s) => {
        const active = s.value === value;
        return (
          <button
            key={s.value}
            role="tab"
            aria-selected={active}
            title={s.title}
            onClick={() => onChange(s.value)}
            className={cn(
              "inline-flex shrink-0 items-center gap-1.5 rounded-pill font-medium transition-colors",
              // Touch screens get finger-sized segments; a mouse keeps the compact ones.
              size === "sm" ? "h-7 px-2.5 text-xs pointer-coarse:h-9 pointer-coarse:px-3 pointer-coarse:text-[13px]" : "h-8 px-3 text-[13px] pointer-coarse:h-10",
              active ? "bg-card text-fg shadow-card" : "text-muted hover:text-fg",
              "[&>svg]:h-4 [&>svg]:w-4"
            )}
          >
            {s.label}
          </button>
        );
      })}
    </div>
  );
}
