// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { moveToTrash } from "../../lib/trash";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { closestCenter, DndContext, KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent } from "@dnd-kit/core";
import { arrayMove, SortableContext, sortableKeyboardCoordinates, useSortable, verticalListSortingStrategy } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";
import { GripVertical, ImageUp, Trash2, X } from "lucide-react";
import {
  createPlaylist,
  deletePlaylist,
  listPlaylistTracks,
  playlistAudience,
  sharePlaylist,
  removeTrackFromPlaylist,
  reorderPlaylistTracks,
  setPlaylistVisibility,
  updatePlaylist,
  uploadPlaylistCover,
} from "../../api/music";
import { formatDuration } from "../../lib/format";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, IconButton, Input, SegmentedControl, Skeleton, Textarea, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { VisibilityField } from "../../components/library/VisibilityField";
import type { MusicPlaylist, MusicPlaylistTrack, Visibility } from "../../api/types";
import { useT } from "../../i18n";

// ── Create ────────────────────────────────────────────────────────────────

export function CreatePlaylistDialog({ onClose, onCreated }: { onClose: () => void; onCreated?: (p: MusicPlaylist) => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);

  const create = useMutation({
    mutationFn: () => createPlaylist({ name: name.trim(), description: description.trim() || undefined }),
    onSuccess: (p) => {
      qc.invalidateQueries({ queryKey: ["playlists"] });
      toast.success(t("music.playlist.created"), p.name);
      onCreated?.(p);
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.playlist.createFailed"))),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.action.newPlaylist")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button onClick={() => create.mutate()} loading={create.isPending} disabled={!name.trim()}>
              {t("music.action.create")}
            </Button>
          </>
        }
      >
        <Input label={t("music.field.name")} autoFocus value={name} onChange={(e) => setName(e.target.value)} placeholder={t("music.field.playlistPlaceholder")} />
        <div className="mt-3">
          <Textarea label={t("music.field.description")} value={description} onChange={(e) => setDescription(e.target.value)} placeholder={t("music.field.optional")} />
        </div>
        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}

// ── Edit, reorder, remove ─────────────────────────────────────────────────

function SortableEntry({ entry, onRemove }: { entry: MusicPlaylistTrack; onRemove: () => void }) {
  const { t } = useT();
  const { attributes, listeners, setNodeRef, transform, transition, isDragging } = useSortable({ id: entry.entry_id });
  return (
    <li
      ref={setNodeRef}
      style={{ transform: CSS.Transform.toString(transform), transition }}
      className={`group flex items-center gap-2 border-b border-border bg-card px-2 py-2 last:border-b-0 ${isDragging ? "opacity-60" : ""}`}
    >
      <button {...attributes} {...listeners} aria-label={t("music.playlist.reorderItem", { title: entry.track.title })} className="cursor-grab text-muted hover:text-fg active:cursor-grabbing">
        <GripVertical className="h-4 w-4" />
      </button>
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm">{entry.track.title}</span>
        {entry.track.artist && <span className="block truncate text-xs text-muted">{entry.track.artist}</span>}
      </span>
      {entry.track.duration_secs != null && (
        <span className="shrink-0 text-xs tabular-nums text-muted">{formatDuration(entry.track.duration_secs)}</span>
      )}
      <IconButton size="sm" label={t("music.playlist.removeItem", { title: entry.track.title })} onClick={onRemove}>
        <X className="h-3.5 w-3.5" />
      </IconButton>
    </li>
  );
}

export function EditPlaylistSheet({ playlist, onClose }: { playlist: MusicPlaylist; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const { data: serverEntries = [] } = useQuery({
    queryKey: ["playlist-tracks", playlist.id],
    queryFn: () => listPlaylistTracks(playlist.id),
  });

  const [name, setName] = useState(playlist.name);
  const [description, setDescription] = useState(playlist.description ?? "");
  const [cover, setCover] = useState<File | null>(null);
  const [order, setOrder] = useState<MusicPlaylistTrack[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const entries = order ?? serverEntries;
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 4 } }), useSensor(KeyboardSensor, { coordinateGetter: sortableKeyboardCoordinates }));

  function onDragEnd(e: DragEndEvent) {
    const { active, over } = e;
    if (!over || active.id === over.id) return;
    const from = entries.findIndex((x) => x.entry_id === active.id);
    const to = entries.findIndex((x) => x.entry_id === over.id);
    setOrder(arrayMove(entries, from, to));
  }

  const invalidate = () => {
    qc.invalidateQueries({ queryKey: ["playlists"] });
    qc.invalidateQueries({ queryKey: ["playlist-tracks", playlist.id] });
  };

  const save = useMutation({
    mutationFn: async () => {
      await updatePlaylist(playlist.id, { name: name.trim(), description: description.trim() || undefined });
      if (cover) await uploadPlaylistCover(playlist.id, cover);
      // The server takes the whole entry order, not a moved-item delta.
      if (order) await reorderPlaylistTracks(playlist.id, order.map((e) => e.entry_id));
    },
    onSuccess: () => {
      invalidate();
      toast.success(t("music.playlist.updated"));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.playlist.saveFailed"))),
  });

  const removeEntry = useMutation({
    mutationFn: (entryId: string) => removeTrackFromPlaylist(playlist.id, entryId),
    onSuccess: invalidate,
    onError: () => toast.error(t("music.playlist.removeSongFailed")),
  });

  const changeVisibility = useMutation({
    mutationFn: (v: Visibility) => setPlaylistVisibility(playlist.id, v),
    onSuccess: (_d, v) => {
      invalidate();
      toast.success(t(v === "family" ? "music.toast.sharedWithFamily" : "music.toast.madePrivate"));
    },
  });

  const [trashing, setTrashing] = useState(false);
  async function trash() {
    setTrashing(true);
    try {
      await moveToTrash({ title: playlist.name, remove: (batch) => deletePlaylist(playlist.id, batch), onChanged: invalidate });
      onClose();
    } catch {
      setError(t("music.playlist.trashFailed"));
      setTrashing(false);
    }
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.playlist.editTitle")}
        className="sm:max-w-2xl"
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
        <div className="grid gap-3 sm:grid-cols-2">
          <Input label={t("music.field.name")} value={name} onChange={(e) => setName(e.target.value)} />
          <label className="block">
            <span className="mb-1.5 block text-[13px] font-medium text-fg">{t("music.field.cover")}</span>
            <label className="flex h-10 cursor-pointer items-center gap-2 rounded-[10px] border border-border bg-card px-3 text-sm text-muted hover:text-fg">
              <ImageUp className="h-4 w-4" />
              <span className="truncate">{cover ? cover.name : t("music.field.chooseImage")}</span>
              <input type="file" accept="image/*" hidden onChange={(e) => setCover(e.target.files?.[0] ?? null)} />
            </label>
          </label>
        </div>

        <div className="mt-3">
          <Textarea label={t("music.field.description")} value={description} onChange={(e) => setDescription(e.target.value)} />
        </div>

        <div className="mt-4">
          <VisibilityField value={playlist.visibility} disabled={!playlist.is_owner} onChange={(v) => changeVisibility.mutate(v)} />
        </div>

        {entries.length > 0 && (
          <div className="mt-5">
            <p className="mb-1.5 text-[13px] font-medium">{t("music.mode.songs")}</p>
            <p className="mb-2 text-xs text-muted">{t("music.playlist.dragHint")}</p>
            <ul className="max-h-72 overflow-y-auto rounded-card border border-border">
              <DndContext sensors={sensors} collisionDetection={closestCenter} onDragEnd={onDragEnd}>
                <SortableContext items={entries.map((e) => e.entry_id)} strategy={verticalListSortingStrategy}>
                  {entries.map((e) => (
                    <SortableEntry
                      key={e.entry_id}
                      entry={e}
                      onRemove={() => {
                        setOrder(entries.filter((x) => x.entry_id !== e.entry_id));
                        removeEntry.mutate(e.entry_id);
                      }}
                    />
                  ))}
                </SortableContext>
              </DndContext>
            </ul>
          </div>
        )}

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}

        {playlist.is_owner && (
          <div className="mt-6 border-t border-border pt-4">
            <Button size="sm" variant="ghost" icon={<Trash2 className="h-4 w-4" />} className="text-error" loading={trashing} onClick={() => void trash()}>
              {t("music.action.moveToTrash")}
            </Button>
            <p className="mt-1 text-xs text-muted">{t("music.playlist.songsStay")}</p>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ── Share ─────────────────────────────────────────────────────────────────

/**
 * Share a playlist with chosen family members — live (they play yours and see each change)
 * or as a copy each of them owns. The server first shares your private songs in it with
 * them, since a song they cannot play would just be a gap.
 */
export function SharePlaylistDialog({
  playlist,
  privateSongs,
  onClose,
}: {
  playlist: MusicPlaylist;
  /** How many songs in it only you can play now. */
  privateSongs: number;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { t } = useT();
  const [mode, setMode] = useState<"live" | "copy">("live");
  const [picked, setPicked] = useState<Set<string> | null>(null);
  const { data: members = [], isLoading } = useQuery({
    queryKey: ["playlist-audience", playlist.id],
    queryFn: () => playlistAudience(playlist.id),
  });
  // Live starts from who can hear it now; a copy starts from nobody.
  const liveNow = new Set(members.filter((m) => m.can_listen && !m.locked).map((m) => m.user_id));
  const chosen = picked ?? (mode === "live" ? liveNow : new Set<string>());
  const isShared = playlist.visibility === "family";

  const share = useMutation({
    mutationFn: (ids: string[]) => sharePlaylist(playlist.id, mode, ids),
    onSuccess: (r, ids) => {
      qc.invalidateQueries({ queryKey: ["playlists"] });
      qc.invalidateQueries({ queryKey: ["playlist"] });
      qc.invalidateQueries({ queryKey: ["playlist-audience", playlist.id] });
      if (r.shared_tracks > 0) qc.invalidateQueries({ queryKey: ["music-tracks"] });
      if (mode === "copy") toast.success(t("music.share.doneCopy", { count: r.copies }), playlist.name);
      else toast.success(ids.length > 0 ? t("music.share.doneLive") : t("music.share.stopped"), playlist.name);
      onClose();
    },
    onError: (err) => toast.error(t("music.share.failed"), apiErrorMessage(err, t("music.toast.tryAgain"))),
  });

  const toggle = (id: string) => {
    const next = new Set(chosen);
    if (next.has(id)) next.delete(id);
    else next.add(id);
    setPicked(next);
  };

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.share.title", { name: playlist.name })}
        variant="sheet"
        footer={
          <>
            {mode === "live" && isShared && (
              <Button variant="ghost" className="mr-auto" disabled={share.isPending} onClick={() => share.mutate([])}>
                {t("music.share.stop")}
              </Button>
            )}
            <Button variant="ghost" onClick={onClose}>{t("common.action.cancel")}</Button>
            <Button loading={share.isPending} disabled={chosen.size === 0} onClick={() => share.mutate([...chosen])}>
              {t(mode === "live" ? "music.share.submitLive" : "music.share.submitCopy")}
            </Button>
          </>
        }
      >
        <SegmentedControl<"live" | "copy">
          value={mode}
          onChange={(m) => { setMode(m); setPicked(null); }}
          segments={[
            { value: "live", label: t("music.share.mode.live") },
            { value: "copy", label: t("music.share.mode.copy") },
          ]}
        />
        <p className="mt-2 text-sm text-muted">{t(mode === "live" ? "music.share.mode.liveHint" : "music.share.mode.copyHint")}</p>

        <p className="mt-5 text-[13px] font-medium">{t("music.share.who")}</p>
        {isLoading ? (
          <Skeleton className="mt-2 h-24" />
        ) : members.length === 0 ? (
          <p className="mt-2 text-sm text-muted">{t("music.share.nobody")}</p>
        ) : (
          <ul className="mt-2 divide-y divide-border rounded-card border border-border">
            {members.map((m) => {
              // An admin sees whatever the family shares; in live mode there is nothing to choose.
              const always = mode === "live" && m.locked;
              return (
                <li key={m.user_id}>
                  <label className={cn("flex items-center gap-3 px-3 py-2.5 text-sm pointer-coarse:py-3.5", always ? "opacity-70" : "cursor-pointer")}>
                    <input
                      type="checkbox"
                      className="h-4 w-4 accent-[var(--accent)]"
                      checked={always || chosen.has(m.user_id)}
                      disabled={always}
                      onChange={() => toggle(m.user_id)}
                    />
                    <span className="min-w-0 flex-1 truncate">{m.display_label || m.display_name}</span>
                    {always && <span className="shrink-0 text-xs text-muted">{t("music.share.adminAlways")}</span>}
                  </label>
                </li>
              );
            })}
          </ul>
        )}

        {privateSongs > 0 && chosen.size > 0 && (
          <p className="mt-3 text-xs text-muted">{t("music.share.privateSongs", { count: privateSongs })}</p>
        )}
      </DialogContent>
    </Dialog>
  );
}
