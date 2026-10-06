// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, Library, Plus } from "lucide-react";
import { addBookToCollection, createCollection, listCollectionBooks, listCollections } from "../../api/collections";
import { runWithLimit } from "../../lib/uploadQueue";
import { Button, Cover, Dialog, DialogContent, EmptyState, Input, SearchField, Skeleton, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/* Adding N books is N requests — there is no bulk add.
 *
 * Sequential, like playlist adds, but for a different reason. A playlist
 * rejects parallel adds outright (`UNIQUE (playlist_id, position)`); a
 * collection accepts them — its key is `(collection_id, book_id)` — and
 * quietly gives every book the same position, because they all read the same
 * `MAX(position)` before any of them writes. Adding six books four at a time
 * landed all six at position 1, leaving the collection in arbitrary order.
 * Nothing is lost, but the order the user picked is. */
const CONCURRENCY = 1;

/**
 * Put a selection of books into a collection, creating one on the spot if the
 * right collection doesn't exist yet.
 *
 * The Organize section already does this in the other direction — open a
 * collection, then go find books. This is the direction you want when you have
 * the books in front of you and have just selected them.
 */
export default function AddToCollectionDialog({
  bookIds,
  label,
  onClose,
  onDone,
}: {
  bookIds: string[];
  /** What is being added when it is one book (its title); a count is shown otherwise. */
  label?: string;
  onClose: () => void;
  onDone?: () => void;
}) {
  const qc = useQueryClient();
  const { t } = useT();
  const [filter, setFilter] = useState("");
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const { data: collections = [], isLoading } = useQuery({ queryKey: ["collections"], queryFn: listCollections });

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase();
    return q ? collections.filter((c) => c.name.toLowerCase().includes(q)) : collections;
  }, [collections, filter]);

  async function addTo(collectionId: string, collectionName: string) {
    setBusy(collectionId);
    const { failed } = await runWithLimit(bookIds, CONCURRENCY, (id) => addBookToCollection(collectionId, id));
    qc.invalidateQueries({ queryKey: ["collections"] });
    qc.invalidateQueries({ queryKey: ["collection-books", collectionId] });
    setBusy(null);

    const added = bookIds.length - failed.length;
    if (failed.length === 0) {
      toast.success(
        bookIds.length === 1
          ? t("audiobooks.addToCollection.addedOne", { name: collectionName })
          : t("audiobooks.addToCollection.addedMany", { count: added, name: collectionName })
      );
      onDone?.();
      onClose();
    } else {
      toast.error(
        t("audiobooks.addToCollection.partial", { added, total: bookIds.length }),
        t("audiobooks.addToCollection.partialHint")
      );
    }
  }

  const create = useMutation({
    mutationFn: () => createCollection({ name: newName.trim() }),
    onSuccess: async (c) => {
      qc.invalidateQueries({ queryKey: ["collections"] });
      await addTo(c.id, c.name);
    },
    onError: () => toast.error(t("audiobooks.addToCollection.errorCreate")),
  });

  function submitNew(e: FormEvent) {
    e.preventDefault();
    if (newName.trim()) create.mutate();
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("audiobooks.addToCollection.title")}
        description={bookIds.length === 1 && label ? label : t("audiobooks.addToCollection.selected", { count: bookIds.length })}
        footer={
          <Button variant="ghost" onClick={onClose}>
            {t("common.action.close")}
          </Button>
        }
      >
        <form onSubmit={submitNew} className="flex items-end gap-2">
          <Input
            label={t("audiobooks.collection.new")}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder={t("audiobooks.addToCollection.newPlaceholder")}
            className="flex-1"
          />
          <Button type="submit" icon={<Plus className="h-4 w-4" />} loading={create.isPending} disabled={!newName.trim()}>
            {t("audiobooks.addToCollection.createAndAdd")}
          </Button>
        </form>

        {collections.length > 0 && (
          <>
            <div className="my-4 flex items-center gap-3">
              <span className="h-px flex-1 bg-border" />
              <span className="text-xs text-muted">{t("audiobooks.addToCollection.orPick")}</span>
              <span className="h-px flex-1 bg-border" />
            </div>

            {collections.length > 6 && (
              <SearchField
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder={t("audiobooks.addToCollection.find")}
                className="mb-2 w-full"
              />
            )}
          </>
        )}

        {isLoading ? (
          <Skeleton className="h-32" />
        ) : shown.length === 0 ? (
          collections.length === 0 ? null : (
            <p className="py-4 text-center text-sm text-muted">{t("audiobooks.empty.noMatch.title")}</p>
          )
        ) : (
          <ul className="max-h-72 space-y-0.5 overflow-y-auto">
            {shown.map((c) => (
              <li key={c.id}>
                <CollectionRow
                  id={c.id}
                  name={c.name}
                  cover={c.cover_url}
                  bookCount={c.book_count}
                  bookIds={bookIds}
                  busy={busy === c.id}
                  disabled={busy !== null}
                  onAdd={() => addTo(c.id, c.name)}
                />
              </li>
            ))}
          </ul>
        )}

        {!isLoading && collections.length === 0 && (
          <EmptyState
            icon={<Library />}
            title={t("audiobooks.organize.emptyCollections")}
            description={t("audiobooks.addToCollection.emptyHint")}
            className="py-8"
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Says up front when everything being added is already in there, so a second
 *  click isn't needed to find out nothing changed. */
function CollectionRow({
  id,
  name,
  cover,
  bookCount,
  bookIds,
  busy,
  disabled,
  onAdd,
}: {
  id: string;
  name: string;
  cover: string | null;
  bookCount: number;
  bookIds: string[];
  busy: boolean;
  disabled: boolean;
  onAdd: () => void;
}) {
  const { t } = useT();
  const { data: existing } = useQuery({
    queryKey: ["collection-books", id],
    queryFn: () => listCollectionBooks(id),
    staleTime: 30_000,
  });

  const already = existing ? bookIds.filter((b) => existing.some((e) => e.id === b)).length : 0;
  const allPresent = already > 0 && already === bookIds.length;

  return (
    <button
      onClick={onAdd}
      disabled={disabled}
      className={cn(
        "flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors",
        "hover:bg-bg-alt disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      )}
    >
      <Cover kind="audiobook" src={cover} alt="" aspect="square" className="h-9 w-9 shrink-0 rounded-md" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{name}</span>
        <span className="block truncate text-xs text-muted">
          {allPresent
            ? t("audiobooks.addToCollection.allPresent", { count: bookCount })
            : already > 0
              ? t("audiobooks.addToCollection.somePresent", { count: bookCount, already })
              : t("audiobooks.bookCount", { count: bookCount })}
        </span>
      </span>
      {busy ? (
        <span className="text-xs text-muted">{t("audiobooks.addToCollection.adding")}</span>
      ) : allPresent ? (
        <Check className="h-4 w-4 shrink-0 text-success" />
      ) : (
        <Plus className="h-4 w-4 shrink-0 text-muted" />
      )}
    </button>
  );
}
