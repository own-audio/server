// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Plus, X } from "lucide-react";
import {
  createAuthor,
  linkBookAuthor,
  listAllTags,
  listAuthors,
  listBookAuthors,
  listBookTags,
  setBookTags,
  unlinkBookAuthor,
} from "../../api/authors";
import { Button, IconButton, Input, Select, Skeleton, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";

/* `role` is free text in the schema, with a documented convention rather than
   a constraint. Offering the three the migration names keeps the data tidy
   without pretending the server would reject anything else. */
const ROLES = ["author", "narrator", "editor"] as const;

const ROLE_LABELS: Record<(typeof ROLES)[number], PlainKey> = {
  author: "audiobooks.role.author",
  narrator: "audiobooks.role.narrator",
  editor: "audiobooks.role.editor",
};

/** Any other role came from somewhere else and is shown as stored. */
const roleKey = (role: string): PlainKey | null =>
  (ROLES as readonly string[]).includes(role) ? ROLE_LABELS[role as (typeof ROLES)[number]] : null;

export function BookAuthorsField({ bookId }: { bookId: string }) {
  const qc = useQueryClient();
  const { t } = useT();
  const roleLabel = (role: string) => { const k = roleKey(role); return k ? t(k) : role; };
  const [adding, setAdding] = useState(false);
  const [name, setName] = useState("");
  const [role, setRole] = useState<string>("author");

  const { data: linked = [], isLoading } = useQuery({
    queryKey: ["book-authors", bookId],
    queryFn: () => listBookAuthors(bookId),
  });
  const { data: all = [] } = useQuery({ queryKey: ["authors"], queryFn: listAuthors, enabled: adding });

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["book-authors", bookId] });
    qc.invalidateQueries({ queryKey: ["authors"] });
  };

  const add = useMutation({
    mutationFn: async () => {
      const typed = name.trim();
      // Reuse an author who already exists rather than creating a duplicate
      // that differs only by capitalisation.
      const existing = all.find((a) => a.name.toLowerCase() === typed.toLowerCase());
      const author = existing ?? (await createAuthor({ name: typed }));
      await linkBookAuthor(bookId, author.id, role);
    },
    onSuccess: () => {
      invalidate();
      setName("");
      setAdding(false);
    },
    onError: () => toast.error(t("audiobooks.credits.errorAdd")),
  });

  const remove = useMutation({
    mutationFn: ({ authorId, r }: { authorId: string; r: string }) => unlinkBookAuthor(bookId, authorId, r),
    onSuccess: invalidate,
    onError: () => toast.error(t("audiobooks.credits.errorRemove")),
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    if (name.trim()) add.mutate();
  }

  return (
    <div>
      <p className="text-[13px] font-medium">{t("audiobooks.credits.title")}</p>
      <p className="mt-0.5 text-xs text-muted">{t("audiobooks.credits.hint")}</p>

      {isLoading ? (
        <Skeleton className="mt-2 h-10" />
      ) : (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {linked.map((a) => (
            <span
              key={`${a.author_id}-${a.role}`}
              className="inline-flex items-center gap-1.5 rounded-pill bg-bg-alt py-1 pl-3 pr-1 text-sm"
            >
              {a.author_name}
              <span className="text-xs text-muted">{roleLabel(a.role)}</span>
              <IconButton
                size="sm"
                label={t("audiobooks.credits.remove", { name: a.author_name, role: roleLabel(a.role) })}
                className="h-6 w-6"
                onClick={() => remove.mutate({ authorId: a.author_id, r: a.role })}
              >
                <X className="h-3 w-3" />
              </IconButton>
            </span>
          ))}
          {linked.length === 0 && <span className="text-sm text-muted">{t("audiobooks.credits.none")}</span>}
        </div>
      )}

      {adding ? (
        <form onSubmit={submit} className="mt-2 flex flex-wrap items-end gap-2">
          <Input
            label={t("audiobooks.credits.name")}
            autoFocus
            list="known-authors"
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-48"
          />
          <datalist id="known-authors">
            {all.map((a) => (
              <option key={a.id} value={a.name} />
            ))}
          </datalist>
          <Select label={t("audiobooks.credits.role")} value={role} onChange={(e) => setRole(e.target.value)} className="w-32">
            {ROLES.map((r) => (
              <option key={r} value={r}>
                {t(ROLE_LABELS[r])}
              </option>
            ))}
          </Select>
          <Button size="sm" type="submit" loading={add.isPending} disabled={!name.trim()}>
            {t("common.action.add")}
          </Button>
          <Button size="sm" variant="ghost" type="button" onClick={() => setAdding(false)}>
            {t("common.action.cancel")}
          </Button>
        </form>
      ) : (
        <Button size="sm" variant="secondary" className="mt-2" icon={<Plus className="h-4 w-4" />} onClick={() => setAdding(true)}>
          {t("audiobooks.credits.add")}
        </Button>
      )}
    </div>
  );
}

export function BookTagsField({ bookId }: { bookId: string }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [draft, setDraft] = useState("");

  const { data: tags = [], isLoading } = useQuery({ queryKey: ["book-tags", bookId], queryFn: () => listBookTags(bookId) });
  const { data: known = [] } = useQuery({ queryKey: ["all-tags"], queryFn: listAllTags });

  /* The endpoint takes the whole set, not an add or a remove — so both
     operations send the full list with one entry changed. */
  const save = useMutation({
    mutationFn: (next: string[]) => setBookTags(bookId, next),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book-tags", bookId] });
      qc.invalidateQueries({ queryKey: ["all-tags"] });
    },
    onError: () => toast.error(t("audiobooks.tags.errorSave")),
  });

  const names = tags.map((tag) => tag.name);

  function add(e: FormEvent) {
    e.preventDefault();
    const value = draft.trim();
    if (!value || names.some((n) => n.toLowerCase() === value.toLowerCase())) {
      setDraft("");
      return;
    }
    save.mutate([...names, value]);
    setDraft("");
  }

  return (
    <div>
      <p className="text-[13px] font-medium">{t("audiobooks.tags.title")}</p>
      <p className="mt-0.5 text-xs text-muted">{t("audiobooks.tags.hint")}</p>

      {isLoading ? (
        <Skeleton className="mt-2 h-10" />
      ) : (
        <div className="mt-2 flex flex-wrap gap-1.5">
          {tags.map((tag) => (
            <span key={tag.id} className="inline-flex items-center gap-1 rounded-pill bg-bg-alt py-1 pl-3 pr-1 text-sm">
              {tag.name}
              <IconButton
                size="sm"
                label={t("audiobooks.tags.remove", { name: tag.name })}
                className="h-6 w-6"
                onClick={() => save.mutate(names.filter((n) => n !== tag.name))}
              >
                <X className="h-3 w-3" />
              </IconButton>
            </span>
          ))}
          {tags.length === 0 && <span className="text-sm text-muted">{t("audiobooks.tags.none")}</span>}
        </div>
      )}

      <form onSubmit={add} className="mt-2 flex items-end gap-2">
        <Input
          label={t("audiobooks.tags.add")}
          list="known-tags"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          className={cn("w-48")}
        />
        <datalist id="known-tags">
          {known.map((tag) => (
            <option key={tag.id} value={tag.name} />
          ))}
        </datalist>
        <Button size="sm" type="submit" loading={save.isPending} disabled={!draft.trim()}>
          {t("common.action.add")}
        </Button>
      </form>
    </div>
  );
}
