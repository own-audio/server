// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { useQueries, useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { CheckSquare, Library, Pencil, Play, Plus, RotateCcw, ScanSearch, UserPen, Users, UserX } from "lucide-react";
import { BookIcon } from "../../components/ui/CloudIcon";
import { deleteBook, listBooks, setBookVisibility } from "../../api/audiobooks";
import { listBookProgress, resetBookProgress } from "../../api/playback";
import { checkFavorite } from "../../api/collections";
import { formatDuration } from "../../lib/format";
import { playBook } from "../../lib/play";
import { usePlayerStore } from "../../store/playerStore";
import { useMyPermissions } from "../../lib/permissions";
import { canDelete, moveToTrash } from "../../lib/trash";
import { ConfirmTrashDialog } from "../../components/library/ConfirmTrashDialog";
import { SplitView, ColumnHeader } from "../../components/shell/SplitView";
import SectionTheme from "../../components/shell/SectionTheme";
import { MediaCard, MediaGrid, MediaRow } from "../../components/library/MediaCard";
import { AuthorAvatar } from "../../components/library/AuthorAvatar";
import { listAuthors } from "../../api/authors";
import { ViewToggle, SortMenu, FilterField, FilterChips, MoreMenuTrigger } from "../../components/library/BrowseControls";
import { usePersistedState } from "../../lib/persistedState";
import { Button, EmptyState, IconButton, MenuItem, MenuSeparator, Skeleton, toast } from "../../components/ui";
import BookDetail from "./BookDetail";
import UploadBookDialog from "./UploadBookDialog";
import EditBookSheet from "./EditBookSheet";
import IdentifyBookSheet from "./IdentifyBookSheet";
import AddToCollectionDialog from "./AddToCollectionDialog";
import { AuthorDetail, AuthorDialog, CollectionDetail, GroupList, SeriesDetail } from "./Organize";
import { BatchBar } from "../../components/library/BatchBar";
import { useSelection } from "../../lib/useSelection";
import type { AudioBook } from "../../api/types";
import { useT, type PlainKey } from "../../i18n";

type Sort = "title" | "author" | "recent" | "duration";
type Group = "all" | "inProgress" | "finished" | "byAuthor" | "favorites" | "series" | "collections";

const SORTS: { value: Sort; label: PlainKey }[] = [
  { value: "recent", label: "audiobooks.sort.recent" },
  { value: "title", label: "audiobooks.sort.title" },
  { value: "author", label: "audiobooks.sort.author" },
  { value: "duration", label: "audiobooks.sort.length" },
];

export default function AudiobooksPage() {
  const { bookId, groupId } = useParams<{ bookId: string; groupId: string }>();
  const { pathname } = useLocation();
  const uploading = pathname === "/audiobooks/upload";
  // Series, collections and authors open in the detail column like a book does.
  const groupKind = groupId ? (pathname.split("/")[2] as "series" | "collections" | "authors") : null;
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [view, setView] = usePersistedState<"grid" | "list">("own-audio-books-view", "grid");
  const [sort, setSort] = usePersistedState<Sort>("own-audio-books-sort", "recent");
  const [group, setGroup] = usePersistedState<Group>("own-audio-books-group", "all");
  // Orthogonal to Group (progress state): visibility, not listening state. Kept separate rather
  // than a fifth Group segment so "Mine" and "Listening" can be on at the same time.
  const [owner, setOwner] = usePersistedState<"all" | "mine">("own-audio-books-owner", "all");
  const [filter, setFilter] = useState("");
  const [creatingAuthor, setCreatingAuthor] = useState<string | null>(null);
  const [editing, setEditing] = useState<string | null>(null);
  const [identifying, setIdentifying] = useState<AudioBook | null>(null);
  const selection = useSelection();
  const [addingToCollection, setAddingToCollection] = useState(false);
  const { canUpload, isFamilyAdmin } = useMyPermissions();
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const { t } = useT();

  const { data: books = [], isLoading } = useQuery({ queryKey: ["books"], queryFn: listBooks });

  // One request for the whole shelf, not one per book — and the only source of
  // positions reached on another device, since /library and /books carry none.
  const { data: bookProgress = [] } = useQuery({ queryKey: ["book-progress"], queryFn: listBookProgress });
  const favQueries = useQueries({
    queries: books.map((b) => ({ queryKey: ["book-favorite", b.id], queryFn: () => checkFavorite(b.id) })),
  });
  // For the "By author" section headers' photo — book.author is the free-text
  // string a book carries, this is the managed catalogue it's matched against
  // by name. Not every author string has a row here (nobody's created one),
  // which is fine: those headers just render without a photo.
  const { data: authors = [] } = useQuery({ queryKey: ["authors"], queryFn: listAuthors });

  const progressMap = useMemo(
    () => new Map(bookProgress.map((p) => [p.book_id, p])),
    [bookProgress],
  );

  const authorByName = useMemo(
    () => new Map(authors.map((a) => [a.name.toLowerCase(), a])),
    [authors],
  );

  const favSet = useMemo(() => {
    const s = new Set<string>();
    books.forEach((b, i) => { if (favQueries[i]?.data?.is_favorite) s.add(b.id); });
    return s;
  }, [books, favQueries]);

  const [trashingOther, setTrashingOther] = useState<AudioBook | null>(null);
  const trash = (b: AudioBook) =>
    moveToTrash({
      title: b.title,
      remove: (batch) => deleteBook(b.id, batch),
      onChanged: () => qc.invalidateQueries({ queryKey: ["books"] }),
    });

  const visibilityMutation = useMutation({
    mutationFn: ({ id, v }: { id: string; v: "private" | "family" }) => setBookVisibility(id, v),
    onSuccess: (_d, { v }) => {
      qc.invalidateQueries({ queryKey: ["books"] });
      toast.success(v === "family" ? t("audiobooks.visibility.shared") : t("audiobooks.visibility.madePrivate"));
    },
    onError: () => toast.error(t("audiobooks.error.visibility")),
  });

  const restartMutation = useMutation({
    mutationFn: (id: string) => resetBookProgress(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["book-progress"] });
      toast.success(t("audiobooks.toast.restarted"));
    },
    onError: () => toast.error(t("audiobooks.error.restart")),
  });

  const afterBatch = () => qc.invalidateQueries({ queryKey: ["books"] });

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    let list = books.filter((b) => !q || b.title.toLowerCase().includes(q) || (b.author ?? "").toLowerCase().includes(q));
    // A private item is never visible to anyone but its owner (server-side), so "private" here
    // always means "mine" — no separate owner check needed.
    if (owner === "mine") list = list.filter((b) => b.visibility === "private");
    if (group === "inProgress") list = list.filter((b) => { const p = progressMap.get(b.id); return p && p.position_secs > 0 && !p.completed; });
    if (group === "finished") list = list.filter((b) => progressMap.get(b.id)?.completed);
    if (group === "favorites") list = list.filter((b) => favSet.has(b.id));
    const sorted = [...list];
    sorted.sort((a, b) => {
      switch (sort) {
        case "title": return a.title.localeCompare(b.title);
        case "author": return (a.author ?? "").localeCompare(b.author ?? "") || a.title.localeCompare(b.title);
        case "duration": return (b.total_duration_secs ?? 0) - (a.total_duration_secs ?? 0);
        default: return b.created_at.localeCompare(a.created_at);
      }
    });
    return sorted;
  }, [books, filter, group, owner, sort, progressMap, favSet]);

  const byAuthor = useMemo(() => {
    const m = new Map<string, AudioBook[]>();
    for (const b of visible) {
      // "" stands for no author; it is named at render time, in the current language, and sorts last.
      const key = b.author?.trim() ?? "";
      if (!m.has(key)) m.set(key, []);
      m.get(key)!.push(b);
    }
    return [...m.entries()].sort((a, b) => (!a[0] ? 1 : !b[0] ? -1 : a[0].localeCompare(b[0])));
  }, [visible]);

  function statusFor(b: AudioBook) {
    const p = progressMap.get(b.id);
    if (p?.completed) return t("audiobooks.status.finished");
    if (p && b.total_duration_secs) {
      return t("audiobooks.status.progress", {
        progress: p.position_secs / b.total_duration_secs,
        remaining: formatDuration(Math.round(b.total_duration_secs - p.position_secs)),
      });
    }
    return b.total_duration_secs ? formatDuration(b.total_duration_secs) : t("audiobooks.status.new");
  }

  function cardProps(b: AudioBook) {
    const p = progressMap.get(b.id);
    const active = current?.kind === "audiobook" && current.bookId === b.id;
    // Owner, or a family admin when the book is shared — same line BookDetail draws for the
    // same action, so a control never appears here only to 403 on click.
    const canManage = b.is_owner || (isFamilyAdmin && b.visibility === "family");
    return {
      kind: "audiobook" as const,
      title: b.title,
      subtitle: b.author,
      cover: b.cover_url,
      progress: p && b.total_duration_secs ? p.position_secs / b.total_duration_secs : null,
      completed: p?.completed,
      favorite: favSet.has(b.id),
      family: b.visibility === "family",
      active,
      playing: active && playing,
      selected: selection.count > 0 ? selection.has(b.id) : b.id === bookId,
      onClick: () => (selection.count > 0 ? selection.toggle(b.id) : navigate(`/audiobooks/${b.id}`)),
      onPlay: () => void playBook(b),
      menu: (
        <MoreMenuTrigger variant={view === "grid" ? "overlay" : "plain"}>
          <MenuItem icon={<Play />} onSelect={() => void playBook(b)}>{t("common.action.play")}</MenuItem>
          <MenuItem icon={<BookIcon />} onSelect={() => navigate(`/audiobooks/${b.id}`)}>{t("audiobooks.menu.open")}</MenuItem>
          <MenuItem icon={<CheckSquare />} onSelect={() => selection.toggle(b.id)}>{t("audiobooks.menu.select")}</MenuItem>
          {p && (p.position_secs > 0 || p.completed) && (
            <MenuItem icon={<RotateCcw />} onSelect={() => restartMutation.mutate(b.id)}>{t("audiobooks.action.startOver")}</MenuItem>
          )}
          {b.is_owner && <MenuItem icon={<Pencil />} onSelect={() => setEditing(b.id)}>{t("audiobooks.menu.edit")}</MenuItem>}
          {canManage && <MenuItem icon={<ScanSearch />} onSelect={() => setIdentifying(b)}>{t("audiobooks.menu.identify")}</MenuItem>}
          {b.is_owner && (
            <MenuItem
              icon={b.visibility === "family" ? <UserX /> : <Users />}
              onSelect={() => visibilityMutation.mutate({ id: b.id, v: b.visibility === "family" ? "private" : "family" })}
            >
              {b.visibility === "family" ? t("audiobooks.action.makePrivate") : t("audiobooks.action.shareWithFamily")}
            </MenuItem>
          )}
          <MenuSeparator />
          <MenuItem
            destructive
            disabled={!canDelete(b, isFamilyAdmin)}
            onSelect={() =>
              b.is_owner
                ? void trash(b).catch(() => toast.error(t("audiobooks.error.trash")))
                : setTrashingOther(b)
            }
          >
            {b.is_owner || !canDelete(b, isFamilyAdmin) ? t("audiobooks.menu.moveToTrash") : t("audiobooks.menu.moveToTrashConfirm")}
          </MenuItem>
        </MoreMenuTrigger>
      ),
    };
  }

  // Series and Collections list groups, not books.
  const isGroupList = group === "series" || group === "collections";

  const content = (
    <>
      <ColumnHeader
        title={t("common.kind.audiobooks")}
        actions={
          <>
            <IconButton
              size="sm"
              label={selection.count > 0 ? t("audiobooks.selection.clear") : t("audiobooks.selection.selectAll")}
              active={selection.count > 0}
              onClick={() => (selection.count > 0 ? selection.clear() : selection.selectAll(visible.map((b) => b.id)))}
            >
              <CheckSquare className="h-4 w-4" />
            </IconButton>
            {canUpload && <Button size="sm" icon={<Plus className="h-4 w-4" />} onClick={() => navigate("/audiobooks/upload")}>{t("common.action.add")}</Button>}
          </>
        }
      >
        <div className="flex items-center gap-1.5">
          <FilterField value={filter} onChange={setFilter} placeholder={t("audiobooks.list.filter")} className="min-w-0 flex-1" />
          {!isGroupList && <SortMenu value={sort} onChange={setSort} options={SORTS.map((s) => ({ value: s.value, label: t(s.label) }))} />}
          {!isGroupList && <ViewToggle value={view} onChange={setView} />}
        </div>
        <FilterChips<Group>
          value={group}
          onChange={setGroup}
          toggle={isGroupList ? undefined : { label: t("audiobooks.owner.mine"), on: owner === "mine", onChange: (on) => setOwner(on ? "mine" : "all") }}
          chips={[
            { value: "all", label: t("audiobooks.group.all") },
            { value: "inProgress", label: t("audiobooks.group.inProgress") },
            { value: "finished", label: t("audiobooks.group.finished") },
            { value: "byAuthor", label: t("audiobooks.group.byAuthor") },
            { value: "favorites", label: t("audiobooks.organize.mode.favorites") },
            { value: "series", label: t("audiobooks.organize.mode.series") },
            { value: "collections", label: t("audiobooks.organize.mode.collections") },
          ]}
        />
      </ColumnHeader>

      {isGroupList && <GroupList mode={group} filter={filter} selectedId={groupId} />}

      {!isGroupList && isLoading && <MediaGrid>{[...Array(8)].map((_, i) => <Skeleton key={i} className="aspect-[2/3]" />)}</MediaGrid>}

      {!isGroupList && !isLoading && visible.length === 0 && (
        <EmptyState
          icon={<BookIcon />}
          title={filter ? t("audiobooks.empty.noMatch.title") : group === "favorites" ? t("audiobooks.organize.emptyFavorites") : owner === "mine" ? t("audiobooks.empty.noPrivate.title") : t("audiobooks.empty.none.title")}
          description={
            filter
              ? t("audiobooks.empty.noMatch.description")
              : group === "favorites"
                ? t("audiobooks.organize.emptyFavoritesHint")
                : owner === "mine"
                ? t("audiobooks.empty.noPrivate.description")
                : t("audiobooks.empty.none.description")
          }
          action={!filter && owner === "all" && canUpload && <Button onClick={() => navigate("/audiobooks/upload")}>{t("audiobooks.upload.title")}</Button>}
        />
      )}

      {!isLoading && visible.length > 0 && group === "byAuthor" && (
        <div className="pb-6">
          {byAuthor.map(([author, list]) => (
            <section key={author}>
              <h2 className="sticky top-0 z-[5] flex items-center gap-2 bg-bg/85 px-5 py-2 sm:top-[var(--column-header-h,3.5rem)] text-[13px] font-semibold text-muted backdrop-blur">
                <AuthorAvatar src={authorByName.get(author.toLowerCase())?.image_url} alt={author || t("audiobooks.unknownAuthor")} className="h-5 w-5" />
                <span className="min-w-0 flex-1 truncate">{author || t("audiobooks.unknownAuthor")}</span>
                {/* The author's page — photo, name, books — or, for a name no catalogue entry has yet, a way to make one. */}
                {author && (
                  <IconButton
                    size="sm"
                    label={t("audiobooks.organize.editNamed", { name: author })}
                    onClick={() => {
                      const known = authorByName.get(author.toLowerCase());
                      if (known) navigate(`/audiobooks/authors/${known.id}`);
                      else setCreatingAuthor(author);
                    }}
                  >
                    <UserPen className="h-4 w-4" />
                  </IconButton>
                )}
              </h2>
              {view === "grid" ? (
                <MediaGrid dense>{list.map((b) => <MediaCard key={b.id} {...cardProps(b)} meta={statusFor(b)} />)}</MediaGrid>
              ) : (
                <div className="px-3 pb-2">{list.map((b) => <MediaRow key={b.id} {...cardProps(b)} trailing={statusFor(b)} />)}</div>
              )}
            </section>
          ))}
        </div>
      )}

      {!isGroupList && !isLoading && visible.length > 0 && group !== "byAuthor" && (
        view === "grid" ? (
          <MediaGrid>{visible.map((b) => <MediaCard key={b.id} {...cardProps(b)} meta={statusFor(b)} />)}</MediaGrid>
        ) : (
          <div className="p-3">{visible.map((b) => <MediaRow key={b.id} {...cardProps(b)} trailing={statusFor(b)} />)}</div>
        )
      )}

      {addingToCollection && (
        <AddToCollectionDialog
          bookIds={visible.filter((b) => selection.has(b.id)).map((b) => b.id)}
          onClose={() => setAddingToCollection(false)}
          onDone={selection.clear}
        />
      )}

      <BatchBar
        selected={visible
          .filter((b) => selection.has(b.id))
          .map((b) => ({ id: b.id, label: b.title, isOwner: b.is_owner, canDelete: canDelete(b, isFamilyAdmin) }))}
        onClear={selection.clear}
        extraActions={
          /* Not restricted to books you own: a collection is your own grouping,
             and putting someone else's shared book in it changes nothing for them. */
          <Button
            size="sm"
            variant="secondary"
            icon={<Library className="h-4 w-4" />}
            onClick={() => setAddingToCollection(true)}
          >
            {t("audiobooks.list.addToCollection")}
          </Button>
        }
        onSetVisibility={async (id, v) => {
          await setBookVisibility(id, v);
          afterBatch();
        }}
        onTrash={(id, batch) => deleteBook(id, batch)}
        afterTrash={afterBatch}
      />
    </>
  );

  return (
    <SectionTheme cloud="book">
      {uploading && <UploadBookDialog />}
      {editing && <EditBookSheet bookId={editing} onClose={() => setEditing(null)} />}
      {creatingAuthor && <AuthorDialog initialName={creatingAuthor} onClose={() => setCreatingAuthor(null)} />}
      {identifying && <IdentifyBookSheet book={identifying} onClose={() => setIdentifying(null)} />}
      {trashingOther && (
        <ConfirmTrashDialog title={trashingOther.title} onConfirm={() => trash(trashingOther)} onClose={() => setTrashingOther(null)} />
      )}
    <SplitView
      contentWidth="wide"
      widthKey="audiobooks"
      hasDetail={!!bookId || !!groupId}
      onBack={() => navigate("/audiobooks")}
      content={content}
      detail={
        groupKind === "series" && groupId ? (
          <SeriesDetail seriesId={groupId} onDeleted={() => navigate("/audiobooks")} />
        ) : groupKind === "collections" && groupId ? (
          <CollectionDetail collectionId={groupId} onDeleted={() => navigate("/audiobooks")} />
        ) : groupKind === "authors" && groupId ? (
          <AuthorDetail authorId={groupId} onDeleted={() => navigate("/audiobooks")} />
        ) : bookId ? (
          <BookDetail bookId={bookId} />
        ) : (
          <EmptyState icon={<BookIcon />} title={t("audiobooks.empty.pickBook.title")} description={t("audiobooks.empty.pickBook.description")} />
        )
      }
    />
    </SectionTheme>
  );
}
