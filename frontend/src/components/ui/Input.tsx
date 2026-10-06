// SPDX-License-Identifier: AGPL-3.0-or-later
import { forwardRef, useState, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TextareaHTMLAttributes } from "react";
import { ChevronDown, Eye, EyeOff } from "lucide-react";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

const base =
  "w-full rounded-[10px] border border-border bg-card px-3 text-sm text-fg placeholder:text-muted " +
  "transition-colors focus:border-accent focus:outline-none focus:ring-2 focus:ring-accent/30 " +
  "disabled:opacity-50";

export interface InputProps extends InputHTMLAttributes<HTMLInputElement> {
  label?: string;
  hint?: string;
  error?: string;
  /** Sits inside the field at its right edge, e.g. a show-password button. */
  trailing?: ReactNode;
}

export const Input = forwardRef<HTMLInputElement, InputProps>(function Input(
  { label, hint, error, trailing, className, id, ...rest },
  ref
) {
  const inputId = id ?? (label ? label.toLowerCase().replace(/\s+/g, "-") : undefined);
  return (
    // A div, not a wrapping <label>: a button in the `trailing` slot would
    // otherwise become part of the field's accessible name.
    <div>
      {label && (
        <label htmlFor={inputId} className="mb-1.5 block text-[13px] font-medium text-fg">
          {label}
        </label>
      )}
      <span className="relative block">
        <input
          ref={ref}
          id={inputId}
          aria-invalid={error ? true : undefined}
          className={cn(base, "h-10", trailing != null && "pr-11", error && "border-error focus:ring-error/30", className)}
          {...rest}
        />
        {trailing != null && <span className="absolute inset-y-0 right-1 flex items-center">{trailing}</span>}
      </span>
      {error ? (
        <span className="mt-1 block text-xs text-error">{error}</span>
      ) : hint ? (
        <span className="mt-1 block text-xs text-muted">{hint}</span>
      ) : null}
    </div>
  );
});

/* One password field with a show/hide eye instead of a second "confirm"
   field: seeing what you typed catches the typo a confirm field is there for,
   with half the typing. */
export const PasswordInput = forwardRef<HTMLInputElement, Omit<InputProps, "type" | "trailing">>(function PasswordInput(props, ref) {
  const [shown, setShown] = useState(false);
  const { t } = useT();
  return (
    <Input
      ref={ref}
      {...props}
      type={shown ? "text" : "password"}
      autoCapitalize="none"
      autoCorrect="off"
      spellCheck={false}
      trailing={
        <button
          type="button"
          aria-label={shown ? t("auth.password.hide") : t("auth.password.show")}
          aria-pressed={shown}
          // Keep focus (and the phone keyboard) in the field.
          onMouseDown={(e) => e.preventDefault()}
          onClick={() => setShown((v) => !v)}
          className="flex h-8 w-9 items-center justify-center rounded-lg text-muted transition-colors hover:text-fg"
        >
          {shown ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
        </button>
      }
    />
  );
});

export const Textarea = forwardRef<HTMLTextAreaElement, TextareaHTMLAttributes<HTMLTextAreaElement> & { label?: string }>(
  function Textarea({ label, className, id, ...rest }, ref) {
    const inputId = id ?? (label ? label.toLowerCase().replace(/\s+/g, "-") : undefined);
    return (
      <label className="block" htmlFor={inputId}>
        {label && <span className="mb-1.5 block text-[13px] font-medium text-fg">{label}</span>}
        <textarea ref={ref} id={inputId} className={cn(base, "min-h-24 py-2", className)} {...rest} />
      </label>
    );
  }
);

export const Select = forwardRef<HTMLSelectElement, SelectHTMLAttributes<HTMLSelectElement> & { label?: string }>(
  function Select({ label, className, id, children, ...rest }, ref) {
    const inputId = id ?? (label ? label.toLowerCase().replace(/\s+/g, "-") : undefined);
    return (
      <label className="block" htmlFor={inputId}>
        {label && <span className="mb-1.5 block text-[13px] font-medium text-fg">{label}</span>}
        <span className="relative block">
          <select ref={ref} id={inputId} className={cn(base, "h-10 appearance-none pr-9", className)} {...rest}>
            {children}
          </select>
          <ChevronDown className="pointer-events-none absolute right-3 top-1/2 h-4 w-4 -translate-y-1/2 text-muted" />
        </span>
      </label>
    );
  }
);

export function SearchField({ className, ...rest }: InputHTMLAttributes<HTMLInputElement>) {
  return (
    <input
      type="search"
      className={cn(base, "h-9 rounded-pill bg-bg-alt px-4 focus:bg-card pointer-coarse:h-11", className)}
      {...rest}
    />
  );
}
