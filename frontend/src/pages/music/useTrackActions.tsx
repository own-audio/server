// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { Ban, ListMusic, ListPlus, MessageSquareQuote, Pencil, Play, ScanSearch, SlidersHorizontal, ThumbsDown, Trash2 } from "lucide-react";
import { deleteTrack, setTrackFeedback, type FeedbackKind } from "../../api/music";
import { queueTrack } from "../../lib/play";
import { MenuItem, toast } from "../../components/ui";
import { ConfirmTrashDialog } from "../../components/library/ConfirmTrashDialog";
import { canDelete, moveToTrash } from "../../lib/trash";
import { useMyPermissions } from "../../lib/permissions";
import { removeDownload } from "../../lib/offline/downloads";
import EditTrackSheet from "./EditTrackSheet";
import RefineIdentityDialog from "./RefineIdentityDialog";
import AddToPlaylistDialog from "./AddToPlaylistDialog";
import { IdentifyTrackSheet, LyricsSheet } from "./MusicTools";
import { MUSIC_KEYS } from "./musicKeys";
import { DownloadMenuItem } from "./Downloads";
import type { MusicTrack } from "../../api/types";
import { useT } from "../../i18n";

/**
 * The per-track verbs and the dialogs behind them, in one place.
 *
 * Both the browse column and the detail column need the same menu, and every
 * entry drags a dialog and its state along with it — six of them. Kept here so
 * the two columns share one copy rather than drifting: the browse column's
 * song rows had no menu at all and a click that navigated to the page they
 * were already on, which is how filtering could find a track you then could
 * not do anything with.
 *
 * Page-specific entries (selecting for a batch, changing visibility) stay with
 * the page that owns that state; this covers only what is true of a track
 * anywhere it appears.
 */
export function useTrackActions() {
  const qc = useQueryClient();
  const { t } = useT();
  const { isFamilyAdmin } = useMyPermissions();

  /* Two meanings, deliberately not one control. A dislike fades over months —
     a track that did not suit the moment is not one you never want to hear
     again — and that is exactly what the second entry is for.

     NEITHER REMOVES ANYTHING. The track stays in the family library, stays
     visible here, and is untouched for every other member; only this user's
     automatic selection skips it. It is a preference, not a permission, so it
     gets no lock icon and it is not a way to delete something. */
  const feedback = useMutation({
    mutationFn: ({ id, kind }: { id: string; kind: FeedbackKind }) => setTrackFeedback(id, kind),
    onSuccess: (_d, { kind }) => {
      qc.invalidateQueries({ queryKey: ["track-feedback"] });
      toast.show(
        t(kind === "banned" ? "music.toast.bannedTitle" : "music.toast.dislikedTitle"),
        t(kind === "banned" ? "music.toast.bannedBody" : "music.toast.dislikedBody")
      );
    },
    onError: () => toast.error(t("music.toast.feedbackFailed"), t("music.toast.tryAgain")),
  });
  const [editing, setEditing] = useState<MusicTrack | null>(null);
  const [identifying, setIdentifying] = useState<MusicTrack | null>(null);
  const [refining, setRefining] = useState<MusicTrack | null>(null);
  const [lyricsFor, setLyricsFor] = useState<MusicTrack | null>(null);
  const [deleting, setDeleting] = useState<MusicTrack | null>(null);
  const [addingToPlaylist, setAddingToPlaylist] = useState<{ ids: string[]; label: string } | null>(null);

  const invalidate = () => MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));

  /** Into the trash with Undo; the copy saved in this browser goes too, so a
   *  deleted song cannot keep playing offline here. */
  const trash = (track: MusicTrack) =>
    moveToTrash({
      title: track.title,
      remove: async (batch) => {
        await deleteTrack(track.id, batch);
        await removeDownload(track.id).catch(() => {});
      },
      onChanged: invalidate,
    });

  /** The menu entries for one track. `onPlay` differs per column — the browse
   *  column plays its filtered list, the detail column plays its own. */
  const items = (track: MusicTrack, onPlay: () => void) => (
    <>
      <MenuItem icon={<Play />} onSelect={onPlay}>
        {t("common.action.play")}
      </MenuItem>
      <MenuItem
        icon={<ListPlus />}
        onSelect={() => {
          void queueTrack(track);
          toast.show(t("music.toast.addedToQueue"), track.title);
        }}
      >
        {t("music.action.addToQueue")}
      </MenuItem>
      <MenuItem icon={<ListMusic />} onSelect={() => setAddingToPlaylist({ ids: [track.id], label: track.title })}>
        {t("music.action.addToPlaylistMenu")}
      </MenuItem>
      <DownloadMenuItem track={track} />
      {track.is_owner && (
        <MenuItem icon={<Pencil />} onSelect={() => setEditing(track)}>
          {t("music.action.editMenu")}
        </MenuItem>
      )}
      {track.is_owner && (
        <MenuItem icon={<ScanSearch />} onSelect={() => setIdentifying(track)}>
          {t("music.action.identifyMenu")}
        </MenuItem>
      )}
      {track.is_owner && (
        <MenuItem icon={<SlidersHorizontal />} onSelect={() => setRefining(track)}>
          {t("music.action.fixIdentityMenu")}
        </MenuItem>
      )}
      <MenuItem icon={<MessageSquareQuote />} onSelect={() => setLyricsFor(track)}>
        {t("music.action.lyricsMenu")}
      </MenuItem>
      <MenuItem
        icon={<ThumbsDown />}
        onSelect={() => feedback.mutate({ id: track.id, kind: "dislike" })}
      >
        {t("music.action.notFeelingIt")}
      </MenuItem>
      <MenuItem icon={<Ban />} onSelect={() => feedback.mutate({ id: track.id, kind: "banned" })}>
        {t("music.action.neverPlay")}
      </MenuItem>
      {canDelete(track, isFamilyAdmin) && (
        <MenuItem
          icon={<Trash2 />}
          destructive
          onSelect={() =>
            track.is_owner
              ? void trash(track).catch(() => toast.error(t("music.error.trash")))
              : setDeleting(track)
          }
        >
          {t(track.is_owner ? "music.action.moveToTrash" : "music.action.moveToTrashConfirm")}
        </MenuItem>
      )}
    </>
  );

  const dialogs = (
    <>
      {deleting && (
        <ConfirmTrashDialog title={deleting.title} onConfirm={() => trash(deleting)} onClose={() => setDeleting(null)} />
      )}
      {refining && <RefineIdentityDialog track={refining} onClose={() => setRefining(null)} />}
      {editing && <EditTrackSheet track={editing} onClose={() => setEditing(null)} />}
      {identifying && <IdentifyTrackSheet track={identifying} onClose={() => setIdentifying(null)} />}
      {lyricsFor && <LyricsSheet track={lyricsFor} onClose={() => setLyricsFor(null)} />}
      {addingToPlaylist && addingToPlaylist.ids.length > 0 && (
        <AddToPlaylistDialog
          trackIds={addingToPlaylist.ids}
          label={addingToPlaylist.label}
          onClose={() => setAddingToPlaylist(null)}
        />
      )}
    </>
  );

  return { items, dialogs, setAddingToPlaylist };
}
