// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ArrowLeft, Search } from "lucide-react";
import {
  applyBookMetadata,
  searchBookMetadata,
  type BookCandidate,
  type IdentifyFields,
} from "../../api/audiobooks";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, Input, Pill, Skeleton, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import type { AudioBook } from "../../api/types";
import { useT, type PlainKey } from "../../i18n";

/* "Identify" on a book — Google Books search, then a per-field choice of what
   to actually write.

   The two steps are deliberate. Audiobookshelf's quick match applies the top
   result whole, which is how a book ends up with a study guide's description
   attached to it; here nothing is written until a candidate is picked and its
   fields confirmed. Search terms are editable and seeded from the book rather
   than fixed to it, because the books worth identifying are the ones whose
   stored title and author are wrong. */

/** Google Books describes the print edition — there is no narrator in this
    data, so the book's own narrator is never part of the choice. */
const FIELD_LABELS: { key: keyof IdentifyFields; label: PlainKey }[] = [
  { key: "title", label: "audiobooks.field.title" },
  { key: "author", label: "audiobooks.field.author" },
  { key: "description", label: "audiobooks.field.description" },
  { key: "publisher", label: "audiobooks.field.publisher" },
  { key: "published_year", label: "audiobooks.field.year" },
  { key: "isbn", label: "audiobooks.field.isbn" },
  { key: "cover", label: "audiobooks.field.cover" },
];

function CandidateRow({ candidate, onPick }: { candidate: BookCandidate; onPick: () => void }) {
  const [artOk, setArtOk] = useState(true);
  const { t } = useT();
  return (
    <li>
      <button
        type="button"
        onClick={onPick}
        className="flex w-full gap-3 rounded-card border border-border p-3 text-left hover:border-accent/60 hover:bg-accent/5"
      >
        {candidate.cover_url && artOk ? (
          <img
            src={candidate.cover_url}
            alt=""
            loading="lazy"
            onError={() => setArtOk(false)}
            className="h-20 w-14 shrink-0 rounded-lg object-cover"
          />
        ) : (
          <span className="h-20 w-14 shrink-0 rounded-lg bg-bg-alt" />
        )}
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium">{candidate.title}</span>
          {candidate.subtitle && <span className="block truncate text-xs text-muted">{candidate.subtitle}</span>}
          <span className="block truncate text-xs text-muted">{candidate.author ?? t("audiobooks.unknownAuthor")}</span>
          <span className="mt-1.5 flex flex-wrap gap-1.5 text-[11px] text-muted">
            {candidate.published_year && <Pill>{candidate.published_year}</Pill>}
            {candidate.publisher && <Pill>{candidate.publisher}</Pill>}
            {candidate.page_count != null && <Pill>{t("audiobooks.identify.pages", { count: candidate.page_count })}</Pill>}
            <Pill tone={candidate.score >= 80 ? "success" : "neutral"}>{t("audiobooks.identify.score", { score: candidate.score })}</Pill>
          </span>
        </span>
      </button>
    </li>
  );
}

/** Step two: what the match would change, field by field, with the current
    value beside the new one so an apply is never a guess. */
function Confirm({
  book,
  candidate,
  fields,
  toggle,
}: {
  book: AudioBook;
  candidate: BookCandidate;
  fields: Required<IdentifyFields>;
  toggle: (key: keyof IdentifyFields, value: boolean) => void;
}) {
  const { t } = useT();
  const proposed: Record<keyof IdentifyFields, { next: string | null; current: string | null }> = {
    title: { next: candidate.title, current: book.title },
    author: { next: candidate.author, current: book.author },
    description: { next: candidate.description, current: book.description },
    publisher: { next: candidate.publisher, current: book.publisher },
    published_year: {
      next: candidate.published_year != null ? String(candidate.published_year) : null,
      current: book.published_year != null ? String(book.published_year) : null,
    },
    isbn: { next: candidate.isbn, current: book.isbn },
    cover: {
      next: candidate.cover_url ? t("audiobooks.identify.newCover") : null,
      current: book.cover_url ? t("audiobooks.identify.currentCover") : null,
    },
  };

  return (
    <ul className="space-y-2">
      {FIELD_LABELS.map(({ key, label }) => {
        const { next, current } = proposed[key];
        const available = next != null && next !== "";
        return (
          <li key={key}>
            <label
              className={cn(
                "flex items-start gap-3 rounded-card border border-border p-3",
                !available && "opacity-50"
              )}
            >
              <input
                type="checkbox"
                checked={available && fields[key]}
                disabled={!available}
                onChange={(e) => toggle(key, e.target.checked)}
                className="mt-0.5 h-4 w-4 accent-[var(--accent)]"
              />
              <span className="min-w-0 flex-1">
                <span className="text-sm font-medium">{t(label)}</span>
                <span className="mt-0.5 block line-clamp-3 text-xs text-muted">
                  {available ? next : t("audiobooks.identify.nothingHere")}
                </span>
                {available && current && (
                  <span className="mt-0.5 block truncate text-xs text-muted/70">{t("audiobooks.identify.now", { value: current })}</span>
                )}
              </span>
            </label>
          </li>
        );
      })}
      <li className="px-1 pt-1 text-xs text-muted">{t("audiobooks.identify.narratorNote")}</li>
    </ul>
  );
}

export default function IdentifyBookSheet({ book, onClose }: { book: AudioBook; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [title, setTitle] = useState(book.title);
  const [author, setAuthor] = useState(book.author ?? "");
  const [results, setResults] = useState<BookCandidate[] | null>(null);
  const [picked, setPicked] = useState<BookCandidate | null>(null);
  const [fields, setFields] = useState<Required<IdentifyFields>>({
    title: true,
    author: true,
    description: true,
    publisher: true,
    published_year: true,
    isbn: true,
    cover: true,
  });
  const [error, setError] = useState<string | null>(null);

  const canSearch = title.trim().length > 0 || author.trim().length > 0;

  const search = useMutation({
    mutationFn: () =>
      searchBookMetadata(book.id, {
        title: title.trim() || undefined,
        author: author.trim() || undefined,
      }),
    onSuccess: (candidates) => {
      setResults(candidates);
      setError(null);
    },
    onError: (err) => setError(apiErrorMessage(err, t("audiobooks.identify.errorSearch"))),
  });

  const apply = useMutation({
    mutationFn: (candidate: BookCandidate) =>
      applyBookMetadata(book.id, { volume_id: candidate.volume_id, fields }),
    onSuccess: (updated) => {
      qc.invalidateQueries({ queryKey: ["book", book.id] });
      qc.invalidateQueries({ queryKey: ["books"] });
      toast.success(t("audiobooks.toast.bookUpdated"), updated.title);
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("audiobooks.identify.errorApply"))),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={picked ? t("audiobooks.identify.confirmTitle") : t("audiobooks.identify.title")}
        description={
          picked
            ? t("audiobooks.identify.confirmDescription")
            : t("audiobooks.identify.searchDescription")
        }
        className="sm:max-w-2xl"
        footer={
          picked ? (
            <>
              <Button variant="ghost" icon={<ArrowLeft className="h-4 w-4" />} onClick={() => setPicked(null)}>
                {t("common.action.back")}
              </Button>
              <Button loading={apply.isPending} onClick={() => apply.mutate(picked)}>
                {t("audiobooks.identify.apply")}
              </Button>
            </>
          ) : (
            <>
              <Button variant="ghost" onClick={onClose}>
                {t("common.action.close")}
              </Button>
              <Button
                icon={<Search className="h-4 w-4" />}
                loading={search.isPending}
                disabled={!canSearch}
                onClick={() => search.mutate()}
              >
                {t("common.action.search")}
              </Button>
            </>
          )
        }
      >
        {picked ? (
          <Confirm
            book={book}
            candidate={picked}
            fields={fields}
            toggle={(key, value) => setFields((f) => ({ ...f, [key]: value }))}
          />
        ) : (
          <>
            <div className="grid gap-3 sm:grid-cols-2">
              <Input label={t("audiobooks.field.title")} value={title} onChange={(e) => setTitle(e.target.value)} />
              <Input label={t("audiobooks.field.author")} value={author} onChange={(e) => setAuthor(e.target.value)} />
            </div>

            {search.isPending && <Skeleton className="mt-4 h-40" />}

            {results && results.length === 0 && (
              <p className="mt-4 text-sm text-muted">{t("audiobooks.identify.noResults")}</p>
            )}

            {results && results.length > 0 && (
              <ul className="mt-4 space-y-2">
                {results.map((c) => (
                  <CandidateRow key={c.volume_id} candidate={c} onPick={() => setPicked(c)} />
                ))}
              </ul>
            )}
          </>
        )}

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
