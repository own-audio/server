// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { ArrowDownCircle, Clock, Download, ExternalLink, Loader2, Play, Shuffle, Smartphone, Trash2, WifiOff, X } from "lucide-react";
import { Button, EmptyState, IconButton, MenuItem, toast } from "../../components/ui";
import { MediaRow } from "../../components/library/MediaCard";
import { MoreMenuTrigger } from "../../components/library/BrowseControls";
import { formatDuration } from "../../lib/format";
import { playTracks, shuffleTracks } from "../../lib/play";
import { usePlayerStore } from "../../store/playerStore";
import {
  cancelAllDownloads,
  cancelDownload,
  clearDownloads,
  downloadTracks,
  HomePlaylistLimitError,
  MAX_HOME_PLAYLISTS,
  removeDownload,
  removePlaylistOffline,
  savePlaylistOffline,
  useDownloads,
} from "../../lib/offline/downloads";
import type { MusicTrack } from "../../api/types";
import { intlLocale, useT } from "../../i18n";

function formatBytes(n: number): string {
  const num = (v: number, digits: number) =>
    new Intl.NumberFormat(intlLocale(), { minimumFractionDigits: digits, maximumFractionDigits: digits }).format(v);
  if (n < 1024 * 1024) return `${num(Math.max(1, Math.round(n / 1024)), 0)} KB`;
  if (n < 1024 * 1024 * 1024) return `${num(n / (1024 * 1024), n < 10 * 1024 * 1024 ? 1 : 0)} MB`;
  return `${num(n / (1024 * 1024 * 1024), 1)} GB`;
}

/** The marker on a song row: on this device, waiting, or coming down now. */
export function DownloadBadge({ id }: { id: string }) {
  const downloaded = useDownloads((s) => !!s.downloaded[id]);
  const queued = useDownloads((s) => s.queue.some((q) => q.id === id));
  const active = useDownloads((s) => s.active === id);
  const { t } = useT();
  if (!downloaded && !queued) return null;
  const icon = downloaded ? (
    <ArrowDownCircle className="h-3.5 w-3.5 text-accent" aria-label={t("music.downloads.downloaded")} />
  ) : active ? (
    <Loader2 className="h-3.5 w-3.5 animate-spin" aria-label={t("music.downloads.downloading")} />
  ) : (
    <Clock className="h-3.5 w-3.5" aria-label={t("music.downloads.waiting")} />
  );
  return <span className="mr-1.5 inline-flex align-[-2px]">{icon}</span>;
}

/** Row trailing: the marker plus the duration, as one element. */
export function TrackTrailing({ id, durationSecs }: { id: string; durationSecs: number | null }) {
  return (
    <>
      <DownloadBadge id={id} />
      {durationSecs ? formatDuration(durationSecs) : null}
    </>
  );
}

/** The per-song menu entry; which verb depends on where the song is now. */
export function DownloadMenuItem({ track }: { track: MusicTrack }) {
  const downloaded = useDownloads((s) => !!s.downloaded[track.id]);
  const queued = useDownloads((s) => s.queue.some((q) => q.id === track.id));
  const { t } = useT();
  if (downloaded) {
    return (
      <MenuItem icon={<Trash2 />} onSelect={() => void removeDownload(track.id)}>
        {t("music.downloads.remove")}
      </MenuItem>
    );
  }
  if (queued) {
    return (
      <MenuItem icon={<X />} onSelect={() => void cancelDownload(track.id)}>
        {t("music.downloads.cancel")}
      </MenuItem>
    );
  }
  return (
    <MenuItem
      icon={<Download />}
      onSelect={() => {
        void downloadTracks([track]);
        toast.show(t("music.downloads.downloading"), track.title);
      }}
    >
      {t("music.downloads.download")}
    </MenuItem>
  );
}

/** "Download" for a whole playlist, album, artist or genre. */
export function DownloadAllButton({ tracks }: { tracks: MusicTrack[] }) {
  const downloaded = useDownloads((s) => s.downloaded);
  const queue = useDownloads((s) => s.queue);
  const ready = useDownloads((s) => s.ready);
  const { t } = useT();
  if (!ready || tracks.length === 0) return null;

  const have = tracks.filter((tr) => downloaded[tr.id]).length;
  const waiting = tracks.filter((tr) => queue.some((q) => q.id === tr.id)).length;

  if (waiting > 0) {
    return (
      <Button
        size="lg"
        variant="secondary"
        icon={<Loader2 className="h-4 w-4 animate-spin" />}
        onClick={() => tracks.forEach((tr) => void cancelDownload(tr.id))}
        title={t("music.downloads.cancelRest")}
      >
        {t("music.downloads.progress", { count: have, total: tracks.length })}
      </Button>
    );
  }
  if (have === tracks.length) {
    return (
      <Button
        size="lg"
        variant="secondary"
        icon={<Trash2 className="h-4 w-4" />}
        onClick={() => {
          tracks.forEach((tr) => void removeDownload(tr.id));
          toast.show(t("music.downloads.removed"), t("music.count.songs", { count: have }));
        }}
      >
        {t("music.downloads.remove")}
      </Button>
    );
  }
  return (
    <Button
      size="lg"
      variant="secondary"
      icon={<Download className="h-4 w-4" />}
      onClick={async () => {
        const added = await downloadTracks(tracks);
        toast.show(t("music.downloads.downloading"), t("music.downloads.started", { count: added }));
      }}
    >
      {have > 0 ? t("music.downloads.downloadRest", { count: tracks.length - have }) : t("music.downloads.download")}
    </Button>
  );
}

/**
 * A playlist's header button. It keeps the whole playlist on the device and
 * opens its own player page, which is where the browser's "Add to Home Screen"
 * picks up the playlist's name and icon — a web page cannot add an icon by
 * itself, so this is as close as it gets to one tap.
 */
export function HomeScreenButton({ playlistId }: { playlistId: string }) {
  const navigate = useNavigate();
  const saved = useDownloads((s) => !!s.playlists[playlistId]);
  const ready = useDownloads((s) => s.ready);
  const [busy, setBusy] = useState(false);
  const { t } = useT();
  if (!ready) return null;

  return (
    <Button
      size="lg"
      variant="secondary"
      loading={busy}
      icon={<Smartphone className="h-4 w-4" />}
      onClick={async () => {
        if (saved) {
          navigate(`/play/${playlistId}`);
          return;
        }
        setBusy(true);
        try {
          await savePlaylistOffline(playlistId);
          navigate(`/play/${playlistId}`);
        } catch (e) {
          if (e instanceof HomePlaylistLimitError) {
            toast.error(t("music.downloads.homeLimit", { max: MAX_HOME_PLAYLISTS }), t("music.downloads.homeLimitBody"));
          } else {
            toast.error(t("music.downloads.savePlaylistFailed"), t("music.downloads.checkConnection"));
          }
        } finally {
          setBusy(false);
        }
      }}
    >
      {t(saved ? "music.downloads.openPlayer" : "music.downloads.addToHomeScreen")}
    </Button>
  );
}

function HomePlaylists() {
  const navigate = useNavigate();
  const playlists = useDownloads((s) => s.playlists);
  const downloaded = useDownloads((s) => s.downloaded);
  const list = Object.values(playlists).sort((a, b) => a.name.localeCompare(b.name));
  const { t } = useT();
  if (list.length === 0) return null;

  return (
    <section className="rounded-card border border-border p-4">
      <p className="flex items-center gap-2 text-sm font-medium">
        <Smartphone className="h-4 w-4" /> {t("music.downloads.homePlaylists", { count: list.length, max: MAX_HOME_PLAYLISTS })}
      </p>
      <p className="mt-1 text-xs text-muted">{t("music.downloads.homePlaylistsHint")}</p>
      <ul className="mt-3 space-y-1">
        {list.map((p) => {
          const have = p.tracks.filter((tr) => downloaded[tr.id]).length;
          return (
            <li key={p.id} className="flex items-center justify-between gap-2 text-sm">
              <span className="min-w-0 truncate">
                {p.name}
                <span className="ml-2 text-xs text-muted">
                  {t("music.count.ofTotal", { count: have, total: p.tracks.length })}
                </span>
              </span>
              <span className="flex shrink-0 gap-1">
                <IconButton size="sm" label={t("music.downloads.open", { name: p.name })} onClick={() => navigate(`/play/${p.id}`)}>
                  <ExternalLink className="h-3.5 w-3.5" />
                </IconButton>
                <IconButton size="sm" label={t("music.downloads.stopKeeping", { name: p.name })} onClick={() => void removePlaylistOffline(p.id)}>
                  <X className="h-3.5 w-3.5" />
                </IconButton>
              </span>
            </li>
          );
        })}
      </ul>
    </section>
  );
}

function QueueStatus() {
  const queue = useDownloads((s) => s.queue);
  const active = useDownloads((s) => s.active);
  const progress = useDownloads((s) => s.progress);
  const waiting = useDownloads((s) => s.waitingForNetwork);
  const full = useDownloads((s) => s.storageFull);
  const { t } = useT();
  if (queue.length === 0) return null;

  const current = queue.find((q) => q.id === active);
  const pct = progress?.total ? Math.round((progress.loaded / progress.total) * 100) : null;

  return (
    <section className="rounded-card border border-border p-4">
      <div className="flex items-center justify-between gap-3">
        <p className="flex items-center gap-2 text-sm font-medium">
          {waiting ? <WifiOff className="h-4 w-4" /> : <Loader2 className="h-4 w-4 animate-spin" />}
          {t("music.downloads.left", { count: queue.length })}
        </p>
        <Button size="sm" variant="ghost" onClick={() => void cancelAllDownloads()}>
          {t("music.downloads.cancelAll")}
        </Button>
      </div>
      <p className="mt-1 text-xs text-muted">
        {full
          ? t("music.downloads.full")
          : waiting
            ? t("music.downloads.waitingForNetwork")
            : current
              ? pct != null
                ? t("music.downloads.nowPercent", { title: current.track.title, pct })
                : t("music.downloads.now", { title: current.track.title })
              : t("music.downloads.starting")}
      </p>
      {pct != null && !waiting && (
        <div className="mt-2 h-1.5 overflow-hidden rounded-full bg-border">
          <div className="h-full rounded-full bg-accent transition-[width] duration-300" style={{ width: `${pct}%` }} />
        </div>
      )}
      <ul className="mt-3 space-y-1">
        {queue.slice(0, 50).map((q) => (
          <li key={q.id} className="flex items-center justify-between gap-2 text-sm">
            <span className="truncate text-muted">
              {q.track.artist ? `${q.track.artist} — ` : ""}
              {q.track.title}
            </span>
            <IconButton size="sm" label={t("music.downloads.cancelItem", { title: q.track.title })} onClick={() => void cancelDownload(q.id)}>
              <X className="h-3.5 w-3.5" />
            </IconButton>
          </li>
        ))}
        {queue.length > 50 && <li className="text-xs text-muted">{t("music.count.andMore", { count: queue.length - 50 })}</li>}
      </ul>
    </section>
  );
}

/**
 * Everything on this device. Reads only IndexedDB, never the server, so it is
 * the one music view that works with no connection at all.
 */
export default function DownloadsView() {
  const downloaded = useDownloads((s) => s.downloaded);
  const ready = useDownloads((s) => s.ready);
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const { t } = useT();

  const entries = useMemo(
    // Download order, so an album or playlist fetched in one go plays in its order.
    () => Object.values(downloaded).sort((a, b) => a.downloadedAt - b.downloadedAt),
    [downloaded]
  );
  const tracks = entries.map((e) => e.track);
  const bytes = entries.reduce((sum, e) => sum + e.bytes, 0);
  const secs = tracks.reduce((sum, tr) => sum + (tr.duration_secs ?? 0), 0);

  if (!ready) {
    return <EmptyState icon={<Download />} title={t("music.downloads.unavailableTitle")} description={t("music.downloads.unavailableBody")} />;
  }

  return (
    <div className="space-y-4 p-3">
      <QueueStatus />
      <HomePlaylists />

      {tracks.length === 0 ? (
        <EmptyState
          icon={<Download />}
          title={t("music.downloads.emptyTitle")}
          description={t("music.downloads.emptyBody")}
        />
      ) : (
        <>
          <div className="flex flex-wrap items-center justify-between gap-3 px-2">
            <p className="text-xs text-muted">
              {secs > 0
                ? t("music.downloads.summaryWithDuration", { count: tracks.length, duration: formatDuration(secs), size: formatBytes(bytes) })
                : t("music.downloads.summary", { count: tracks.length, size: formatBytes(bytes) })}
            </p>
            <div className="flex gap-2">
              <Button size="sm" icon={<Play className="h-4 w-4 fill-current" />} onClick={() => void playTracks(tracks, 0)}>
                {t("common.action.play")}
              </Button>
              <Button size="sm" variant="secondary" icon={<Shuffle className="h-4 w-4" />} onClick={() => void shuffleTracks(tracks)}>
                {t("music.action.shuffle")}
              </Button>
              <Button
                size="sm"
                variant="ghost"
                icon={<Trash2 className="h-4 w-4" />}
                onClick={() => {
                  if (window.confirm(t("music.downloads.removeAllConfirm", { count: tracks.length }))) void clearDownloads();
                }}
              >
                {t("music.downloads.removeAll")}
              </Button>
            </div>
          </div>
          <div>
            {tracks.map((track, i) => {
              const active = current?.kind === "music" && current.trackId === track.id;
              return (
                <MediaRow
                  key={track.id}
                  kind="music"
                  index={i + 1}
                  title={track.title}
                  subtitle={[track.artist, track.album].filter(Boolean).join(" — ") || null}
                  cover={track.cover_url}
                  active={active}
                  playing={active && playing}
                  trailing={track.duration_secs ? formatDuration(track.duration_secs) : undefined}
                  onClick={() => void playTracks(tracks, i)}
                  onPlay={() => void playTracks(tracks, i)}
                  menu={
                    <MoreMenuTrigger>
                      <MenuItem icon={<Play />} onSelect={() => void playTracks(tracks, i)}>
                        {t("common.action.play")}
                      </MenuItem>
                      <MenuItem icon={<Trash2 />} onSelect={() => void removeDownload(track.id)}>
                        {t("music.downloads.remove")}
                      </MenuItem>
                    </MoreMenuTrigger>
                  }
                />
              );
            })}
          </div>
        </>
      )}
    </div>
  );
}
