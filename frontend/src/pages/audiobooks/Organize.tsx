// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Navigate, useNavigate, useParams, useSearchParams } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ImageUp, Pencil, Plus, RotateCcw, Trash2 } from "lucide-react";
import {
  addBookToCollection,
  createCollection,
  createSeries,
  deleteCollection,
  deleteSeries,
  listCollectionBooks,
  listCollections,
  listSeries,
  removeBookFromCollection,
  removeBookFromSeries,
  updateCollection,
  updateSeries,
} from "../../api/collections";
import {
  createAuthor,
  deleteAuthor,
  deleteAuthorImage,
  getAuthorBooks,
  listAuthors,
  updateAuthor,
  uploadAuthorImage,
} from "../../api/authors";
import { listBooks } from "../../api/audiobooks";
import { formatDuration } from "../../lib/format";
import { playBook } from "../../lib/play";
import { apiErrorMessage } from "../../lib/apiError";
import { MediaRow } from "../../components/library/MediaCard";
import { AuthorAvatar } from "../../components/library/AuthorAvatar";
import { Button, Dialog, DialogContent, EmptyState, IconButton, Input, Skeleton, Textarea, toast } from "../../components/ui";
import { FilterField } from "../../components/library/BrowseControls";
import type { AudioBook } from "../../api/types";
import { useT } from "../../i18n";

/*
 * Series, collections and authors — the audiobook library's own groupings. They used to be a
 * separate "Organize" page in the sidebar, which read as if it organised the whole library; since
 * 2026-09-30 they are chips on the Audiobooks page (Series, Collections, and the author headings of
 * "By author"), and this file holds the pieces that page uses.
 */
export type GroupMode = "collections" | "series";

export function NewGroupDialog({ mode, onClose }: { mode: "collections" | "series"; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: async () => {
      const body = { name: name.trim(), description: description.trim() || undefined };
      if (mode === "collections") await createCollection(body);
      else await createSeries(body);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: [mode] });
      toast.success(mode === "collections" ? t("audiobooks.collection.created") : t("audiobooks.series.created"));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("audiobooks.organize.errorCreate"))),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={mode === "collections" ? t("audiobooks.collection.new") : t("audiobooks.series.new")}
        description={
          mode === "collections"
            ? t("audiobooks.collection.about")
            : t("audiobooks.series.about")
        }
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button onClick={() => create.mutate()} loading={create.isPending} disabled={!name.trim()}>
              {t("audiobooks.organize.create")}
            </Button>
          </>
        }
      >
        <Input label={t("audiobooks.organize.name")} autoFocus value={name} onChange={(e) => setName(e.target.value)} />
        <div className="mt-3">
          <Textarea label={t("audiobooks.field.description")} value={description} onChange={(e) => setDescription(e.target.value)} placeholder={t("audiobooks.optional")} />
        </div>
        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}

export function RenameDialog({
  mode,
  id,
  initialName,
  initialDescription,
  onClose,
}: {
  mode: "collections" | "series";
  id: string;
  initialName: string;
  initialDescription: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { t } = useT();
  const [name, setName] = useState(initialName);
  const [description, setDescription] = useState(initialDescription);

  const save = useMutation({
    mutationFn: async () => {
      const body = { name: name.trim(), description: description.trim() || undefined };
      if (mode === "collections") await updateCollection(id, body);
      else await updateSeries(id, body);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: [mode] });
      toast.success(t("audiobooks.organize.saved"));
      onClose();
    },
    onError: () => toast.error(t("audiobooks.organize.errorSave")),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={mode === "collections" ? t("audiobooks.collection.edit") : t("audiobooks.series.edit")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!name.trim()}>
              {t("common.action.save")}
            </Button>
          </>
        }
      >
        <Input label={t("audiobooks.organize.name")} autoFocus value={name} onChange={(e) => setName(e.target.value)} />
        <div className="mt-3">
          <Textarea label={t("audiobooks.field.description")} value={description} onChange={(e) => setDescription(e.target.value)} />
        </div>
      </DialogContent>
    </Dialog>
  );
}

export function AuthorDialog({
  author,
  initialName,
  onClose,
}: {
  author?: { id: string; name: string; sort_name: string | null; bio: string | null };
  /** Creating a catalogue entry for a name books already carry ("By author" heading). */
  initialName?: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { t } = useT();
  const [name, setName] = useState(author?.name ?? initialName ?? "");
  const [sortName, setSortName] = useState(author?.sort_name ?? "");
  const [bio, setBio] = useState(author?.bio ?? "");
  const [error, setError] = useState<string | null>(null);

  const save = useMutation({
    mutationFn: async () => {
      const body = { name: name.trim(), sort_name: sortName.trim() || undefined, bio: bio.trim() || undefined };
      if (author) await updateAuthor(author.id, body);
      else await createAuthor(body);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["authors"] });
      toast.success(author ? t("audiobooks.organize.saved") : t("audiobooks.organize.added"));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("audiobooks.organize.errorSave"))),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={author ? t("audiobooks.person.edit") : t("audiobooks.person.add")}
        description={t("audiobooks.person.about")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>{t("common.action.cancel")}</Button>
            <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!name.trim()}>{t("common.action.save")}</Button>
          </>
        }
      >
        <div className="grid gap-3 sm:grid-cols-2">
          <Input label={t("audiobooks.credits.name")} autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={t("audiobooks.person.namePlaceholder")} />
          <Input
            label={t("audiobooks.person.sortAs")}
            value={sortName}
            onChange={(e) => setSortName(e.target.value)}
            placeholder={t("audiobooks.person.sortAsPlaceholder")}
            hint={t("audiobooks.person.sortAsHint")}
          />
        </div>
        <div className="mt-3">
          <Textarea label={t("audiobooks.person.bio")} value={bio} onChange={(e) => setBio(e.target.value)} placeholder={t("audiobooks.optional")} />
        </div>
        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}

export function AuthorDetail({ authorId, onDeleted }: { authorId: string; onDeleted: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [editing, setEditing] = useState(false);
  // Bumped after a photo upload/reset so the avatar remounts and re-fetches —
  // its src (the fetch-and-cache route) never changes, so nothing else would
  // tell AuthImage the picture underneath it is different now.
  const [avatarVersion, setAvatarVersion] = useState(0);
  const { data: authors = [] } = useQuery({ queryKey: ["authors"], queryFn: listAuthors });
  const { data: books = [], isLoading } = useQuery({ queryKey: ["author-books", authorId], queryFn: () => getAuthorBooks(authorId) });
  const { data: allBooks = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const author = authors.find((a) => a.id === authorId);

  const remove = useMutation({
    mutationFn: () => deleteAuthor(authorId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["authors"] });
      toast.success(t("audiobooks.person.removed"), t("audiobooks.organize.booksUntouched"));
      onDeleted();
    },
    onError: () => toast.error(t("audiobooks.person.errorRemove")),
  });

  const uploadPhoto = useMutation({
    mutationFn: (file: File) => uploadAuthorImage(authorId, file),
    onSuccess: () => setAvatarVersion((v) => v + 1),
    onError: () => toast.error(t("audiobooks.person.errorPhoto")),
  });

  const resetPhoto = useMutation({
    mutationFn: () => deleteAuthorImage(authorId),
    onSuccess: () => {
      setAvatarVersion((v) => v + 1);
      toast.success(t("audiobooks.person.photoReset"));
    },
    onError: () => toast.error(t("audiobooks.person.errorPhotoReset")),
  });

  if (!author) return <div className="p-6"><Skeleton className="h-24" /></div>;

  return (
    <div className="p-6">
      <div className="flex flex-wrap items-start gap-3">
        <div className="group relative">
          <AuthorAvatar key={avatarVersion} src={author.image_url} alt={author.name} className="h-16 w-16" />
          <label className="absolute inset-0 flex cursor-pointer items-center justify-center rounded-full bg-black/50 text-white opacity-0 transition-opacity group-hover:opacity-100">
            <ImageUp className="h-5 w-5" />
            <input
              type="file"
              accept="image/*"
              hidden
              onChange={(e) => {
                const file = e.target.files?.[0];
                if (file) uploadPhoto.mutate(file);
                e.target.value = "";
              }}
            />
          </label>
        </div>
        <div className="min-w-0 flex-1">
          <h1 className="text-2xl font-semibold tracking-tight">{author.name}</h1>
          {author.sort_name && <p className="mt-0.5 text-xs text-muted">{t("audiobooks.person.sortsAs", { name: author.sort_name })}</p>}
          {author.bio && <p className="mt-2 max-w-2xl text-sm leading-relaxed text-muted">{author.bio}</p>}
        </div>
        <IconButton
          size="sm"
          label={t("audiobooks.person.useAutomaticPhoto")}
          disabled={resetPhoto.isPending}
          onClick={() => resetPhoto.mutate()}
        >
          <RotateCcw className="h-4 w-4" />
        </IconButton>
        <IconButton size="sm" label={t("audiobooks.organize.editNamed", { name: author.name })} onClick={() => setEditing(true)}>
          <Pencil className="h-4 w-4" />
        </IconButton>
        <IconButton size="sm" label={t("audiobooks.organize.removeNamed", { name: author.name })} onClick={() => remove.mutate()}>
          <Trash2 className="h-4 w-4" />
        </IconButton>
      </div>

      <div className="mt-5">
        {isLoading ? (
          <Skeleton className="h-24" />
        ) : books.length === 0 ? (
          <EmptyState title={t("audiobooks.person.noBooks")} description={t("audiobooks.person.noBooksHint")} />
        ) : (
          books.map((b) => {
            const full = allBooks.find((x) => x.id === b.id);
            return (
              <MediaRow
                key={b.id}
                kind="audiobook"
                title={b.title}
                subtitle={b.author}
                cover={b.cover_url}
                trailing={b.total_duration_secs ? formatDuration(b.total_duration_secs) : undefined}
                onClick={() => full && void playBook(full)}
                onPlay={full ? () => void playBook(full) : undefined}
              />
            );
          })
        )}
      </div>

      {editing && <AuthorDialog author={author} onClose={() => setEditing(false)} />}
    </div>
  );
}

function AddBooksDialog({ collectionId, onClose }: { collectionId: string; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [filter, setFilter] = useState("");
  const { data: books = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const { data: inCollection = [] } = useQuery({
    queryKey: ["collection-books", collectionId],
    queryFn: () => listCollectionBooks(collectionId),
  });
  const already = new Set(inCollection.map((b) => b.id));

  // One request per book — there is no bulk add.
  const add = useMutation({
    mutationFn: (bookId: string) => addBookToCollection(collectionId, bookId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["collection-books", collectionId] });
      qc.invalidateQueries({ queryKey: ["collections"] });
    },
    onError: () => toast.error(t("audiobooks.collection.errorAddBook")),
  });

  const q = filter.trim().toLowerCase();
  const visible = books.filter((b) => !q || b.title.toLowerCase().includes(q) || (b.author ?? "").toLowerCase().includes(q));

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent title={t("audiobooks.collection.addBooks")} className="sm:max-w-xl">
        <FilterField value={filter} onChange={setFilter} placeholder={t("audiobooks.collection.searchBooks")} />
        <ul className="mt-3 max-h-96 space-y-0.5 overflow-y-auto">
          {visible.map((b) => (
            <li key={b.id} className="flex items-center gap-3 rounded-lg px-2 py-1.5 hover:bg-bg-alt">
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm">{b.title}</span>
                {b.author && <span className="block truncate text-xs text-muted">{b.author}</span>}
              </span>
              <Button
                size="sm"
                variant={already.has(b.id) ? "secondary" : "primary"}
                disabled={already.has(b.id)}
                onClick={() => add.mutate(b.id)}
              >
                {already.has(b.id) ? t("audiobooks.collection.added") : t("common.action.add")}
              </Button>
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}

export function CollectionDetail({ collectionId, onDeleted }: { collectionId: string; onDeleted: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [adding, setAdding] = useState(false);
  const { data: collections = [] } = useQuery({ queryKey: ["collections"], queryFn: listCollections });
  const { data: books = [], isLoading } = useQuery({
    queryKey: ["collection-books", collectionId],
    queryFn: () => listCollectionBooks(collectionId),
  });
  const { data: allBooks = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const collection = collections.find((c) => c.id === collectionId);

  const removeBook = useMutation({
    mutationFn: (bookId: string) => removeBookFromCollection(collectionId, bookId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["collection-books", collectionId] });
      qc.invalidateQueries({ queryKey: ["collections"] });
    },
  });

  const remove = useMutation({
    mutationFn: () => deleteCollection(collectionId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["collections"] });
      toast.success(t("audiobooks.collection.deleted"), t("audiobooks.organize.booksUntouched"));
      onDeleted();
    },
  });

  if (isLoading || !collection) return <div className="space-y-3 p-6"><Skeleton className="h-8 w-48" /><Skeleton className="h-40" /></div>;

  return (
    <div className="p-6">
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h1 className="text-2xl font-semibold tracking-tight">{collection.name}</h1>
          {collection.description && <p className="mt-1 text-sm text-muted">{collection.description}</p>}
          <p className="mt-1 text-xs text-muted">{t("audiobooks.bookCount", { count: books.length })}</p>
        </div>
        <Button size="sm" icon={<Plus className="h-4 w-4" />} onClick={() => setAdding(true)}>
          {t("audiobooks.collection.addBooks")}
        </Button>
        <IconButton size="sm" label={t("audiobooks.collection.delete")} onClick={() => remove.mutate()}>
          <Trash2 className="h-4 w-4" />
        </IconButton>
      </div>

      <div className="mt-5">
        {books.length === 0 ? (
          <EmptyState
            title={t("audiobooks.collection.emptyTitle")}
            description={t("audiobooks.collection.emptyHint")}
            action={<Button onClick={() => setAdding(true)}>{t("audiobooks.collection.addBooks")}</Button>}
          />
        ) : (
          books.map((b) => {
            const full = allBooks.find((x) => x.id === b.id);
            return (
              <MediaRow
                key={b.id}
                kind="audiobook"
                title={b.title}
                subtitle={b.author}
                cover={b.cover_url}
                trailing={b.total_duration_secs ? formatDuration(b.total_duration_secs) : undefined}
                onPlay={full ? () => void playBook(full as AudioBook) : undefined}
                onClick={() => full && void playBook(full as AudioBook)}
                menu={
                  <IconButton size="sm" label={t("audiobooks.organize.removeNamed", { name: b.title })} onClick={() => removeBook.mutate(b.id)}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </IconButton>
                }
              />
            );
          })
        )}
      </div>

      {adding && <AddBooksDialog collectionId={collectionId} onClose={() => setAdding(false)} />}
    </div>
  );
}

export function SeriesDetail({ seriesId, onDeleted }: { seriesId: string; onDeleted: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const { data: allSeries = [], isLoading } = useQuery({ queryKey: ["series"], queryFn: listSeries });
  const { data: allBooks = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks });
  const series = allSeries.find((s) => s.id === seriesId);

  const removeBook = useMutation({
    mutationFn: (bookId: string) => removeBookFromSeries(seriesId, bookId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["series"] }),
  });

  const remove = useMutation({
    mutationFn: () => deleteSeries(seriesId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["series"] });
      toast.success(t("audiobooks.series.deleted"), t("audiobooks.organize.booksUntouched"));
      onDeleted();
    },
  });

  if (isLoading || !series) return <div className="space-y-3 p-6"><Skeleton className="h-8 w-48" /><Skeleton className="h-40" /></div>;

  return (
    <div className="p-6">
      <div className="flex flex-wrap items-start gap-3">
        <div className="min-w-0 flex-1">
          <h1 className="text-2xl font-semibold tracking-tight">{series.name}</h1>
          {series.description && <p className="mt-1 text-sm text-muted">{series.description}</p>}
          <p className="mt-1 text-xs text-muted">{t("audiobooks.series.countInOrder", { count: series.books.length })}</p>
        </div>
        <IconButton size="sm" label={t("audiobooks.series.delete")} onClick={() => remove.mutate()}>
          <Trash2 className="h-4 w-4" />
        </IconButton>
      </div>

      <div className="mt-5">
        {series.books.length === 0 ? (
          <EmptyState title={t("audiobooks.series.emptyTitle")} description={t("audiobooks.series.emptyHint")} />
        ) : (
          series.books
            .slice()
            .sort((a, b) => a.position - b.position)
            .map((entry) => {
              const full = allBooks.find((x) => x.id === entry.book_id);
              return (
                <MediaRow
                  key={entry.book_id}
                  kind="audiobook"
                  index={entry.position}
                  title={entry.book_title}
                  subtitle={full?.author}
                  cover={full?.cover_url}
                  onPlay={full ? () => void playBook(full) : undefined}
                  onClick={() => full && void playBook(full)}
                  menu={
                    <IconButton size="sm" label={t("audiobooks.organize.removeNamed", { name: entry.book_title })} onClick={() => removeBook.mutate(entry.book_id)}>
                      <Trash2 className="h-3.5 w-3.5" />
                    </IconButton>
                  }
                />
              );
            })
        )}
      </div>
    </div>
  );
}

/** Collections, series, authors and favorites — the ways a library gets grouped
 *  that aren't the library itself. */
/** The Series or Collections chip on the Audiobooks page: the groups, a New button, and rename. */
export function GroupList({ mode, filter, selectedId }: { mode: GroupMode; filter: string; selectedId?: string }) {
  const navigate = useNavigate();
  const { t } = useT();
  const [creating, setCreating] = useState(false);
  const [renaming, setRenaming] = useState<{ id: string; name: string; description: string } | null>(null);
  const collections = useQuery({ queryKey: ["collections"], queryFn: listCollections, enabled: mode === "collections" });
  const series = useQuery({ queryKey: ["series"], queryFn: listSeries, enabled: mode === "series" });
  const q = filter.trim().toLowerCase();

  const rows = (mode === "collections" ? collections.data ?? [] : series.data ?? [])
    .map((g) => ({
      id: g.id,
      name: g.name,
      description: g.description ?? "",
      count: "book_count" in g ? g.book_count : g.books.length,
      cover: "cover_url" in g ? g.cover_url : null,
    }))
    .filter((g) => !q || g.name.toLowerCase().includes(q));
  const loading = mode === "collections" ? collections.isLoading : series.isLoading;

  return (
    <div className="p-3">
      <div className="mb-2 flex justify-end">
        <Button size="sm" variant="secondary" icon={<Plus className="h-4 w-4" />} onClick={() => setCreating(true)}>
          {t("audiobooks.organize.new")}
        </Button>
      </div>
      {loading && <Skeleton className="h-32" />}
      {!loading && rows.length === 0 && (
        <EmptyState
          title={q ? t("audiobooks.empty.noMatch.title") : t(mode === "collections" ? "audiobooks.organize.emptyCollections" : "audiobooks.organize.emptySeries")}
          description={q ? t("audiobooks.empty.noMatch.description") : t("audiobooks.organize.emptyHint")}
        />
      )}
      {rows.map((g) => (
        <MediaRow
          key={g.id}
          kind="audiobook"
          title={g.name}
          subtitle={t("audiobooks.bookCount", { count: g.count })}
          cover={g.cover}
          selected={g.id === selectedId}
          onClick={() => navigate(`/audiobooks/${mode}/${g.id}`)}
          menu={
            <IconButton size="sm" label={t("audiobooks.organize.editNamed", { name: g.name })} onClick={() => setRenaming(g)}>
              <Pencil className="h-3.5 w-3.5" />
            </IconButton>
          }
        />
      ))}
      {creating && <NewGroupDialog mode={mode} onClose={() => setCreating(false)} />}
      {renaming && (
        <RenameDialog
          mode={mode}
          id={renaming.id}
          initialName={renaming.name}
          initialDescription={renaming.description}
          onClose={() => setRenaming(null)}
        />
      )}
    </div>
  );
}

/** The old /audiobooks/organize links (bookmarks, other clients' deep links) land in the new place. */
export function OrganizeRedirect() {
  const { groupId } = useParams<{ groupId: string }>();
  const [params] = useSearchParams();
  const mode = params.get("mode");
  const base = mode === "series" ? "series" : mode === "authors" ? "authors" : "collections";
  return <Navigate to={groupId ? `/audiobooks/${base}/${groupId}` : "/audiobooks"} replace />;
}
