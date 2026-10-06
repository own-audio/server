// SPDX-License-Identifier: AGPL-3.0-or-later
/* eslint-disable react-refresh/only-export-components -- Radix roots are components; the rule cannot tell from a re-export */
import * as DM from "@radix-ui/react-dropdown-menu";
import { Check } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "../../lib/cn";
import { useSectionStyle } from "../../lib/sectionTheme";

export const Menu = DM.Root;
export const MenuTrigger = DM.Trigger;

const contentClass =
  "z-50 min-w-44 rounded-card border border-border bg-card p-1 shadow-pop " +
  "data-[state=open]:animate-in data-[state=closed]:animate-out";

export function MenuContent({ children, align = "end", className, style, ...rest }: DM.DropdownMenuContentProps) {
  const sectionStyle = useSectionStyle();
  return (
    <DM.Portal>
      <DM.Content align={align} sideOffset={6} className={cn(contentClass, className)} style={{ ...sectionStyle, ...style }} {...rest}>
        {children}
      </DM.Content>
    </DM.Portal>
  );
}

const itemClass =
  "flex cursor-default select-none items-center gap-2 rounded-lg px-2.5 py-1.5 pointer-coarse:py-2.5 text-sm text-fg outline-none " +
  "data-[highlighted]:bg-bg-alt data-[disabled]:opacity-40 data-[disabled]:pointer-events-none";

export function MenuItem({ children, icon, destructive, className, ...rest }: DM.DropdownMenuItemProps & { icon?: ReactNode; destructive?: boolean }) {
  return (
    <DM.Item className={cn(itemClass, destructive && "text-error", className)} {...rest}>
      {icon && <span className="[&>svg]:h-4 [&>svg]:w-4 text-muted">{icon}</span>}
      {children}
    </DM.Item>
  );
}

export function MenuCheckItem({ children, className, ...rest }: DM.DropdownMenuCheckboxItemProps) {
  return (
    <DM.CheckboxItem className={cn(itemClass, "pl-7 relative", className)} {...rest}>
      <DM.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2">
        <Check className="h-3.5 w-3.5" />
      </DM.ItemIndicator>
      {children}
    </DM.CheckboxItem>
  );
}

export function MenuRadioGroup(props: DM.DropdownMenuRadioGroupProps) {
  return <DM.RadioGroup {...props} />;
}

export function MenuRadioItem({ children, className, ...rest }: DM.DropdownMenuRadioItemProps) {
  return (
    <DM.RadioItem className={cn(itemClass, "pl-7 relative", className)} {...rest}>
      <DM.ItemIndicator className="absolute left-2 top-1/2 -translate-y-1/2">
        <Check className="h-3.5 w-3.5" />
      </DM.ItemIndicator>
      {children}
    </DM.RadioItem>
  );
}

export function MenuSeparator() {
  return <DM.Separator className="my-1 h-px bg-border" />;
}

export function MenuLabel({ children }: { children: ReactNode }) {
  return <DM.Label className="px-2.5 py-1 text-[11px] font-medium uppercase tracking-wide text-muted">{children}</DM.Label>;
}
