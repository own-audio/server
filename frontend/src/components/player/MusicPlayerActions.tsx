// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Heart, ListPlus, MessageSquareQuote, ThumbsDown } from "lucide-react";
import { listStarred, setTrackFeedback, starTrack, unstarTrack } from "../../api/music";
import { IconButton } from "../ui/Button";
import { toast } from "../../lib/toast";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";
import { LyricsSheet } from "../../pages/music/MusicTools";
import AddToPlaylistDialog from "../../pages/music/AddToPlaylistDialog";
import type { MusicPlayerTrack } from "../../store/playerStore";

/**
 * What a song needs on the Now Playing screen beyond transport: the heart (the
 * stars "Forgotten favourites" reads), its lyrics, adding it to a playlist, and
 * "not feeling it", which also moves on to the next song.
 */
export default function MusicPlayerActions({ track, onSkip }: { track: MusicPlayerTrack; onSkip: () => void }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [lyricsOpen, setLyricsOpen] = useState(false);
  const [addingToPlaylist, setAddingToPlaylist] = useState(false);
  const { data: starred = [] } = useQuery({ queryKey: ["music-starred"], queryFn: listStarred });
  const isStarred = starred.includes(track.trackId);

  const star = useMutation({
    mutationFn: (on: boolean) => (on ? starTrack(track.trackId) : unstarTrack(track.trackId)),
    // The heart answers at once; a failure puts it back.
    onMutate: (on) => {
      const before = qc.getQueryData<string[]>(["music-starred"]) ?? [];
      qc.setQueryData(["music-starred"], on ? [...before, track.trackId] : before.filter((id) => id !== track.trackId));
      return { before };
    },
    onError: (_e, _on, ctx) => {
      if (ctx) qc.setQueryData(["music-starred"], ctx.before);
      toast.error(t("common.error.generic"));
    },
  });

  const dislike = useMutation({
    mutationFn: () => setTrackFeedback(track.trackId, "dislike"),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["track-feedback"] });
      toast.show(t("music.toast.dislikedTitle"), t("music.toast.dislikedBody"));
      onSkip();
    },
    onError: () => toast.error(t("music.toast.feedbackFailed"), t("music.toast.tryAgain")),
  });

  return (
    <>
      <IconButton
        label={isStarred ? t("player.favorite.remove") : t("player.favorite.add")}
        active={isStarred}
        onClick={() => star.mutate(!isStarred)}
      >
        <Heart className={cn("h-5 w-5", isStarred && "fill-current")} />
      </IconButton>
      <IconButton label={t("player.lyrics")} onClick={() => setLyricsOpen(true)}>
        <MessageSquareQuote className="h-5 w-5" />
      </IconButton>
      <IconButton label={t("music.action.addToPlaylist")} onClick={() => setAddingToPlaylist(true)}>
        <ListPlus className="h-5 w-5" />
      </IconButton>
      <IconButton label={t("music.action.notFeelingIt")} disabled={dislike.isPending} onClick={() => dislike.mutate()}>
        <ThumbsDown className="h-5 w-5" />
      </IconButton>
      {lyricsOpen && <LyricsSheet track={{ id: track.trackId, title: track.title }} onClose={() => setLyricsOpen(false)} />}
      {addingToPlaylist && (
        <AddToPlaylistDialog trackIds={[track.trackId]} label={track.title} onClose={() => setAddingToPlaylist(false)} />
      )}
    </>
  );
}
