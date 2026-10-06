// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Copy, Disc3, Download, ListMusic, ListPlus, Plus, Sparkles, Tag, User } from "lucide-react";
import { MusicIcon } from "../../components/ui/CloudIcon";
import { artistImageUrl, listAlbums, listArtists, listGenres, listPlaylists, listTracks } from "../../api/music";
import { SplitView, ColumnHeader } from "../../components/shell/SplitView";
import SectionTheme from "../../components/shell/SectionTheme";
import { MediaCard, MediaGrid, MediaRow } from "../../components/library/MediaCard";
import { ViewToggle, SortMenu, FilterField, FilterChips, MoreMenuTrigger } from "../../components/library/BrowseControls";
import { usePersistedState } from "../../lib/persistedState";
import { Button, EmptyState, IconButton, Skeleton } from "../../components/ui";
import SmartPlaylists from "./SmartPlaylists";
import DownloadsView, { TrackTrailing } from "./Downloads";
import MusicDetail, { type MusicSelection } from "./MusicDetail";
import { useTrackActions } from "./useTrackActions";
import { albumKey, parseAlbumKey } from "./albumKey";
import UploadMusicDialog from "./UploadMusicDialog";
import { CreatePlaylistDialog } from "./PlaylistTools";
import { playTracks } from "../../lib/play";
import { useMyPermissions } from "../../lib/permissions";
import { useT, type PlainKey } from "../../i18n";

export type MusicMode = "songs" | "artists" | "albums" | "genres" | "playlists" | "smart" | "downloads";
type Sort = "name" | "count" | "recent";

const MODES: { value: MusicMode; icon: React.ReactNode; label: PlainKey }[] = [
  { value: "songs", icon: <MusicIcon />, label: "music.mode.songs" },
  { value: "artists", icon: <User />, label: "music.mode.artists" },
  { value: "albums", icon: <Disc3 />, label: "music.mode.albums" },
  { value: "genres", icon: <Tag />, label: "music.mode.genres" },
  { value: "playlists", icon: <ListMusic />, label: "music.mode.playlists" },
  { value: "smart", icon: <Sparkles />, label: "music.mode.smart" },
  { value: "downloads", icon: <Download />, label: "music.mode.downloads" },
];

const SORTS: { value: Sort; label: PlainKey }[] = [
  { value: "name", label: "music.sort.name" },
  { value: "count", label: "music.sort.mostTracks" },
  { value: "recent", label: "music.sort.recent" },
];


export default function MusicPage() {
  const params = useParams();
  const { pathname } = useLocation();
  const uploading = pathname === "/music/upload";
  const navigate = useNavigate();
  const [savedMode, setMode] = usePersistedState<MusicMode>("own-audio-music-mode", "songs");
  // Its own URL, so the offline banner can link straight to it.
  const mode: MusicMode = pathname === "/music/downloads" ? "downloads" : savedMode;
  const [view, setView] = usePersistedState<"grid" | "list">("own-audio-music-view", "list");
  const [sort, setSort] = usePersistedState<Sort>("own-audio-music-sort", "name");
  // Only meaningful for the flat Songs list — an album or artist can already mix tracks from
  // different owners, so "mine" doesn't cleanly apply to those groupings.
  const [owner, setOwner] = usePersistedState<"all" | "mine">("own-audio-music-owner", "all");
  const [filter, setFilter] = useState("");
  const [creatingPlaylist, setCreatingPlaylist] = useState(false);
  const { canUpload } = useMyPermissions();
  const trackActions = useTrackActions();
  const { t } = useT();

  const selection: MusicSelection | null = params.artist
    ? { kind: "artist", artist: decodeURIComponent(params.artist) }
    : params.albumKey
      ? { kind: "album", ...parseAlbumKey(decodeURIComponent(params.albumKey)) }
      : params.genre
        ? { kind: "genre", genre: decodeURIComponent(params.genre) }
        : params.playlistId
          ? { kind: "playlist", playlistId: params.playlistId }
          : mode === "songs"
            ? { kind: "allSongs" }
            : null;

  const tracks = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks, enabled: mode === "songs" });
  const artists = useQuery({ queryKey: ["music-artists"], queryFn: listArtists, enabled: mode === "artists" });
  // Songs use it too: a track with no picture of its own shows its album's.
  const albums = useQuery({ queryKey: ["music-albums"], queryFn: () => listAlbums(), enabled: mode === "albums" || mode === "songs" });
  const genres = useQuery({ queryKey: ["music-genres"], queryFn: listGenres, enabled: mode === "genres" });
  const playlists = useQuery({ queryKey: ["playlists"], queryFn: listPlaylists, enabled: mode === "playlists" });

  const albumCover = useMemo(
    () => new Map((albums.data ?? []).filter((a) => a.cover_url).map((a) => [albumKey(a.artist, a.album), a.cover_url])),
    [albums.data]
  );
  const coverOf = (track: { cover_url: string | null; album: string | null; album_artist?: string | null; artist: string | null }) =>
    track.cover_url ?? (track.album ? albumCover.get(albumKey(track.album_artist ?? track.artist ?? "", track.album)) ?? null : null);

  const isLoading = [tracks, artists, albums, genres, playlists].some((q) => q.isLoading && q.fetchStatus !== "idle");
  const q = filter.trim().toLowerCase();

  const rows = useMemo(() => {
    const by = <T,>(list: T[], name: (x: T) => string, count: (x: T) => number, recent: (x: T) => string) => {
      const filtered = list.filter((x) => !q || name(x).toLowerCase().includes(q));
      return [...filtered].sort((a, b) =>
        sort === "count" ? count(b) - count(a) : sort === "recent" ? recent(b).localeCompare(recent(a)) : name(a).localeCompare(name(b))
      );
    };
    switch (mode) {
      case "songs": {
        let list = (tracks.data ?? []).filter((t) => !q || t.title.toLowerCase().includes(q) || (t.artist ?? "").toLowerCase().includes(q) || (t.album ?? "").toLowerCase().includes(q));
        if (owner === "mine") list = list.filter((t) => t.visibility === "private");
        return [...list].sort((a, b) => (sort === "recent" ? b.created_at.localeCompare(a.created_at) : a.title.localeCompare(b.title)));
      }
      case "artists": return by(artists.data ?? [], (a) => a.artist, (a) => a.track_count, () => "");
      case "albums": return by(albums.data ?? [], (a) => `${a.album} ${a.artist}`, (a) => a.track_count, () => "");
      case "genres": return by(genres.data ?? [], (g) => g.genre, (g) => g.track_count, () => "");
      case "playlists": return by(playlists.data ?? [], (p) => p.name, (p) => p.track_count, (p) => p.updated_at);
      // Smart playlists are not rows to sort or filter — they are built on
      // demand, so this mode renders its own section instead. Same for
      // downloads, which come from the device rather than the server.
      case "smart":
      case "downloads": return [];
    }
  }, [mode, q, sort, owner, tracks.data, artists.data, albums.data, genres.data, playlists.data]);

  function renderRows() {
    if (mode === "smart") return <SmartPlaylists />;
    if (mode === "downloads") return <DownloadsView />;
    if (rows.length === 0) {
      return (
        <EmptyState
          icon={<MusicIcon />}
          title={t(q ? "music.browse.noMatch" : mode === "playlists" ? "music.browse.noPlaylists" : "music.browse.noMusic")}
          description={t(q ? "music.browse.noMatchHint" : "music.browse.emptyHint")}
          action={!q && canUpload && <Button onClick={() => navigate("/music/upload")}>{t("music.action.addMusic")}</Button>}
        />
      );
    }

    switch (mode) {
      case "songs": {
        const list = rows as NonNullable<typeof tracks.data>;
        /* Playing starts from the list as filtered, so "filter, then hit the
           one you wanted" queues what you are looking at rather than the whole
           library. */
        const play = (i: number) => () => void playTracks(list, i);
        return view === "grid" ? (
          <MediaGrid dense>
            {list.map((track, i) => (
              <MediaCard
                key={track.id}
                kind="music"
                title={track.title}
                subtitle={track.artist}
                cover={coverOf(track)}
                onClick={play(i)}
                menu={<MoreMenuTrigger>{trackActions.items(track, play(i))}</MoreMenuTrigger>}
              />
            ))}
          </MediaGrid>
        ) : (
          <div className="p-3">
            {list.map((track, i) => (
              <MediaRow
                key={track.id}
                kind="music"
                index={i + 1}
                title={track.title}
                subtitle={[track.artist, track.album].filter(Boolean).join(" — ") || null}
                cover={coverOf(track)}
                trailing={<TrackTrailing id={track.id} durationSecs={track.duration_secs} />}
                onClick={play(i)}
                onPlay={play(i)}
                menu={<MoreMenuTrigger>{trackActions.items(track, play(i))}</MoreMenuTrigger>}
              />
            ))}
          </div>
        );
      }
      case "artists": {
        const list = rows as NonNullable<typeof artists.data>;
        const props = (a: (typeof list)[number]) => ({
          kind: "music" as const, title: a.artist, cover: artistImageUrl(a.artist), roundCover: true,
          subtitle: t("music.count.albumsAndSongs", { albums: a.album_count, songs: a.track_count }),
          selected: selection?.kind === "artist" && selection.artist === a.artist,
          onClick: () => navigate(`/music/artists/${encodeURIComponent(a.artist)}`),
        });
        return view === "grid"
          ? <MediaGrid dense>{list.map((a) => <MediaCard key={a.artist} {...props(a)} />)}</MediaGrid>
          : <div className="p-3">{list.map((a) => <MediaRow key={a.artist} {...props(a)} />)}</div>;
      }
      case "albums": {
        const list = rows as NonNullable<typeof albums.data>;
        const props = (a: (typeof list)[number]) => ({
          kind: "music" as const, title: a.album, subtitle: a.artist, cover: a.cover_url,
          selected: selection?.kind === "album" && selection.album === a.album && selection.artist === a.artist,
          onClick: () => navigate(`/music/albums/${encodeURIComponent(albumKey(a.artist, a.album))}`),
        });
        return view === "grid"
          ? <MediaGrid>{list.map((a) => <MediaCard key={albumKey(a.artist, a.album)} {...props(a)} meta={t("music.count.songs", { count: a.track_count })} />)}</MediaGrid>
          : <div className="p-3">{list.map((a) => <MediaRow key={albumKey(a.artist, a.album)} {...props(a)} trailing={t("music.count.songs", { count: a.track_count })} />)}</div>;
      }
      case "genres": {
        const list = rows as NonNullable<typeof genres.data>;
        return <div className="p-3">{list.map((g) => (
          <MediaRow key={g.genre} kind="music" title={g.genre} subtitle={t("music.count.songs", { count: g.track_count })} selected={selection?.kind === "genre" && selection.genre === g.genre} onClick={() => navigate(`/music/genres/${encodeURIComponent(g.genre)}`)} />
        ))}</div>;
      }
      case "playlists": {
        const list = rows as NonNullable<typeof playlists.data>;
        const props = (p: (typeof list)[number]) => ({
          kind: "music" as const, title: p.name, subtitle: t("music.count.songs", { count: p.track_count }), cover: p.cover_url,
          selected: selection?.kind === "playlist" && selection.playlistId === p.id,
          onClick: () => navigate(`/music/playlists/${p.id}`),
        });
        return view === "grid"
          ? <MediaGrid>{list.map((p) => <MediaCard key={p.id} {...props(p)} />)}</MediaGrid>
          : <div className="p-3">{list.map((p) => <MediaRow key={p.id} {...props(p)} />)}</div>;
      }
    }
  }

  const content = (
    <>
      <ColumnHeader
        title={t("common.kind.music")}
        actions={
          <>
            <IconButton size="sm" label={t("music.browse.findDuplicates")} onClick={() => navigate("/music/duplicates")}>
              <Copy className="h-4 w-4" />
            </IconButton>
            {mode === "playlists" ? (
              <Button size="sm" icon={<ListPlus className="h-4 w-4" />} onClick={() => setCreatingPlaylist(true)}>{t("music.action.newPlaylist")}</Button>
            ) : (
              canUpload && <Button size="sm" icon={<Plus className="h-4 w-4" />} onClick={() => navigate("/music/upload")}>{t("common.action.add")}</Button>
            )}
          </>
        }
      >
        {/* Smart playlists and Downloads build their own lists; filter, sort and view do nothing there. */}
        {mode !== "smart" && mode !== "downloads" && <div className="flex items-center gap-1.5">
          <FilterField value={filter} onChange={setFilter} placeholder={t("music.browse.filter")} className="min-w-0 flex-1" />
          <SortMenu value={sort} onChange={setSort} options={SORTS.map((s) => ({ value: s.value, label: t(s.label) }))} />
          <ViewToggle value={view} onChange={setView} />
        </div>}
        {/* "Mine" only filters the flat Songs list — an album or artist can mix owners. */}
        <FilterChips<MusicMode>
          value={mode}
          onChange={(m) => { setMode(m); navigate(m === "downloads" ? "/music/downloads" : "/music"); }}
          toggle={mode === "songs" ? { label: t("music.browse.mine"), on: owner === "mine", onChange: (on) => setOwner(on ? "mine" : "all") } : undefined}
          chips={MODES.map((m) => ({ value: m.value, label: <span className="flex items-center gap-1.5 [&>svg]:h-4 [&>svg]:w-4">{m.icon}{t(m.label)}</span> }))}
        />
      </ColumnHeader>
      {isLoading ? <MediaGrid>{[...Array(6)].map((_, i) => <Skeleton key={i} className="aspect-square" />)}</MediaGrid> : renderRows()}
    </>
  );

  return (
    <SectionTheme cloud="music">
      {trackActions.dialogs}
      {uploading && <UploadMusicDialog />}
      {creatingPlaylist && <CreatePlaylistDialog onClose={() => setCreatingPlaylist(false)} onCreated={(p) => navigate(`/music/playlists/${p.id}`)} />}
    <SplitView
      contentWidth="medium"
      widthKey="music"
      hasDetail={!!selection && selection.kind !== "allSongs"}
      onBack={() => navigate("/music")}
      content={content}
      detail={selection ? <MusicDetail selection={selection} /> : <EmptyState icon={<MusicIcon />} title={t("music.browse.pickTitle")} description={t("music.browse.pickDescription")} />}
    />
    </SectionTheme>
  );
}
