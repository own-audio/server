// SPDX-License-Identifier: AGPL-3.0-or-later
import { forwardRef, type ButtonHTMLAttributes, type ReactNode } from "react";
import { Loader2 } from "lucide-react";
import { cn } from "../../lib/cn";

export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md" | "lg";

export interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
  loading?: boolean;
  icon?: ReactNode;
}

const variants: Record<ButtonVariant, string> = {
  primary: "bg-accent text-on-accent hover:brightness-110 active:brightness-95",
  secondary: "bg-bg-alt text-fg border border-border hover:bg-border/60",
  ghost: "text-fg hover:bg-bg-alt",
  // White on the dark-mode error red is only 3.4:1; the foreground has to flip.
  danger: "bg-error text-on-error hover:brightness-110",
};

const sizes: Record<ButtonSize, string> = {
  // Touch screens get 40 px, close to the 44 pt Apple asks for; a mouse keeps the compact sizes.
  sm: "h-8 px-3 text-[13px] gap-1.5 pointer-coarse:h-10 pointer-coarse:px-4 pointer-coarse:text-sm",
  md: "h-9 px-4 text-sm gap-2 pointer-coarse:h-11",
  lg: "h-11 px-5 text-[15px] gap-2",
};

export const Button = forwardRef<HTMLButtonElement, ButtonProps>(function Button(
  { variant = "primary", size = "md", loading, icon, className, children, disabled, ...rest },
  ref
) {
  return (
    <button
      ref={ref}
      disabled={disabled || loading}
      className={cn(
        "inline-flex items-center justify-center rounded-pill font-medium whitespace-nowrap select-none",
        "transition-[background-color,filter,transform] duration-150",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent focus-visible:ring-offset-2 focus-visible:ring-offset-bg",
        "disabled:opacity-50 disabled:pointer-events-none",
        variants[variant],
        sizes[size],
        className
      )}
      {...rest}
    >
      {loading ? <Loader2 className="h-4 w-4 animate-spin" /> : icon}
      {children}
    </button>
  );
});

/**
 * `tone` exists so a caller never has to fight the default hover styles with
 * appended classes. Two `hover:` utilities of equal specificity are resolved by
 * stylesheet order, not by the order they appear in the class string — an
 * override that looked fine produced a white icon on a near-white background,
 * i.e. an invisible button.
 */
export type IconButtonTone = "default" | "accent" | "solid";

export interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  size?: ButtonSize;
  active?: boolean;
  tone?: IconButtonTone;
}

const iconTones: Record<IconButtonTone, string> = {
  default: "text-muted hover:text-fg hover:bg-bg-alt",
  /** The primary action in a row — takes the section's colour. */
  accent: "bg-accent text-on-accent hover:brightness-110",
  /** Neutral fill for controls that sit over artwork or drive the player. */
  solid: "bg-fg text-bg hover:brightness-125",
};

export const IconButton = forwardRef<HTMLButtonElement, IconButtonProps>(function IconButton(
  { label, size = "md", active, tone = "default", className, children, ...rest },
  ref
) {
  const dim = size === "sm" ? "h-8 w-8 pointer-coarse:h-10 pointer-coarse:w-10" : size === "lg" ? "h-11 w-11" : "h-9 w-9 pointer-coarse:h-11 pointer-coarse:w-11";
  return (
    <button
      ref={ref}
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex items-center justify-center rounded-pill transition-colors duration-150",
        "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
        "disabled:opacity-40 disabled:pointer-events-none",
        active && tone === "default" ? "text-accent bg-accent/10" : iconTones[tone],
        dim,
        className
      )}
      {...rest}
    >
      {children}
    </button>
  );
});
