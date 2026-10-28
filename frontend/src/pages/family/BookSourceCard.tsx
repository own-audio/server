// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BookOpen } from "lucide-react";
import { getBookSource, setBookSource, type BookSource } from "../../api/family";
import { apiErrorMessage } from "../../lib/apiError";
import { useServerFeatures } from "../../lib/features";
import { Button, PasswordInput, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

const SOURCES: BookSource[] = ["openlibrary", "apple", "google"];

/**
 * Where "Identify this book" looks. Only on a server that offers the choice
 * (the open-source edition): keyless Google Books is turned away nearly
 * everywhere, so the family starts on Open Library and an admin can switch.
 *
 * The Google key is write-only here. The server stores it encrypted and only
 * ever says whether one is set, so the field starts empty and an empty field
 * on save keeps the key that is there.
 */
export default function BookSourceCard({ isAdmin }: { isAdmin: boolean }) {
  const { t } = useT();
  const qc = useQueryClient();
  const { features } = useServerFeatures();
  const { data: status } = useQuery({
    queryKey: ["book-source"],
    queryFn: getBookSource,
    enabled: features.book_source_choice,
  });

  const [picked, setPicked] = useState<BookSource | null>(null);
  const [key, setKey] = useState("");

  const save = useMutation({
    mutationFn: (req: { source: BookSource; google_books_api_key?: string | null }) => setBookSource(req),
    onSuccess: (next) => {
      qc.setQueryData(["book-source"], next);
      setPicked(null);
      setKey("");
      toast.success(t("family.bookSource.saved"));
    },
    onError: (err) => toast.error(t("family.error.change"), apiErrorMessage(err, t("family.tryAgain"))),
  });

  if (!features.book_source_choice || !status?.configurable) return null;

  const source = picked ?? status.source;
  const hasKey = status.google_key_set || status.server_google_key;
  const changed = source !== status.source || key.trim() !== "";
  const needsKey = source === "google" && !hasKey && key.trim() === "";

  return (
    <section className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold uppercase tracking-wide text-muted">
        <BookOpen className="h-4 w-4" /> {t("family.bookSource.title")}
      </h2>

      <div className="rounded-card border border-border p-4">
        <p className="text-sm text-muted">{t("family.bookSource.body")}</p>

        <div role="radiogroup" aria-label={t("family.bookSource.title")} className="mt-3 grid grid-cols-1 gap-2 sm:grid-cols-3">
          {SOURCES.map((s) => (
            <button
              key={s}
              type="button"
              role="radio"
              aria-checked={source === s}
              disabled={!isAdmin}
              onClick={() => setPicked(s)}
              className={cn(
                "rounded-card border p-3 text-left transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
                source === s ? "border-accent bg-accent/5" : "border-border",
                isAdmin && source !== s && "hover:border-accent/60"
              )}
            >
              <span className="block text-sm font-medium">{t(`family.bookSource.${s}.name`)}</span>
              <span className="mt-0.5 block text-xs text-muted">{t(`family.bookSource.${s}.body`)}</span>
            </button>
          ))}
        </div>

        {isAdmin && source === "google" && (
          <div className="mt-4">
            <PasswordInput
              label={t("family.bookSource.key")}
              value={key}
              onChange={(e) => setKey(e.target.value)}
              autoComplete="off"
              placeholder={status.google_key_set ? t("family.bookSource.keyKept") : undefined}
              hint={
                status.google_key_set
                  ? t("family.bookSource.keySet")
                  : status.server_google_key
                    ? t("family.bookSource.keyServer")
                    : t("family.bookSource.keyNeeded")
              }
            />
            {status.google_key_set && (
              <button
                type="button"
                className="mt-2 text-xs text-muted underline-offset-2 hover:text-fg hover:underline"
                disabled={save.isPending}
                onClick={() => save.mutate({ source: status.server_google_key ? source : "openlibrary", google_books_api_key: null })}
              >
                {t("family.bookSource.removeKey")}
              </button>
            )}
          </div>
        )}

        {isAdmin ? (
          <div className="mt-4 flex justify-end">
            <Button
              disabled={!changed || needsKey || save.isPending}
              onClick={() => save.mutate({ source, ...(key.trim() ? { google_books_api_key: key.trim() } : {}) })}
            >
              {t("common.action.save")}
            </Button>
          </div>
        ) : (
          <p className="mt-3 text-xs text-muted">{t("family.bookSource.adminOnly")}</p>
        )}
      </div>
    </section>
  );
}
