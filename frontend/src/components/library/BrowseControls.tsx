// SPDX-License-Identifier: AGPL-3.0-or-later
import { ArrowUpDown, LayoutGrid, List, MoreHorizontal } from "lucide-react";
import { useCallback, useLayoutEffect, useRef, useState, type ReactNode } from "react";
import { SegmentedControl } from "../ui/SegmentedControl";
import { SearchField } from "../ui/Input";
import { IconButton } from "../ui/Button";
import { Menu, MenuTrigger, MenuContent, MenuRadioGroup, MenuRadioItem, MenuLabel } from "../ui/Menu";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

export type ViewMode = "grid" | "list";

export function ViewToggle({ value, onChange }: { value: ViewMode; onChange: (v: ViewMode) => void }) {
  const { t } = useT();
  return (
    <SegmentedControl<ViewMode>
      size="sm"
      value={value}
      onChange={onChange}
      segments={[
        { value: "grid", label: <LayoutGrid />, title: t("library.view.grid") },
        { value: "list", label: <List />, title: t("library.view.list") },
      ]}
    />
  );
}

export function SortMenu<T extends string>({ value, onChange, options, label: labelProp }: { value: T; onChange: (v: T) => void; options: { value: T; label: string }[]; label?: string }) {
  const { t } = useT();
  const label = labelProp ?? t("library.sort");
  return (
    <Menu>
      <MenuTrigger asChild>
        <IconButton size="sm" label={label}><ArrowUpDown className="h-4 w-4" /></IconButton>
      </MenuTrigger>
      <MenuContent>
        <MenuLabel>{label}</MenuLabel>
        <MenuRadioGroup value={value} onValueChange={(v) => onChange(v as T)}>
          {options.map((o) => <MenuRadioItem key={o.value} value={o.value}>{o.label}</MenuRadioItem>)}
        </MenuRadioGroup>
      </MenuContent>
    </Menu>
  );
}

export function FilterField({ value, onChange, placeholder: placeholderProp, className = "w-40 md:w-52" }: { value: string; onChange: (v: string) => void; placeholder?: string; className?: string }) {
  const { t } = useT();
  const placeholder = placeholderProp ?? t("library.filter");
  return <SearchField value={value} onChange={(e) => onChange(e.target.value)} placeholder={placeholder} className={className} aria-label={placeholder} />;
}

/**
 * `overlay` is the frosted pill that sits on top of cover art, where it has to
 * stay legible over any image. On a flat row it reads as a stray white disc, so
 * lists use `plain`, which matches the other icon buttons beside it.
 */
export function MoreMenuTrigger({ children, variant = "plain", size = "sm" }: { children: ReactNode; variant?: "plain" | "overlay"; size?: "sm" | "lg" }) {
  const { t } = useT();
  return (
    <Menu>
      <MenuTrigger asChild>
        <button
          aria-label={t("common.action.more")}
          onClick={(e) => e.stopPropagation()}
          className={cn(
            "flex items-center justify-center rounded-pill transition-colors",
            "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
            variant === "overlay"
              ? "h-7 w-7 bg-bg/85 text-fg backdrop-blur hover:bg-bg pointer-coarse:h-9 pointer-coarse:w-9"
              : cn(
                  "text-muted hover:bg-bg-alt hover:text-fg",
                  size === "lg" ? "h-11 w-11" : "h-8 w-8 pointer-coarse:h-10 pointer-coarse:w-10"
                )
          )}
        >
          <MoreHorizontal className="h-4 w-4" />
        </button>
      </MenuTrigger>
      <MenuContent onClick={(e) => e.stopPropagation()}>{children}</MenuContent>
    </Menu>
  );
}

export interface Chip<T extends string> { value: T; label: ReactNode }

/**
 * On a touch screen, one row of filter chips that scrolls sideways instead of
 * wrapping, so a phone keeps the filters on one line. With a mouse, where
 * sideways scrolling is awkward, the chips wrap. The last visible chip is cut by a fade,
 * which is what says there is more to scroll to — the reason SegmentedControl
 * wraps instead is that its hidden options had no such cue.
 *
 * `toggle` is an on/off chip for a separate dimension (e.g. "Mine"); it sits
 * before the single-choice chips, behind a divider.
 */
export function FilterChips<T extends string>({
  value,
  onChange,
  chips,
  toggle,
  label: labelProp,
}: {
  value: T;
  onChange: (v: T) => void;
  chips: Chip<T>[];
  toggle?: { label: ReactNode; on: boolean; onChange: (on: boolean) => void };
  label?: string;
}) {
  const { t } = useT();
  const label = labelProp ?? t("library.show");
  const ref = useRef<HTMLDivElement>(null);
  const [fade, setFade] = useState({ start: false, end: false });
  const measure = useCallback(() => {
    const el = ref.current;
    if (!el) return;
    setFade({ start: el.scrollLeft > 1, end: el.scrollLeft + el.clientWidth < el.scrollWidth - 1 });
  }, []);
  useLayoutEffect(() => {
    measure();
    const el = ref.current;
    if (!el) return;
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [measure]);

  const mask =
    fade.start || fade.end
      ? `linear-gradient(to right, ${fade.start ? "transparent, black 24px" : "black"}, ${fade.end ? "black calc(100% - 32px), transparent" : "black"})`
      : undefined;

  return (
    <div
      ref={ref}
      onScroll={measure}
      style={{ maskImage: mask, WebkitMaskImage: mask }}
      className="scrollbar-none -mx-5 flex items-center gap-1.5 overflow-x-auto px-5 py-0.5 pointer-fine:flex-wrap"
    >
      {toggle && (
        <>
          <ChipButton pressed={toggle.on} onClick={() => toggle.onChange(!toggle.on)}>
            {toggle.label}
          </ChipButton>
          <span aria-hidden className="mx-1 h-5 w-px shrink-0 bg-border" />
        </>
      )}
      <div role="radiogroup" aria-label={label} className="flex items-center gap-1.5 pointer-fine:contents">
        {chips.map((c) => (
          <ChipButton key={c.value} role="radio" pressed={c.value === value} onClick={() => onChange(c.value)}>
            {c.label}
          </ChipButton>
        ))}
      </div>
    </div>
  );
}

function ChipButton({ pressed, role, onClick, children }: { pressed: boolean; role?: "radio"; onClick: () => void; children: ReactNode }) {
  return (
    <button
      type="button"
      role={role}
      aria-checked={role === "radio" ? pressed : undefined}
      aria-pressed={role === "radio" ? undefined : pressed}
      onClick={onClick}
      className={cn(
        "inline-flex h-8 shrink-0 items-center whitespace-nowrap rounded-pill px-3.5 text-[13px] font-medium transition-colors pointer-coarse:h-9",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
        pressed ? "bg-fg text-bg" : "bg-bg-alt text-muted hover:text-fg"
      )}
    >
      {children}
    </button>
  );
}
