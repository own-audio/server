// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import {
  contentAudience,
  getFamily,
  isFamilyAdmin,
  setContentAudience,
  type AudienceEntry,
  type MediaKind,
} from "../../api/family";
import { Skeleton, toast } from "../ui";
import { cn } from "../../lib/cn";
import { useT, type Locale } from "../../i18n";

/** "Kornel, Bernard and Admin Tester" — never an Oxford comma, matching the
 *  rest of the console, which is why English is joined the British way. */
function joinNames(names: string[], locale: Locale): string {
  return new Intl.ListFormat(locale === "en" ? "en-GB" : locale, { type: "conjunction" }).format(names);
}

/**
 * "Who can actually hear this", and the place to change it.
 *
 * Visibility alone doesn't answer that: a family-shared book is still hidden
 * from a member whose access policy denies audiobooks, or who has a specific
 * deny grant for it. The server resolves both together and returns the real
 * answer per member, which is the only reason this exists rather than the UI
 * inferring it.
 *
 * The same answer is also the control. Family settings can already say what one
 * *person* may reach; an admin looking at a book is thinking the other way
 * round — who this *book* is for — and had no way to say it. Both write the
 * same grants, so the two screens cannot disagree.
 *
 * Defaults to "Everyone" collapsed rather than a flat roll call: most items are
 * never restricted, and the owner and every family admin can't be restricted at
 * all (`audio2_can_access` grants them unconditionally), so listing them as rows
 * next to the people an admin can actually toggle was mostly noise. They get one
 * summary line instead, and a per-person picker only appears once "Just some
 * people" is chosen.
 *
 * Family admins only — the endpoint refuses anyone else, so the section simply
 * doesn't render for a regular member.
 */
export default function AudienceList({ kind, itemId }: { kind: MediaKind; itemId: string }) {
  const qc = useQueryClient();
  const { t, locale } = useT();
  const { data: family } = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false });
  const admin = isFamilyAdmin(family?.my_role);

  const { data, isLoading } = useQuery({
    queryKey: ["audience", kind, itemId],
    queryFn: () => contentAudience(kind, itemId),
    enabled: admin,
    retry: false,
  });

  const save = useMutation({
    mutationFn: (canListen: string[]) => setContentAudience(kind, itemId, canListen),
    // The server's answer replaces the list rather than triggering a refetch: it already
    // resolved the rules, and a member who cannot be excluded must snap back visibly.
    onSuccess: (next) => qc.setQueryData(["audience", kind, itemId], next),
    onError: () => toast.error(t("library.audience.failed")),
  });

  // Sticky once entered: unchecking everyone back to "on" while picking would otherwise collapse
  // the picker out from under the admin mid-edit, right as they're looking at it.
  const [forceCustom, setForceCustom] = useState(false);

  if (!admin) return null;
  if (isLoading) return <Skeleton className="h-20" />;
  if (!data || data.length === 0) return null;

  const locked = data.filter((m) => m.locked);
  const togglable = data.filter((m) => !m.locked);
  const allOpen = togglable.every((m) => m.can_listen);
  const custom = forceCustom || !allOpen;
  const listeners = data.filter((m) => m.can_listen).length;

  function openToEveryone() {
    setForceCustom(false);
    save.mutate(togglable.map((m) => m.user_id));
  }

  function toggle(member: AudienceEntry) {
    if (member.locked) return;
    const next = togglable
      .filter((m) => (m.user_id === member.user_id ? !m.can_listen : m.can_listen))
      .map((m) => m.user_id);
    save.mutate(next);
  }

  return (
    <div>
      <p className="text-xs text-muted">
        {t("library.audience.summary", { listeners, total: data.length })}
      </p>

      {locked.length > 0 && (
        <p className="mt-1 text-xs text-muted">
          {t("library.audience.locked", {
            count: locked.length,
            names: joinNames(locked.map((m) => m.display_label ?? m.display_name), locale),
          })}
        </p>
      )}

      {togglable.length > 0 && (
        <>
          <div className="mt-2 inline-flex rounded-pill bg-bg-alt p-0.5">
            <button
              onClick={openToEveryone}
              disabled={save.isPending}
              className={cn(
                "rounded-pill px-2.5 py-1 text-xs font-medium transition-colors",
                !custom ? "bg-card text-fg shadow-card" : "text-muted hover:text-fg"
              )}
            >
              {t("library.audience.everyone")}
            </button>
            <button
              onClick={() => setForceCustom(true)}
              disabled={save.isPending}
              className={cn(
                "rounded-pill px-2.5 py-1 text-xs font-medium transition-colors",
                custom ? "bg-card text-fg shadow-card" : "text-muted hover:text-fg"
              )}
            >
              {t("library.audience.some")}
            </button>
          </div>

          {custom && (
            <ul className={cn("mt-2 divide-y divide-border rounded-card border border-border", save.isPending && "opacity-60")}>
              {togglable.map((m) => (
                <li key={m.user_id} className="flex items-center gap-2.5 px-3 py-2">
                  <input
                    type="checkbox"
                    checked={m.can_listen}
                    disabled={save.isPending}
                    onChange={() => toggle(m)}
                    aria-label={t("library.audience.let", { name: m.display_label ?? m.display_name })}
                    className="h-4 w-4 accent-[var(--accent)]"
                  />
                  <span className="min-w-0 flex-1 truncate text-sm">{m.display_label ?? m.display_name}</span>
                </li>
              ))}
            </ul>
          )}
        </>
      )}
    </div>
  );
}
