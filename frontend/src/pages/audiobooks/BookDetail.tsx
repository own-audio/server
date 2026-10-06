// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { arrayMove, SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, Heart, Pause, Pencil, Play, RotateCcw, ScanSearch, Trash2, UserCheck, UserX, Users } from "lucide-react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import {
  deleteBookFile,
  getBook,
  listBookChapters,
  listBookFiles,
  reorderBookFiles,
  setBookVisibility,
  updateFileTitle,
} from "../../api/audiobooks";
import type { AudioBookFile } from "../../api/types";
import { listBookProgress, resetBookProgress } from "../../api/playback";
import { addFavorite, checkFavorite, removeFavorite } from "../../api/collections";
import { getFamily, isFamilyAdmin } from "../../api/family";
import { formatDuration } from "../../lib/format";
import { playBook } from "../../lib/play";
import { usePlayerStore } from "../../store/playerStore";
import { Button, IconButton, Cover, Skeleton, Pill, Dialog, DialogContent, MenuItem, toast } from "../../components/ui";
import { MoreMenuTrigger } from "../../components/library/BrowseControls";
import { ClampedText } from "../../components/library/ClampedText";
import AudienceList from "../../components/library/AudienceList";
import EditBookSheet from "./EditBookSheet";
import IdentifyBookSheet from "./IdentifyBookSheet";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

function FileRow({
  file,
  offset,
  isPlayingFile,
  playing,
  canManage,
  editing,
  editingTitle,
  onEditingTitleChange,
  onStartRename,
  onCommitRename,
  onCancelRename,
  onDelete,
  onPlay,
  renaming,
}: {
  file: AudioBookFile;
  offset: number;
  isPlayingFile: boolean;
  playing: boolean;
  canManage: boolean;
  editing: boolean;
  editingTitle: string;
  onEditingTitleChange: (title: string) => void;
  onStartRename: () => void;
  onCommitRename: () => void;
  onCancelRename: () => void;
  onDelete: () => void;
  onPlay: () => void;
  renaming: boolean;
}) {
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: file.id });
  const { t } = useT();
  const name = file.title ?? t("audiobooks.file.part", { n: file.position });
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={cn(
        "flex items-center gap-1 border-b border-border px-1 py-1 last:border-b-0 sm:gap-2 sm:px-3 sm:py-1.5",
        isPlayingFile && "bg-accent/8",
        isDragging && "opacity-60"
      )}
    >
      {canManage && (
        <button
          {...attributes}
          {...listeners}
          aria-label={t("audiobooks.file.reorder", { title: name })}
          className="flex h-10 w-8 shrink-0 cursor-grab touch-none items-center justify-center text-muted hover:text-fg active:cursor-grabbing sm:w-6"
        >
          <GripVertical className="h-4 w-4" />
        </button>
      )}
      <span className="w-6 shrink-0 text-right text-xs tabular-nums text-muted">{file.position}</span>
      {editing ? (
        <input
          autoFocus
          value={editingTitle}
          disabled={renaming}
          onChange={(e) => onEditingTitleChange(e.target.value)}
          onBlur={onCommitRename}
          onKeyDown={(e) => {
            if (e.key === "Enter") onCommitRename();
            if (e.key === "Escape") onCancelRename();
          }}
          className="min-w-0 flex-1 rounded-lg border border-border bg-bg px-2 py-1 text-sm outline-none focus:border-accent"
        />
      ) : (
        <div className="min-w-0 flex-1 py-1.5">
          {/* A phone gives the name two lines and puts the length under it; its rename and
              delete move into the row's menu, so the name is not squeezed to a few letters. */}
          <p className={cn("line-clamp-2 text-sm sm:truncate", isPlayingFile && "text-accent")}>{name}</p>
          {file.duration_secs != null && <p className="text-xs tabular-nums text-muted sm:hidden">{formatDuration(file.duration_secs)}</p>}
        </div>
      )}
      {file.duration_secs != null && <span className="shrink-0 text-xs tabular-nums text-muted max-sm:hidden">{formatDuration(file.duration_secs)}</span>}
      {canManage && !editing && (
        <IconButton size="sm" label={t("audiobooks.file.rename")} onClick={onStartRename} className="max-sm:hidden">
          <Pencil className="h-4 w-4" />
        </IconButton>
      )}
      {canManage && (
        <IconButton size="sm" label={t("audiobooks.file.delete")} onClick={onDelete} className="max-sm:hidden">
          <Trash2 className="h-4 w-4" />
        </IconButton>
      )}
      <IconButton size="sm" label={isPlayingFile && playing ? t("common.action.pause") : t("audiobooks.file.play")} onClick={onPlay}>
        {isPlayingFile && playing ? <Pause className="h-4 w-4 fill-current" /> : <Play className="h-4 w-4 fill-current" />}
      </IconButton>
      {canManage && !editing && (
        <span className="sm:hidden">
          <MoreMenuTrigger>
            <MenuItem icon={<Pencil />} onSelect={onStartRename}>{t("audiobooks.file.rename")}</MenuItem>
            <MenuItem icon={<Trash2 />} destructive onSelect={onDelete}>{t("audiobooks.file.delete")}</MenuItem>
          </MoreMenuTrigger>
        </span>
      )}
      <span className="sr-only">{offset}</span>
    </li>
  );
}

/* The detail column for one book: header, files, and a chapter list when the
   book actually has distinct in-book chapter positions. Nothing extracts
   chapters from M4B/ID3 server-side yet (client guide §12), so an empty
   chapter list is normal, not an error. */
export default function BookDetail({ bookId }: { bookId: string }) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [identifying, setIdentifying] = useState(false);
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const { t } = useT();
  const partName = (f: AudioBookFile) => f.title ?? t("audiobooks.file.part", { n: f.position });

  const { data: book, isLoading } = useQuery({ queryKey: ["book", bookId], queryFn: () => getBook(bookId) });
  const { data: serverFiles = [] } = useQuery({ queryKey: ["book-files", bookId], queryFn: () => listBookFiles(bookId) });
  // Set for the duration of a drag/drop, so the list can reflect the new order before the
  // server confirms it; cleared once the reorder mutation settles and serverFiles catches up.
  const [order, setOrder] = useState<AudioBookFile[] | null>(null);
  const files = order ?? serverFiles;
  const { data: chapters = [] } = useQuery({ queryKey: ["book-chapters", bookId], queryFn: () => listBookChapters(bookId) });
  // Shares the shelf's cache entry, and answers in positions measured from the
  // start of the book. The per-book endpoint reports where you are inside the
  // current *file*, which drew "3% listened" over a book someone was halfway through.
  const { data: bookProgress = [] } = useQuery({ queryKey: ["book-progress"], queryFn: listBookProgress });
  const progress = bookProgress.find((p) => p.book_id === bookId);
  const { data: fav } = useQuery({ queryKey: ["book-favorite", bookId], queryFn: () => checkFavorite(bookId) });
  const { data: family } = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false });

  const [audienceOpen, setAudienceOpen] = useState(false);
  const [editingFileId, setEditingFileId] = useState<string | null>(null);
  const [editingTitle, setEditingTitle] = useState("");

  const renameFile = useMutation({
    mutationFn: ({ fileId, title }: { fileId: string; title: string }) => updateFileTitle(bookId, fileId, title),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book-files", bookId] });
      setEditingFileId(null);
    },
    onError: () => toast.error(t("audiobooks.file.errorRename")),
  });

  function startRename(file: AudioBookFile) {
    setEditingFileId(file.id);
    setEditingTitle(partName(file));
  }

  function commitRename(file: AudioBookFile) {
    const title = editingTitle.trim();
    // Nothing worth sending back, and the server has no way to turn a title back into the
    // "Part N" fallback other than being told that literally — reverting silently beats a 400.
    if (!title || title === partName(file)) {
      setEditingFileId(null);
      return;
    }
    renameFile.mutate({ fileId: file.id, title });
  }

  const sensors = useSensors(
    useSensor(PointerSensor, { activationConstraint: { distance: 4 } }),
    useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates })
  );

  const reorder = useMutation({
    mutationFn: (fileIds: string[]) => reorderBookFiles(bookId, fileIds),
    onSuccess: (updated) => {
      qc.setQueryData(["book-files", bookId], updated);
      setOrder(null);
    },
    onError: () => {
      toast.error(t("audiobooks.file.errorReorder"));
      setOrder(null);
      qc.invalidateQueries({ queryKey: ["book-files", bookId] });
    },
  });

  function onDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    const from = files.findIndex((f) => f.id === active.id);
    const to = files.findIndex((f) => f.id === over.id);
    const next = arrayMove(files, from, to);
    setOrder(next);
    reorder.mutate(next.map((f) => f.id));
  }

  const [deletingFile, setDeletingFile] = useState<AudioBookFile | null>(null);
  const deleteFile = useMutation({
    mutationFn: (fileId: string) => deleteBookFile(bookId, fileId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book-files", bookId] });
      qc.invalidateQueries({ queryKey: ["book", bookId] });
      qc.invalidateQueries({ queryKey: ["books"] });
      qc.invalidateQueries({ queryKey: ["book-progress"] });
      toast.success(t("audiobooks.file.deleted"));
      setDeletingFile(null);
    },
    onError: () => toast.error(t("audiobooks.file.errorDelete")),
  });

  // Sharing lived only in the library card's overflow menu and inside the edit sheet, both of
  // which you have to know about. This is the screen you are on when you wonder who can hear it.
  const changeVisibility = useMutation({
    mutationFn: (v: "private" | "family") => setBookVisibility(bookId, v),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book", bookId] });
      qc.invalidateQueries({ queryKey: ["books"] });
      qc.invalidateQueries({ queryKey: ["audience", "audiobook", bookId] });
    },
  });

  const toggleFav = useMutation({
    mutationFn: () => (fav?.is_favorite ? removeFavorite(bookId) : addFavorite(bookId)),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book-favorite", bookId] });
      qc.invalidateQueries({ queryKey: ["favorites"] });
    },
  });

  const restart = useMutation({
    mutationFn: () => resetBookProgress(bookId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book-progress"] });
      toast.success(t("audiobooks.toast.restarted"));
    },
    onError: () => toast.error(t("audiobooks.error.restart")),
  });

  const offsets = useMemo(() => {
    const out: number[] = [];
    files.reduce((run, f) => { out.push(run); return run + (f.duration_secs ?? 0); }, 0);
    return out;
  }, [files]);
  const offsetByFile = useMemo(() => new Map(files.map((f, i) => [f.id, offsets[i] ?? 0])), [files, offsets]);

  // In a multi-file book each chapter *is* a file, so the file list already
  // shows them; a separate TOC only helps when they differ.
  const showChapters = chapters.length > 0 && chapters.length !== files.length;

  // Owner, or a family admin when the book is shared with their family — same line the server
  // draws for identify and file renames, so a control never appears here only to 403 on click.
  const canManage = !!book?.is_owner || (isFamilyAdmin(family?.my_role) && book?.visibility === "family");

  const isCurrent = current?.kind === "audiobook" && current.bookId === bookId;
  const pct = progress && book?.total_duration_secs ? Math.min(100, Math.round((progress.position_secs / book.total_duration_secs) * 100)) : null;
  const remaining = progress && book?.total_duration_secs ? Math.max(0, Math.round(book.total_duration_secs - progress.position_secs)) : null;

  const actions: { key: string; label: string; icon: ReactNode; onSelect: () => void; active?: boolean; disabled?: boolean }[] = [];
  if (book && progress && (progress.position_secs > 0 || progress.completed))
    actions.push({ key: "restart", label: t("audiobooks.action.startOver"), icon: <RotateCcw className="h-4 w-4" />, disabled: restart.isPending, onSelect: () => restart.mutate() });
  if (book?.is_owner)
    actions.push({ key: "edit", label: t("audiobooks.detail.editDetails"), icon: <Pencil className="h-4 w-4" />, onSelect: () => setEditing(true) });
  if (book?.is_owner)
    actions.push({
      key: "visibility",
      label: book.visibility === "family" ? t("audiobooks.action.makePrivate") : t("audiobooks.action.shareWithFamily"),
      icon: book.visibility === "family" ? <UserX className="h-4 w-4" /> : <Users className="h-4 w-4" />,
      active: book.visibility === "family",
      disabled: changeVisibility.isPending,
      onSelect: () => changeVisibility.mutate(book.visibility === "family" ? "private" : "family"),
    });
  // Reachable without owning the book: the edit sheet, where this list also lives, is the
  // owner's alone, so an admin administering someone else's shared book had no way in at all.
  if (book && isFamilyAdmin(family?.my_role) && book.visibility === "family")
    actions.push({ key: "audience", label: t("audiobooks.detail.whoCanHear"), icon: <UserCheck className="h-4 w-4" />, onSelect: () => setAudienceOpen(true) });
  // Identifying is administration of shared content, not ownership of it: a family admin tidying
  // a book someone else uploaded is the case this exists for. The server draws the same line,
  // so a private book stays its owner's alone.
  if (canManage)
    actions.push({ key: "identify", label: t("audiobooks.identify.title"), icon: <ScanSearch className="h-4 w-4" />, onSelect: () => setIdentifying(true) });

  if (isLoading || !book) {
    return <div className="space-y-4 p-6"><Skeleton className="h-44 w-32" /><Skeleton className="h-6 w-64" /><Skeleton className="h-24" /></div>;
  }

  return (
    <div className="pb-8">
      {/* On a phone: a big centred cover, the title under it, and one full-width play button. */}
      <div className="flex flex-col items-center gap-5 px-4 pb-6 pt-2 text-center sm:flex-row sm:items-start sm:p-6 sm:text-left">
        <Cover kind="audiobook" src={book.cover_url} alt={book.title} className="w-48 shrink-0 shadow-card sm:w-40" />
        <div className="w-full min-w-0 flex-1">
          <h1 className="text-2xl font-semibold leading-tight tracking-tight">{book.title}</h1>
          {book.author && <p className="mt-1 text-sm text-muted">{book.author}</p>}
          {book.narrator && <p className="text-sm text-muted">{t("audiobooks.detail.narratedBy", { name: book.narrator })}</p>}

          <div className="mt-3 flex flex-wrap items-center justify-center gap-2 text-xs text-muted sm:justify-start">
            {book.total_duration_secs != null && <Pill>{formatDuration(book.total_duration_secs)}</Pill>}
            <Pill>{t("audiobooks.detail.fileCount", { count: files.length })}</Pill>
            {progress?.completed ? <Pill tone="success">{t("audiobooks.status.finished")}</Pill> : pct != null && pct > 0 ? <Pill tone="accent">{t("audiobooks.status.progress", { progress: pct / 100, remaining: formatDuration(remaining ?? 0) })}</Pill> : null}
            {book.visibility === "family" && <Pill tone="accent">{t("audiobooks.visibility.shared")}</Pill>}
          </div>

          <div className="mt-5 flex items-center gap-2 sm:mt-4 sm:flex-wrap">
            <Button
              size="lg"
              className="max-sm:flex-1"
              icon={isCurrent && playing ? <Pause className="h-4 w-4 fill-current" /> : <Play className="h-4 w-4 fill-current" />}
              onClick={() => void playBook(book)}
              disabled={files.length === 0}
            >
              {isCurrent && playing ? t("common.action.pause") : progress && progress.position_secs > 0 && !progress.completed ? t("common.action.resume") : t("common.action.play")}
            </Button>
            <IconButton
              label={fav?.is_favorite ? t("audiobooks.detail.removeFavorite") : t("audiobooks.detail.addFavorite")}
              active={fav?.is_favorite}
              onClick={() => toggleFav.mutate()}
            >
              <Heart className={cn("h-5 w-5", fav?.is_favorite && "fill-current")} />
            </IconButton>
            {/* Icons beside the play button on a wide screen; on a phone a row of five unlabelled
                icons wrapped onto a second line, so there they are a menu with words. */}
            {actions.map((a) => (
              <IconButton key={a.key} label={a.label} active={a.active} disabled={a.disabled} onClick={a.onSelect} className="max-sm:hidden">
                {a.icon}
              </IconButton>
            ))}
            {actions.length > 0 && (
              <span className="sm:hidden">
                <MoreMenuTrigger size="lg">
                  {actions.map((a) => (
                    <MenuItem key={a.key} icon={a.icon} disabled={a.disabled} onSelect={a.onSelect}>{a.label}</MenuItem>
                  ))}
                </MoreMenuTrigger>
              </span>
            )}
          </div>

          {book.description && <ClampedText text={book.description} className="mt-5" />}

          {/* Print-edition facts, filled in by the identify flow. */}
          {(book.publisher || book.published_year || book.isbn) && (
            <p className="mt-3 text-left text-xs text-muted">
              {[book.publisher, book.published_year, book.isbn && t("audiobooks.detail.isbn", { isbn: book.isbn })].filter(Boolean).join(" · ")}
            </p>
          )}
        </div>
      </div>

      <section className="px-3 sm:px-6">
        <h2 className="mb-2 px-1 text-sm font-semibold uppercase tracking-wide text-muted sm:px-0">{t("audiobooks.detail.files")}</h2>
        {canManage && files.length > 1 && <p className="mb-2 px-1 text-xs text-muted sm:px-0">{t("audiobooks.detail.dragHint")}</p>}
        <ul className="overflow-hidden rounded-card border border-border">
          <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
            <SortableContext items={files.map((f) => f.id)} strategy={verticalListSortingStrategy}>
              {files.map((file, i) => (
                <FileRow
                  key={file.id}
                  file={file}
                  offset={offsets[i]}
                  isPlayingFile={current?.kind === "audiobook" && current.fileId === file.id}
                  playing={playing}
                  canManage={canManage}
                  editing={editingFileId === file.id}
                  editingTitle={editingTitle}
                  onEditingTitleChange={setEditingTitle}
                  onStartRename={() => startRename(file)}
                  onCommitRename={() => commitRename(file)}
                  onCancelRename={() => setEditingFileId(null)}
                  onDelete={() => setDeletingFile(file)}
                  onPlay={() => void playBook(book, file.id)}
                  renaming={renameFile.isPending}
                />
              ))}
            </SortableContext>
          </DndContext>
          {files.length === 0 && <li className="px-3 py-6 text-center text-sm text-muted">{t("audiobooks.detail.noFiles")}</li>}
        </ul>
      </section>

      {showChapters && (
        <section className="mt-6 px-3 sm:px-6">
          <h2 className="mb-2 text-sm font-semibold uppercase tracking-wide text-muted">{t("audiobooks.detail.chapters")}</h2>
          <ul className="overflow-hidden rounded-card border border-border">
            {chapters.map((ch) => (
              <li key={ch.id} className="flex items-center gap-3 border-b border-border px-3 py-2 last:border-b-0">
                <span className="w-6 shrink-0 text-right text-xs tabular-nums text-muted">{ch.position}</span>
                <p className="min-w-0 flex-1 truncate text-sm">{ch.title}</p>
                <span className="shrink-0 text-xs tabular-nums text-muted">
                  {formatDuration(Math.round((ch.file_id ? offsetByFile.get(ch.file_id) ?? 0 : 0) + ch.start_time_secs))}
                </span>
              </li>
            ))}
          </ul>
        </section>
      )}
      <Dialog open={!!deletingFile} onOpenChange={(v) => !v && setDeletingFile(null)}>
        <DialogContent
          title={deletingFile ? t("audiobooks.file.confirmDelete.title", { title: partName(deletingFile) }) : ""}
          footer={
            <>
              <Button variant="ghost" onClick={() => setDeletingFile(null)}>
                {t("common.action.cancel")}
              </Button>
              <Button
                variant="danger"
                loading={deleteFile.isPending}
                onClick={() => deletingFile && deleteFile.mutate(deletingFile.id)}
              >
                {t("common.action.delete")}
              </Button>
            </>
          }
        >
          <p className="text-sm text-muted">{t("audiobooks.file.confirmDelete.body")}</p>
        </DialogContent>
      </Dialog>
      <Dialog open={audienceOpen} onOpenChange={setAudienceOpen}>
        {audienceOpen && (
          <DialogContent title={t("audiobooks.detail.whoCanHear")} variant="sheet">
            <div className="px-6 pb-6">
              <AudienceList kind="audiobook" itemId={bookId} />
            </div>
          </DialogContent>
        )}
      </Dialog>
      {editing && <EditBookSheet bookId={bookId} onClose={() => setEditing(false)} />}
      {identifying && <IdentifyBookSheet book={book} onClose={() => setIdentifying(false)} />}
    </div>
  );
}
