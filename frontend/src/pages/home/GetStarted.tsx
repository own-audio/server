// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Link } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { CheckCircle2, ChevronRight, Circle, X } from "lucide-react";
import { listBooks } from "../../api/audiobooks";
import { listFeeds } from "../../api/podcasts";
import { listTracks } from "../../api/music";
import { getFamily, isFamilyAdmin } from "../../api/family";
import { formatMicro, getBilling } from "../../api/billing";
import { useAuthStore } from "../../store/authStore";
import { IconButton } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";
import { useServerFeatures } from "../../lib/features";

interface Step {
  title: PlainKey;
  description: PlainKey;
  to: string;
  done: boolean;
}

const dismissKey = (userId: string) => `audio2.getStarted.dismissed.${userId}`;

function readDismissed(userId: string | undefined): boolean {
  if (!userId) return false;
  try {
    return localStorage.getItem(dismissKey(userId)) === "1";
  } catch {
    return false;
  }
}

/* A new account lands on a Home of empty widgets. This points at the first
   things worth doing; each step ticks itself off from the library rather than
   from a click, so it is never out of step with what is really there. It is a
   card, not a gate — nothing waits on it. */
export function GetStarted() {
  const { t } = useT();
  const user = useAuthStore((s) => s.user);
  const [dismissed, setDismissed] = useState(() => readDismissed(user?.id));

  const feeds = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });
  const books = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const tracks = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks });
  const family = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false });
  const { features } = useServerFeatures();
  const billing = useQuery({ queryKey: ["billing"], queryFn: getBilling, retry: false, enabled: features.billing });

  if (dismissed || !user) return null;
  if (feeds.isLoading || books.isLoading || tracks.isLoading || family.isLoading) return null;

  const canUpload = family.data?.my_can_upload ?? true;
  const steps: Step[] = [];
  if (canUpload) {
    steps.push(
      { title: "home.getStarted.follow.title", description: "home.getStarted.follow.description", to: "/podcasts/add", done: (feeds.data?.length ?? 0) > 0 },
      { title: "home.getStarted.book.title", description: "home.getStarted.book.description", to: "/audiobooks/upload", done: (books.data?.length ?? 0) > 0 },
      { title: "home.getStarted.music.title", description: "home.getStarted.music.description", to: "/music/upload", done: (tracks.data?.length ?? 0) > 0 },
    );
  }
  if (isFamilyAdmin(family.data?.my_role)) {
    steps.push({
      title: "home.getStarted.family.title",
      description: "home.getStarted.family.description",
      to: "/family",
      done: (family.data?.members.length ?? 0) > 1,
    });
  }

  const remaining = steps.filter((s) => !s.done).length;
  if (steps.length === 0 || remaining === 0) return null;

  function dismiss() {
    try {
      localStorage.setItem(dismissKey(user!.id), "1");
    } catch {
      // storage unavailable — it just comes back next visit
    }
    setDismissed(true);
  }

  const balance = billing.data?.balance_micro ?? 0;

  return (
    <section className="mb-9 rounded-card border border-border bg-card p-4 sm:p-5" aria-labelledby="get-started">
      <div className="flex items-start gap-3">
        <div className="min-w-0 flex-1">
          <h2 id="get-started" className="text-lg font-semibold tracking-tight">
            {t("home.getStarted.title")}
          </h2>
          <p className="mt-0.5 text-sm text-muted">
            {t("home.getStarted.progress", { done: steps.length - remaining, total: steps.length })}
            {balance > 0 && billing.data && <> · {t("home.getStarted.credit", { amount: formatMicro(balance, billing.data.pricing.currency) })}</>}
          </p>
        </div>
        <IconButton label={t("home.getStarted.hide")} size="sm" onClick={dismiss} className="-mr-1 -mt-1 text-muted">
          <X className="h-4 w-4" />
        </IconButton>
      </div>

      <ul className="mt-3 divide-y divide-border">
        {steps.map((step) => (
          <li key={step.title}>
            <Link to={step.to} className="group -mx-2 flex items-center gap-3 rounded-[10px] px-2 py-3 hover:bg-bg-alt">
              {step.done ? (
                <CheckCircle2 className="h-5 w-5 shrink-0 text-accent" aria-label={t("home.getStarted.done")} />
              ) : (
                <Circle className="h-5 w-5 shrink-0 text-muted" aria-hidden="true" />
              )}
              <span className="min-w-0 flex-1">
                <span className={cn("block text-sm font-medium", step.done && "text-muted line-through")}>{t(step.title)}</span>
                {!step.done && <span className="block text-xs text-muted">{t(step.description)}</span>}
              </span>
              {!step.done && <ChevronRight className="h-4 w-4 shrink-0 text-muted transition-transform group-hover:translate-x-0.5" />}
            </Link>
          </li>
        ))}
      </ul>
    </section>
  );
}
