// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode } from "react";
import Logo, { Wordmark } from "../Logo";
import ThemeToggle from "../shell/ThemeToggle";
import { useT } from "../../i18n";

/* Shared frame for the signed-out screens: brand panel on wide screens, one
   card on narrow ones. Copy uses construction D from the brand brief — a
   plain sentence, no wordplay where an instruction is needed. */
export default function AuthLayout({ children, aside }: { children: ReactNode; aside?: ReactNode }) {
  const { t } = useT();
  return (
    <div className="flex min-h-dvh bg-bg text-fg">
      <aside className="hidden w-[42%] max-w-xl flex-col justify-between border-r border-border bg-bg-alt p-10 lg:flex">
        <div className="flex items-center gap-2.5">
          <Logo size={40} />
          <Wordmark className="text-xl" />
        </div>
        <div>
          <p className="text-3xl font-semibold leading-tight tracking-tight">{t("auth.tagline")}</p>
          <p className="mt-3 max-w-sm text-[15px] text-muted">
            {t("auth.brandPitch")}
          </p>
          {aside}
        </div>
        <ThemeToggle binary />
      </aside>

      <main className="relative flex min-w-0 flex-1 flex-col">
        <div className="absolute right-4 top-4 sm:right-6 lg:hidden">
          <ThemeToggle binary compact />
        </div>
        <div className="flex flex-1 items-center justify-center px-4 py-6 sm:px-6 lg:py-0">
          <div className="w-full min-w-0 max-w-sm">
            <div className="mb-8 flex flex-col items-center text-center lg:hidden">
              <Logo size={64} />
              <Wordmark className="mt-3 text-2xl" />
              <p className="mt-1.5 text-sm text-muted">{t("auth.tagline")}</p>
            </div>
            {children}
          </div>
        </div>
      </main>
    </div>
  );
}

export function AuthHeading({ title, subtitle }: { title: string; subtitle?: string }) {
  return (
    <div className="mb-6 text-center lg:mb-7 lg:text-left">
      <h1 className="text-lg font-semibold tracking-tight lg:text-2xl">{title}</h1>
      {subtitle && <p className="mt-1.5 text-sm text-muted">{subtitle}</p>}
    </div>
  );
}

export function FormError({ children }: { children: ReactNode }) {
  if (!children) return null;
  return (
    <p role="alert" className="rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">
      {children}
    </p>
  );
}
