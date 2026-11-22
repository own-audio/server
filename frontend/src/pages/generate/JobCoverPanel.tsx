// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getBook } from "../../api/audiobooks";
import { getDefaultCoverPrompt, setJobCover, type CoverMode } from "../../api/generation";
import type { GenerationJobStatus } from "../../api/types";
import { Button, Cover, Textarea, toast } from "../../components/ui";
import { forgetAuthImage } from "../../lib/authImages";
import { cn } from "../../lib/cn";
import { coverChangeMessage } from "../../lib/paidWork";
import { coverActions } from "../../lib/jobCover";
import { useT } from "../../i18n";

/**
 * A finished narration's cover, changed after the fact: a new generated one (free, a bounded
 * number per book), the file's own, or none. Only on a server that reports
 * `cover_regenerations_left` — older servers have no such route.
 */
export default function JobCoverPanel({ job }: { job: GenerationJobStatus & { book_id: string } }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [generate, setGenerate] = useState<"auto" | "custom">(job.cover_mode === "custom" ? "custom" : "auto");
  const [brief, setBrief] = useState<string | null>(null);
  const [showTitle, setShowTitle] = useState(false);
  const [error, setError] = useState<string | null>(null);
  // The cover's address stays the same after a change; a new query string makes the browser ask again.
  const [version, setVersion] = useState(0);

  const { data: book } = useQuery({ queryKey: ["book", job.book_id], queryFn: () => getBook(job.book_id) });
  const { data: defaultBrief } = useQuery({
    queryKey: ["cover-prompt"],
    queryFn: getDefaultCoverPrompt,
    staleTime: Infinity,
    enabled: generate === "custom",
  });
  const prompt = brief ?? defaultBrief ?? "";

  const change = useMutation({
    mutationFn: (mode: CoverMode) =>
      setJobCover(job.id, { coverMode: mode, coverPrompt: mode === "custom" ? prompt : undefined, coverShowTitle: showTitle }),
    onMutate: () => setError(null),
    onSuccess: (updated) => {
      qc.setQueryData(["generation-status", job.id], updated);
      // Made here lists the same job.
      qc.invalidateQueries({ queryKey: ["generation-jobs"] });
      if (book?.cover_url) forgetAuthImage(book.cover_url);
      qc.invalidateQueries({ queryKey: ["book", job.book_id] });
      qc.invalidateQueries({ queryKey: ["books"] });
      setVersion((v) => v + 1);
      toast.success(t("generate.cover.changed"));
    },
    onError: (err) => {
      setError(coverChangeMessage(err));
      // What is left may have changed meanwhile (a cover made in another tab, the book deleted).
      qc.invalidateQueries({ queryKey: ["generation-status", job.id] });
    },
  });

  const can = coverActions(job);
  const left = job.cover_regenerations_left ?? 0;
  const src = book?.cover_url ? `${book.cover_url}${book.cover_url.includes("?") ? "&" : "?"}v=${version}` : null;
  const busy = change.isPending;

  return (
    <section className="mt-5 border-t border-border pt-4" aria-labelledby="job-cover-title">
      <h2 id="job-cover-title" className="mb-3 text-sm font-semibold">{t("audiobooks.field.cover")}</h2>
      <div className="flex flex-col gap-4 sm:flex-row">
        <Cover key={src ?? "none"} kind="audiobook" src={src} alt={job.title} className="w-28 shrink-0" />

        <div className="min-w-0 flex-1">
          <div role="radiogroup" aria-label={t("generate.cover.regenerate")} className="flex flex-col gap-2 sm:flex-row">
            {(["auto", "custom"] as const).map((mode) => (
              <button
                key={mode}
                type="button"
                role="radio"
                aria-checked={generate === mode}
                disabled={busy}
                onClick={() => setGenerate(mode)}
                className={cn(
                  "flex-1 rounded-xl border p-2.5 text-left text-sm transition disabled:opacity-60",
                  generate === mode ? "border-accent bg-accent/6" : "border-border bg-card hover:border-accent"
                )}
              >
                <span className="block font-semibold">{t(mode === "auto" ? "generate.cover.auto" : "generate.cover.custom")}</span>
                <span className="block text-xs text-muted">{t(mode === "auto" ? "generate.cover.autoHint" : "generate.cover.customHint")}</span>
              </button>
            ))}
          </div>

          {generate === "custom" && (
            <div className="mt-3">
              <Textarea label={t("generate.cover.prompt")} rows={4} value={prompt} disabled={busy} onChange={(e) => setBrief(e.target.value)} />
              {defaultBrief && prompt !== defaultBrief && (
                <button type="button" onClick={() => setBrief(null)} className="mt-1.5 text-xs text-accent-text hover:underline">
                  {t("generate.cover.resetBrief")}
                </button>
              )}
            </div>
          )}

          <label className="mt-3 flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={showTitle}
              disabled={busy}
              onChange={(e) => setShowTitle(e.target.checked)}
              className="h-4 w-4 rounded accent-[var(--accent)]"
            />
            {t("generate.cover.printTitle")}
          </label>

          <div className="mt-3 flex flex-wrap items-center gap-2">
            <Button
              size="sm"
              onClick={() => change.mutate(generate)}
              disabled={!can.regenerate || busy || (generate === "custom" && !prompt.trim())}
              loading={busy && change.variables !== "book" && change.variables !== "none"}
            >
              {t("generate.cover.regenerate")}
            </Button>
            {can.useBook && (
              <Button size="sm" variant="secondary" onClick={() => change.mutate("book")} disabled={busy} loading={busy && change.variables === "book"}>
                {t("generate.cover.useBook")}
              </Button>
            )}
            {can.remove && (
              <Button size="sm" variant="ghost" onClick={() => change.mutate("none")} disabled={busy} loading={busy && change.variables === "none"}>
                {t("generate.cover.remove")}
              </Button>
            )}
          </div>
          <p className="mt-2 text-xs text-muted">
            {left > 0 ? t("generate.cover.regenerateLeft", { count: left }) : t("generate.cover.error.limitReached")}
          </p>

          {busy && change.variables !== "none" && change.variables !== "book" && (
            <p role="status" className="mt-2 text-xs text-muted">{t("generate.cover.working")}</p>
          )}
          {error && <p role="alert" className="mt-2 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
        </div>
      </div>
    </section>
  );
}
