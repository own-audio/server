// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { moveToTrash } from "../../lib/trash";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { arrayMove, SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, ImageUp, Trash2 } from "lucide-react";
import {
  deleteBook,
  getBook,
  listBookFiles,
  reorderBookFiles,
  setBookVisibility,
  updateBook,
  uploadBookCover,
} from "../../api/audiobooks";
import { formatDuration } from "../../lib/format";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, Input, Textarea, Skeleton, toast } from "../../components/ui";
import { VisibilityField } from "../../components/library/VisibilityField";
import AudienceList from "../../components/library/AudienceList";
import { BookAuthorsField, BookTagsField } from "./BookAuthorsAndTags";
import type { AudioBookFile, Visibility } from "../../api/types";
import { useT } from "../../i18n";

function SortableFile({ file }: { file: AudioBookFile }) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: file.id });
  const { t } = useT();
  const name = file.title ?? t("audiobooks.file.part", { n: file.position });
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`flex items-center gap-2 border-b border-border bg-card px-2 py-2 last:border-b-0 ${isDragging ? "opacity-60" : ""}`}
    >
      <button
        {...attributes}
        {...listeners}
        aria-label={t("audiobooks.file.reorder", { title: name })}
        className="cursor-grab text-muted hover:text-fg active:cursor-grabbing"
      >
        <GripVertical className="h-4 w-4" />
      </button>
      <span className="w-6 shrink-0 text-right text-xs tabular-nums text-muted">{file.position}</span>
      <span className="min-w-0 flex-1 truncate text-sm">{name}</span>
      {file.duration_secs != null && (
        <span className="shrink-0 text-xs tabular-nums text-muted">{formatDuration(file.duration_secs)}</span>
      )}
    </li>
  );
}

export default function EditBookSheet({ bookId, onClose }: { bookId: string; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const { data: book, isLoading } = useQuery({ queryKey: ["book", bookId], queryFn: () => getBook(bookId) });
  const { data: serverFiles = [] } = useQuery({ queryKey: ["book-files", bookId], queryFn: () => listBookFiles(bookId) });

  const [draft, setDraft] = useState<{ title: string; author: string; narrator: string; description: string } | null>(null);
  const [order, setOrder] = useState<AudioBookFile[] | null>(null);
  const [cover, setCover] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);

  const files = order ?? serverFiles;
  const form = draft ?? {
    title: book?.title ?? "",
    author: book?.author ?? "",
    narrator: book?.narrator ?? "",
    description: book?.description ?? "",
  };
  const edit = (patch: Partial<typeof form>) => setDraft({ ...form, ...patch });

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));

  function onDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    const from = files.findIndex((f) => f.id === active.id);
    const to = files.findIndex((f) => f.id === over.id);
    setOrder(arrayMove(files, from, to));
  }

  const save = useMutation({
    mutationFn: async () => {
      await updateBook(bookId, {
        title: form.title.trim(),
        author: form.author.trim() || undefined,
        narrator: form.narrator.trim() || undefined,
        description: form.description.trim() || undefined,
      });
      if (cover) await uploadBookCover(bookId, cover);
      // Play order is whatever this list says; the server renumbers positions.
      if (order) await reorderBookFiles(bookId, order.map((f) => f.id));
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book", bookId] });
      qc.invalidateQueries({ queryKey: ["book-files", bookId] });
      qc.invalidateQueries({ queryKey: ["books"] });
      toast.success(t("audiobooks.toast.bookUpdated"));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("audiobooks.edit.errorSave"))),
  });

  const changeVisibility = useMutation({
    mutationFn: (v: Visibility) => setBookVisibility(bookId, v),
    onSuccess: (_d, v) => {
      qc.invalidateQueries({ queryKey: ["book", bookId] });
      qc.invalidateQueries({ queryKey: ["books"] });
      toast.success(v === "family" ? t("audiobooks.visibility.shared") : t("audiobooks.visibility.madePrivate"));
    },
    onError: (err) => setError(apiErrorMessage(err, t("audiobooks.error.visibility"))),
  });

  const [trashing, setTrashing] = useState(false);
  async function trash(title: string) {
    setTrashing(true);
    try {
      await moveToTrash({
        title,
        remove: (batch) => deleteBook(bookId, batch),
        onChanged: () => qc.invalidateQueries({ queryKey: ["books"] }),
      });
      onClose();
    } catch (err) {
      setError(apiErrorMessage(err, t("audiobooks.edit.errorTrash")));
      setTrashing(false);
    }
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("audiobooks.edit.title")}
        className="sm:max-w-2xl"
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!form.title.trim()}>
              {t("common.action.save")}
            </Button>
          </>
        }
      >
        {isLoading || !book ? (
          <Skeleton className="h-64" />
        ) : (
          <>
            <div className="grid gap-3 sm:grid-cols-2">
              <Input label={t("audiobooks.field.title")} value={form.title} onChange={(e) => edit({ title: e.target.value })} />
              <Input label={t("audiobooks.field.author")} value={form.author} onChange={(e) => edit({ author: e.target.value })} />
              <Input label={t("audiobooks.field.narrator")} value={form.narrator} onChange={(e) => edit({ narrator: e.target.value })} />
              <label className="block">
                <span className="mb-1.5 block text-[13px] font-medium text-fg">{t("audiobooks.field.cover")}</span>
                <label className="flex h-10 cursor-pointer items-center gap-2 rounded-[10px] border border-border bg-card px-3 text-sm text-muted hover:text-fg">
                  <ImageUp className="h-4 w-4" />
                  <span className="truncate">{cover ? cover.name : t("audiobooks.edit.chooseImage")}</span>
                  <input type="file" accept="image/*" hidden onChange={(e) => setCover(e.target.files?.[0] ?? null)} />
                </label>
              </label>
            </div>

            <div className="mt-3">
              <Textarea label={t("audiobooks.field.description")} value={form.description} onChange={(e) => edit({ description: e.target.value })} />
            </div>

            <div className="mt-5 space-y-5 border-t border-border pt-5">
              <BookAuthorsField bookId={bookId} />
              <BookTagsField bookId={bookId} />
            </div>

            <div className="mt-5 border-t border-border pt-5">
              <VisibilityField
                value={book.visibility}
                disabled={!book.is_owner || changeVisibility.isPending}
                onChange={(v) => changeVisibility.mutate(v)}
              />
            </div>

            <div className="mt-4">
              <p className="mb-1.5 text-[13px] font-medium">{t("audiobooks.detail.whoCanHear")}</p>
              <AudienceList kind="audiobook" itemId={bookId} />
            </div>

            {files.length > 1 && (
              <div className="mt-5">
                <p className="mb-1.5 text-[13px] font-medium">{t("audiobooks.edit.order")}</p>
                <p className="mb-2 text-xs text-muted">{t("audiobooks.detail.dragHint")}</p>
                <ul className="overflow-hidden rounded-card border border-border">
                  <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
                    <SortableContext items={files.map((f) => f.id)} strategy={verticalListSortingStrategy}>
                      {files.map((f) => (
                        <SortableFile key={f.id} file={f} />
                      ))}
                    </SortableContext>
                  </DndContext>
                </ul>
              </div>
            )}

            {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}

            {book.is_owner && (
              <div className="mt-6 border-t border-border pt-4">
                <Button size="sm" variant="ghost" icon={<Trash2 className="h-4 w-4" />} className="text-error" loading={trashing} onClick={() => void trash(book.title)}>
                  {t("audiobooks.menu.moveToTrash")}
                </Button>
              </div>
            )}
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}
