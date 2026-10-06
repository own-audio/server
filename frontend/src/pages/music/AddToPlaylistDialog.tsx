// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, ListMusic, Plus } from "lucide-react";
import { addTrackToPlaylist, createPlaylist, listPlaylistTracks, listPlaylists } from "../../api/music";
import { runWithLimit } from "../../lib/uploadQueue";
import { Button, Cover, Dialog, DialogContent, EmptyState, Input, SearchField, Skeleton, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/* Adding N songs is N requests — there is no bulk add.
 *
 * Unlike every other batch in this app these must run **one at a time**. The
 * server assigns a position with `COALESCE((SELECT MAX(position) + 1 …))` under
 * a `UNIQUE (playlist_id, position)` constraint, so parallel adds all read the
 * same maximum and all but one are rejected — adding an 8-track album four at a
 * time landed 3 of them. Sequential also preserves album order. */
const CONCURRENCY = 1;

/**
 * Put one song, or a whole album, into a playlist.
 *
 * Creating a playlist lives in this dialog rather than behind a separate trip
 * to the Playlists tab: "add these to a new playlist" is the common case the
 * first time anyone organises anything, and sending them away to make an empty
 * playlist first loses the songs they had in hand.
 */
export default function AddToPlaylistDialog({
  trackIds,
  label,
  onClose,
}: {
  trackIds: string[];
  /** What is being added, for the confirmation line. */
  label: string;
  onClose: () => void;
}) {
  const qc = useQueryClient();
  const { t } = useT();
  const [filter, setFilter] = useState("");
  const [newName, setNewName] = useState("");
  const [busy, setBusy] = useState<string | null>(null);

  const { data: playlists = [], isLoading } = useQuery({ queryKey: ["playlists"], queryFn: listPlaylists });

  const mine = useMemo(() => {
    const q = filter.trim().toLowerCase();
    // Only your own: the API refuses a write to someone else's playlist, so
    // offering it would only produce a failure.
    return playlists.filter((p) => p.is_owner && (!q || p.name.toLowerCase().includes(q)));
  }, [playlists, filter]);

  async function addTo(playlistId: string, playlistName: string) {
    setBusy(playlistId);
    const { failed } = await runWithLimit(trackIds, CONCURRENCY, (id) => addTrackToPlaylist(playlistId, id));
    qc.invalidateQueries({ queryKey: ["playlists"] });
    qc.invalidateQueries({ queryKey: ["playlist-tracks", playlistId] });
    setBusy(null);

    const added = trackIds.length - failed.length;
    if (failed.length === 0) {
      toast.success(
        trackIds.length === 1
          ? t("music.addToPlaylist.addedOne", { playlist: playlistName })
          : t("music.addToPlaylist.addedMany", { count: added, playlist: playlistName })
      );
      onClose();
    } else {
      toast.error(t("music.addToPlaylist.partial", { done: added, total: trackIds.length }), t("music.addToPlaylist.partialBody"));
    }
  }

  const create = useMutation({
    mutationFn: () => createPlaylist({ name: newName.trim() }),
    onSuccess: async (p) => {
      qc.invalidateQueries({ queryKey: ["playlists"] });
      await addTo(p.id, p.name);
    },
    onError: () => toast.error(t("music.playlist.createFailed")),
  });

  function submitNew(e: FormEvent) {
    e.preventDefault();
    if (newName.trim()) create.mutate();
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.addToPlaylist.title")}
        description={trackIds.length === 1 ? label : t("music.addToPlaylist.description", { count: trackIds.length, label })}
        footer={
          <Button variant="ghost" onClick={onClose}>
            {t("common.action.close")}
          </Button>
        }
      >
        <form onSubmit={submitNew} className="flex items-end gap-2">
          <Input
            label={t("music.action.newPlaylist")}
            value={newName}
            onChange={(e) => setNewName(e.target.value)}
            placeholder={t("music.field.playlistPlaceholder")}
            className="flex-1"
          />
          <Button type="submit" icon={<Plus className="h-4 w-4" />} loading={create.isPending} disabled={!newName.trim()}>
            {t("music.addToPlaylist.createAndAdd")}
          </Button>
        </form>

        {playlists.filter((p) => p.is_owner).length > 0 && (
          <>
            <div className="my-4 flex items-center gap-3">
              <span className="h-px flex-1 bg-border" />
              <span className="text-xs text-muted">{t("music.addToPlaylist.orPick")}</span>
              <span className="h-px flex-1 bg-border" />
            </div>

            {playlists.filter((p) => p.is_owner).length > 6 && (
              <SearchField
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder={t("music.addToPlaylist.find")}
                className="mb-2 w-full"
              />
            )}
          </>
        )}

        {isLoading ? (
          <Skeleton className="h-32" />
        ) : mine.length === 0 ? (
          playlists.filter((p) => p.is_owner).length === 0 ? null : (
            <p className="py-4 text-center text-sm text-muted">{t("music.browse.noMatch")}</p>
          )
        ) : (
          <ul className="max-h-72 space-y-0.5 overflow-y-auto">
            {mine.map((p) => (
              <li key={p.id}>
                <PlaylistRow
                  id={p.id}
                  name={p.name}
                  cover={p.cover_url}
                  trackCount={p.track_count}
                  trackIds={trackIds}
                  busy={busy === p.id}
                  disabled={busy !== null}
                  onAdd={() => addTo(p.id, p.name)}
                />
              </li>
            ))}
          </ul>
        )}

        {!isLoading && playlists.filter((p) => p.is_owner).length === 0 && (
          <EmptyState
            icon={<ListMusic />}
            title={t("music.browse.noPlaylists")}
            description={t("music.addToPlaylist.emptyHint")}
            className="py-8"
          />
        )}
      </DialogContent>
    </Dialog>
  );
}

/** Says up front when everything being added is already in there, so a second
 *  click isn't needed to find out nothing changed. */
function PlaylistRow({
  id,
  name,
  cover,
  trackCount,
  trackIds,
  busy,
  disabled,
  onAdd,
}: {
  id: string;
  name: string;
  cover: string | null;
  trackCount: number;
  trackIds: string[];
  busy: boolean;
  disabled: boolean;
  onAdd: () => void;
}) {
  const { t } = useT();
  const { data: existing } = useQuery({
    queryKey: ["playlist-tracks", id],
    queryFn: () => listPlaylistTracks(id),
    staleTime: 30_000,
  });

  const already = existing ? trackIds.filter((id) => existing.some((e) => e.track.id === id)).length : 0;
  const allPresent = already > 0 && already === trackIds.length;

  return (
    <button
      onClick={onAdd}
      disabled={disabled}
      className={cn(
        "flex w-full items-center gap-3 rounded-lg px-2 py-1.5 text-left transition-colors",
        "hover:bg-bg-alt disabled:opacity-50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      )}
    >
      <Cover kind="music" src={cover} alt="" aspect="square" className="h-9 w-9 shrink-0 rounded-md" />
      <span className="min-w-0 flex-1">
        <span className="block truncate text-sm font-medium">{name}</span>
        <span className="block truncate text-xs text-muted">
          {allPresent
            ? t("music.addToPlaylist.allPresent", { count: trackCount })
            : already > 0
              ? t("music.addToPlaylist.somePresent", { count: trackCount, already })
              : t("music.count.songs", { count: trackCount })}
        </span>
      </span>
      {busy ? (
        <span className="text-xs text-muted">{t("music.addToPlaylist.adding")}</span>
      ) : allPresent ? (
        <Check className="h-4 w-4 shrink-0 text-success" />
      ) : (
        <Plus className="h-4 w-4 shrink-0 text-muted" />
      )}
    </button>
  );
}
