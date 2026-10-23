// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check, CheckSquare, ListMusic, Pencil, Play, ScanSearch, Share2, Shuffle, Sparkles, Ticket, Users, UserX, Disc3 } from "lucide-react";
import { useServerFeatures } from "../../lib/features";
import { artistImageUrl, deletePlaylist, deleteTrack, keepPlaylist, listAlbums, listPlaylistTracks, listTracks, getPlaylist, setReleaseKind, setTrackVisibility } from "../../api/music";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { formatDuration } from "../../lib/format";
import { playTracks, shuffleTracks } from "../../lib/play";
import { usePlayerStore, type QueueSource } from "../../store/playerStore";
import { albumKey } from "./albumKey";
import VirtualRows from "../../components/library/VirtualRows";
import EditAlbumDialog from "./EditAlbumDialog";
import ArtistAlbums from "./ArtistAlbums";
import { albumsOf, artistAlbumKey } from "./albumGroups";
import { cn } from "../../lib/cn";
import { Button, Cover, EmptyState, Skeleton, MenuItem, toast } from "../../components/ui";
import { EditPlaylistSheet, SharePlaylistDialog } from "./PlaylistTools";
import { MoreMenuTrigger } from "../../components/library/BrowseControls";
import { MediaRow } from "../../components/library/MediaCard";
import type { MusicTrack, Visibility } from "../../api/types";
import { useTrackActions } from "./useTrackActions";
import { MUSIC_KEYS } from "./musicKeys";
import { DownloadAllButton, HomeScreenButton, TrackTrailing } from "./Downloads";
import { IdentifyBatchSheet } from "./MusicTools";
import { BatchBar } from "../../components/library/BatchBar";
import { useSelection } from "../../lib/useSelection";
import { useMyPermissions } from "../../lib/permissions";
import { canDelete, moveToTrash } from "../../lib/trash";
import { useNavigate, useSearchParams } from "react-router-dom";
import { removeDownload } from "../../lib/offline/downloads";
import { runWithLimit } from "../../lib/uploadQueue";
import { useState } from "react";
import { useT } from "../../i18n";

export type MusicSelection =
  | { kind: "allSongs" }
  | { kind: "artist"; artist: string }
  | { kind: "album"; artist: string; album: string; id?: string }
  | { kind: "genre"; genre: string }
  | { kind: "playlist"; playlistId: string };

/* The detail column is always "the tracks of the one selected thing". Only
   playlists have their own endpoint; artist/album/genre are filtered from the
   track list, which the server has already scoped to what this viewer can see. */

export default function MusicDetail({ selection }: { selection: MusicSelection }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [sharingPlaylist, setSharingPlaylist] = useState(false);
  const [editingPlaylist, setEditingPlaylist] = useState(false);
  const [batchIdentify, setBatchIdentify] = useState<MusicTrack[] | null>(null);
  const [editingAlbum, setEditingAlbum] = useState<MusicTrack[] | null>(null);
  const trackActions = useTrackActions();
  const [sharing, setSharing] = useState(false);
  const setAddingToPlaylist = trackActions.setAddingToPlaylist;
  const picked = useSelection();
  const { isFamilyAdmin } = useMyPermissions();
  const isPlaylist = selection.kind === "playlist";
  const navigate = useNavigate();
  const allTracks = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks, enabled: !isPlaylist });
  const playlistTracks = useQuery({ queryKey: ["playlist-tracks", isPlaylist ? selection.playlistId : ""], queryFn: () => listPlaylistTracks((selection as { playlistId: string }).playlistId), enabled: isPlaylist });
  const playlist = useQuery({ queryKey: ["playlist", isPlaylist ? selection.playlistId : ""], queryFn: () => getPlaylist((selection as { playlistId: string }).playlistId), enabled: isPlaylist });
  // Every selection: the album header uses it, and so does any track without a picture of its own.
  const albums = useQuery({ queryKey: ["music-albums"], queryFn: () => listAlbums() });
  const { features } = useServerFeatures();
  // An album marked live is listed under Concerts (any member who sees it may mark it).
  const albumSummary = selection.kind === "album" && selection.id ? albums.data?.find((a) => a.id === selection.id) : undefined;
  const isConcert = albumSummary?.release_kind === "live";
  const concertMutation = useMutation({
    mutationFn: () => setReleaseKind(albumSummary!.release_group_id!, isConcert ? "album" : "live"),
    onSuccess: () => invalidate(),
    onError: () => toast.error(t("common.error.generic")),
  });

  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);

  const invalidate = () => MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));

  const visibilityMutation = useMutation({
    mutationFn: ({ id, v }: { id: string; v: Visibility }) => setTrackVisibility(id, v),
    onSuccess: (_d, { v }) => {
      invalidate();
      toast.success(t(v === "family" ? "music.toast.sharedWithFamily" : "music.toast.madePrivate"));
    },
    onError: () => toast.error(t("music.error.visibility")),
  });


  const selectedTracks: MusicTrack[] = useMemo(() => {
    if (isPlaylist) return (playlistTracks.data ?? []).map((e) => e.track);
    const list = allTracks.data ?? [];
    switch (selection.kind) {
      // "Unknown Artist"/"Unknown Album" are the server's grouping values, not UI text.
      case "artist": return list.filter((tr) => (tr.artist ?? "Unknown Artist") === selection.artist);
      case "album": return list.filter((tr) => selection.id && tr.album_id
          ? tr.album_id === selection.id
          : (tr.artist ?? "Unknown Artist") === selection.artist && (tr.album ?? "Unknown Album") === selection.album)
        .sort((a, b) => (a.track_number ?? 0) - (b.track_number ?? 0) || a.title.localeCompare(b.title));
      case "genre": return list.filter((tr) => (tr.genre ?? "") === selection.genre);
      default: return list;
    }
  }, [isPlaylist, playlistTracks.data, allTracks.data, selection]);

  /* Artist → albums → songs: an artist's songs split by album in a column of
     their own; the album picked is in the address, so a link keeps it. */
  const [searchParams, setSearchParams] = useSearchParams();
  const artistAlbums = useMemo(
    () => (selection.kind === "artist" ? albumsOf(selectedTracks, "Unknown Album") : []),
    [selection.kind, selectedTracks],
  );
  const splitByAlbum = artistAlbums.length > 1;
  const pickedAlbum = splitByAlbum ? searchParams.get("album") : null;
  const tracks = useMemo(
    () =>
      pickedAlbum
        ? selectedTracks
            .filter((tr) => artistAlbumKey(tr) === pickedAlbum)
            .sort((a, b) => (a.disc_number ?? 0) - (b.disc_number ?? 0) || (a.track_number ?? 0) - (b.track_number ?? 0) || a.title.localeCompare(b.title))
        : selectedTracks,
    [pickedAlbum, selectedTracks],
  );
  const pickAlbum = (key: string | null) =>
    setSearchParams(
      (params) => {
        const next = new URLSearchParams(params);
        if (key) next.set("album", key);
        else next.delete("album");
        return next;
      },
      { replace: true },
    );

  const ownedTracks = tracks.filter((t) => t.is_owner);
  const sharedCount = ownedTracks.filter((t) => t.visibility === "family").length;
  const allShared = ownedTracks.length > 0 && sharedCount === ownedTracks.length;
  const someShared = sharedCount > 0 && !allShared;

  /* Audiobooks and podcasts share as one item; an album is N separate tracks,
     so sharing one meant opening twelve edit sheets or selecting every row
     first. This applies the change to the whole selection at once. Only tracks
     you own — the server refuses the rest, and offering it would just produce
     failures. */
  async function shareAll(v: Visibility) {
    const mine = tracks.filter((t) => t.is_owner);
    if (mine.length === 0) return;
    setSharing(true);
    const { failed } = await runWithLimit(mine, 4, (t) => setTrackVisibility(t.id, v));
    invalidate();
    setSharing(false);
    const done = mine.length - failed.length;
    if (failed.length === 0) {
      toast.success(t(v === "family" ? "music.detail.sharedCount" : "music.detail.privateCount", { count: done }));
    } else {
      toast.error(t("music.detail.partialChanged", { done, total: mine.length }), t("music.detail.partialChangedBody"));
    }
  }

  const heading = isPlaylist ? playlist.data?.name ?? t("music.detail.playlist")
    : selection.kind === "artist" ? selection.artist
      : selection.kind === "album" ? selection.album
        : selection.kind === "genre" ? selection.genre
          : t("music.detail.allSongs");

  const subheading = isPlaylist ? playlist.data?.description ?? null
    : selection.kind === "album" ? selection.artist
      : pickedAlbum ? artistAlbums.find((a) => a.key === pickedAlbum)?.title ?? null
      : null;

  const cover = isPlaylist ? playlist.data?.cover_url ?? null
    : selection.kind === "album"
      ? albums.data?.find((a) => (selection.id ? a.id === selection.id : a.artist === selection.artist && a.album === selection.album))?.cover_url ?? tracks.find((t) => t.cover_url)?.cover_url ?? null
      : tracks.find((t) => t.cover_url)?.cover_url ?? null;

  // A track with no picture of its own shows its album's.
  const coverOf = (track: MusicTrack) =>
    track.album
      ? albums.data?.find((a) => a.album === track.album && a.artist === (track.album_artist ?? track.artist))?.cover_url ?? null
      : null;

  // So Now Playing can say where this queue came from and lead back here.
  const source: QueueSource | null =
    selection.kind === "playlist"
      ? { kind: "playlist", label: heading, path: `/music/playlists/${selection.playlistId}` }
      : selection.kind === "album"
        ? { kind: "album", label: heading, path: `/music/albums/${encodeURIComponent(albumKey(selection.artist, selection.album, selection.id))}` }
        : selection.kind === "artist"
          ? { kind: "artist", label: heading, path: `/music/artists/${encodeURIComponent(selection.artist)}` }
          : selection.kind === "genre"
            ? { kind: "genre", label: heading, path: `/music/genres/${encodeURIComponent(selection.genre)}` }
            : null;

  const totalSecs = tracks.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0);
  const isLoading = (isPlaylist ? playlistTracks.isLoading : allTracks.isLoading);

  const keep = useMutation({
    mutationFn: () => keepPlaylist((selection as { playlistId: string }).playlistId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["playlist"] });
      qc.invalidateQueries({ queryKey: ["playlists"] });
      toast.success(t("music.smart.kept"));
    },
    onError: () => toast.error(t("common.error.generic")),
  });
  // Back to Smart, where it came from; Undo in the toast brings it back.
  const trashGenerated = () => {
    const p = playlist.data;
    if (!p) return;
    return moveToTrash({
      title: p.name,
      remove: (batch) => deletePlaylist(p.id, batch),
      onChanged: () => qc.invalidateQueries({ queryKey: ["playlists"] }),
    })
      .then(() => navigate("/music"))
      .catch(() => toast.error(t("common.error.generic")));
  };

  const actions: { key: string; label: string; icon: ReactNode; onSelect: () => void; loading?: boolean }[] = [];
  if (!isPlaylist && tracks.length > 0)
    actions.push({ key: "playlist", label: t("music.action.addToPlaylist"), icon: <ListMusic className="h-4 w-4" />, onSelect: () => setAddingToPlaylist({ ids: tracks.map((tr) => tr.id), label: heading }) });
  if (!isPlaylist && tracks.length > 1 && tracks.some((tr) => tr.is_owner))
    actions.push({ key: "identify", label: t("music.action.identifyAll"), icon: <ScanSearch className="h-4 w-4" />, onSelect: () => setBatchIdentify(tracks.filter((tr) => tr.is_owner)) });
  // Reads the selection's current state so it offers the change that is actually available:
  // everything shared already only has "make private" left to do.
  if (!isPlaylist && ownedTracks.length > 0)
    actions.push({
      key: "share",
      label: allShared
        ? t("music.action.makePrivate")
        : someShared
          ? t("music.action.shareRest", { count: ownedTracks.length - sharedCount })
          : t("music.action.shareWithFamily"),
      icon: allShared ? <UserX className="h-4 w-4" /> : <Users className="h-4 w-4" />,
      loading: sharing,
      onSelect: () => void shareAll(allShared ? "private" : "family"),
    });
  if (isPlaylist && playlist.data?.is_owner)
    actions.push({ key: "share", label: t("music.share.action"), icon: <Share2 className="h-4 w-4" />, onSelect: () => setSharingPlaylist(true) });
  if (features.track_chapters && albumSummary?.release_group_id)
    actions.push({
      key: "concert",
      label: t(isConcert ? "music.album.unmarkConcert" : "music.album.markConcert"),
      icon: <Ticket className="h-4 w-4" />,
      loading: concertMutation.isPending,
      onSelect: () => concertMutation.mutate(),
    });
  if (isPlaylist && playlist.data?.is_owner)
    actions.push({ key: "edit", label: t("common.action.edit"), icon: <Pencil className="h-4 w-4" />, onSelect: () => setEditingPlaylist(true) });

  if (isLoading) return <div className="space-y-3 p-6"><Skeleton className="h-32 w-32" /><Skeleton className="h-6 w-48" />{[...Array(6)].map((_, i) => <Skeleton key={i} className="h-11" />)}</div>;

  const askToKeep = isPlaylist && playlist.data?.is_owner && playlist.data.generated_at && !playlist.data.kept_at;

  return (
    <div className="pb-8">
      {/* Every play of a smart playlist saves one; this is where the listener decides. */}
      {askToKeep && (
        <div className="mx-4 mt-2 flex flex-wrap items-center gap-x-2 gap-y-2 rounded-card bg-accent/10 px-4 py-3 sm:mx-6">
          <p className="flex min-w-0 flex-1 basis-full items-center gap-2 text-sm sm:basis-0">
            <Sparkles className="h-4 w-4 shrink-0 text-accent-text" />
            {t("music.smart.keepQuestion")}
          </p>
          <span className="flex-1 sm:hidden" />
          <Button size="sm" variant="ghost" onClick={() => void trashGenerated()}>{t("common.action.delete")}</Button>
          <Button size="sm" icon={<Check className="h-4 w-4" />} loading={keep.isPending} onClick={() => keep.mutate()}>{t("music.smart.keep")}</Button>
        </div>
      )}
      {/* On a phone: a big centred picture, then Play and Shuffle side by side; the rest of the
          actions are a menu with words instead of a wall of wrapping buttons. */}
      <div className="flex flex-col items-center gap-5 px-4 pb-6 pt-2 text-center sm:flex-row sm:flex-wrap sm:items-end sm:p-6 sm:text-left">
        <Cover
          kind="music"
          src={selection.kind === "artist" ? artistImageUrl(selection.artist) : cover}
          round={selection.kind === "artist"}
          alt={heading}
          aspect="square"
          className="w-48 shrink-0 shadow-card sm:w-32"
        />
        <div className="w-full min-w-0 flex-1">
          <h1 className="text-2xl font-semibold leading-tight tracking-tight">{heading}</h1>
          {subheading && <p className="mt-1 text-sm text-muted">{subheading}</p>}
          <p className="mt-1 text-xs text-muted">
            {totalSecs > 0
              ? t("music.count.songsWithDuration", { count: tracks.length, duration: formatDuration(totalSecs) })
              : t("music.count.songs", { count: tracks.length })}
          </p>
          <div className="mt-5 flex flex-wrap items-center gap-2 sm:mt-4">
            <Button size="lg" className="max-sm:flex-1" icon={<Play className="h-4 w-4 fill-current" />} disabled={tracks.length === 0} onClick={() => void playTracks(tracks, 0, source)}>{t("common.action.play")}</Button>
            <Button size="lg" variant="secondary" className="max-sm:flex-1" icon={<Shuffle className="h-4 w-4" />} disabled={tracks.length === 0} onClick={() => void shuffleTracks(tracks, source)}>{t("music.action.shuffle")}</Button>
            {/* Stays a button on every screen: it shows the download's progress. */}
            {isPlaylist ? <HomeScreenButton playlistId={selection.playlistId} /> : <DownloadAllButton tracks={tracks} />}
            {actions.map((a) => (
              <Button key={a.key} size="lg" variant="secondary" className="max-sm:hidden" loading={a.loading} icon={a.icon} onClick={a.onSelect}>
                {a.label}
              </Button>
            ))}
            {actions.length > 0 && (
              <span className="sm:hidden">
                <MoreMenuTrigger size="lg">
                  {actions.map((a) => (
                    <MenuItem key={a.key} icon={a.icon} disabled={a.loading} onSelect={a.onSelect}>{a.label}</MenuItem>
                  ))}
                </MoreMenuTrigger>
              </span>
            )}
          </div>
        </div>
      </div>

      <div className={cn("px-3", splitByAlbum && "lg:grid lg:grid-cols-[minmax(13rem,17rem)_minmax(0,1fr)] lg:gap-3")}>
        {splitByAlbum && <ArtistAlbums albums={artistAlbums} total={selectedTracks.length} picked={pickedAlbum} onPick={pickAlbum} />}
        <div className="min-w-0">
        {tracks.length === 0 ? (
          <EmptyState title={t("music.detail.noSongs")} description={t(isPlaylist ? "music.detail.noSongsPlaylist" : "music.detail.noSongsSelection")} />
        ) : (
          // Only the rows on screen: "all songs" is the whole library, and 9,000
          // rows with their menus took 340,000 DOM nodes and 400 MB.
          <VirtualRows key={JSON.stringify(selection)} items={tracks} keyOf={(track, i) => `${track.id}-${i}`} render={(track, i) => {
            const active = current?.kind === "music" && current.trackId === track.id;
            return (
              <MediaRow
                key={`${track.id}-${i}`}
                kind="music"
                index={i + 1}
                title={track.title}
                subtitle={[track.artist, track.album].filter(Boolean).join(" — ") || null}
                cover={track.cover_url ?? coverOf(track)}
                active={active}
                playing={active && playing}
                family={track.visibility === "family"}
                selected={picked.has(track.id)}
                trailing={<TrackTrailing id={track.id} durationSecs={track.duration_secs} identified={!!track.musicbrainz_recording_id} />}
                onClick={() => (picked.count > 0 ? picked.toggle(track.id) : void playTracks(tracks, i, source))}
                onPlay={() => void playTracks(tracks, i, source)}
                menu={
                  <MoreMenuTrigger>
                    {/* Selecting for a batch and changing visibility are this
                        column's own state, so they stay here; the rest is the
                        shared per-track set. */}
                    <MenuItem icon={<CheckSquare />} onSelect={() => picked.toggle(track.id)}>{t("music.action.select")}</MenuItem>
                    {trackActions.items(track, () => void playTracks(tracks, i, source))}
                    {track.is_owner && (
                      <MenuItem
                        icon={track.visibility === "family" ? <UserX /> : <Users />}
                        onSelect={() => visibilityMutation.mutate({ id: track.id, v: track.visibility === "family" ? "private" : "family" })}
                      >
                        {t(track.visibility === "family" ? "music.action.makePrivate" : "music.action.shareWithFamily")}
                      </MenuItem>
                    )}
                  </MoreMenuTrigger>
                }
              />
            );
          }} />
        )}
        </div>
      </div>

      <BatchBar
        selected={tracks
          .filter((tr) => picked.has(tr.id))
          .map((tr) => ({ id: tr.id, label: tr.title, isOwner: tr.is_owner, canDelete: canDelete(tr, isFamilyAdmin) }))}
        onClear={picked.clear}
        extraActions={
          <>
            <Button
              size="sm"
              variant="secondary"
              icon={<ListMusic className="h-4 w-4" />}
              onClick={() => setAddingToPlaylist({ ids: tracks.filter((x) => picked.has(x.id)).map((x) => x.id), label: t("music.detail.yourSelection") })}
            >
              {t("music.action.addToPlaylist")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              icon={<ScanSearch className="h-4 w-4" />}
              onClick={() => setBatchIdentify(tracks.filter((tr) => picked.has(tr.id) && tr.is_owner))}
            >
              {t("music.action.identify")}
            </Button>
            <Button
              size="sm"
              variant="secondary"
              icon={<Disc3 className="h-4 w-4" />}
              disabled={!tracks.some((tr) => picked.has(tr.id) && tr.is_owner)}
              onClick={() => setEditingAlbum(tracks.filter((tr) => picked.has(tr.id)))}
            >
              {t("music.action.editAlbum")}
            </Button>
          </>
        }
        onSetVisibility={async (id, v) => {
          await setTrackVisibility(id, v);
          invalidate();
        }}
        onTrash={async (id, batch) => {
          await deleteTrack(id, batch);
          await removeDownload(id).catch(() => {});
        }}
        afterTrash={invalidate}
      />


      {trackActions.dialogs}
      {sharingPlaylist && playlist.data && (
        <SharePlaylistDialog
          playlist={playlist.data}
          privateSongs={new Set(tracks.filter((tr) => tr.is_owner && tr.visibility === "private").map((tr) => tr.id)).size}
          onClose={() => setSharingPlaylist(false)}
        />
      )}
      {editingPlaylist && playlist.data && <EditPlaylistSheet playlist={playlist.data} onClose={() => setEditingPlaylist(false)} />}
      {editingAlbum && (
        <EditAlbumDialog
          tracks={editingAlbum}
          onClose={() => setEditingAlbum(null)}
          onSaved={() => {
            picked.clear();
            invalidate();
          }}
        />
      )}
      {batchIdentify && batchIdentify.length > 0 && (
        <IdentifyBatchSheet
          tracks={batchIdentify}
          label={selection.kind === "album" ? selection.album : selection.kind === "artist" ? selection.artist : t("music.detail.theseTracks")}
          onClose={() => setBatchIdentify(null)}
        />
      )}
    </div>
  );
}
