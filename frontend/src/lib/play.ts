// SPDX-License-Identifier: AGPL-3.0-or-later
import { getBookFileStreamUrl, listBookChapters, listBookFiles } from "../api/audiobooks";
import { getBookProgress } from "../api/playback";
import { getTrackStreamUrl } from "../api/music";
import { offlineStreamUrl } from "./offline/downloads";
import { downloadEpisode, getStreamUrl } from "../api/podcasts";
import { usePlayerStore, type MusicPlayerTrack, type QueueSource } from "../store/playerStore";
import type { AudioBook, MusicTrack, PodcastEpisode, PodcastFeed } from "../api/types";
import { t as translate } from "../i18n";

/* One place that turns library items into player queues. Stream URLs are
   resolved at play time and never cached — they expire in ~4 h. */

export function isCurrentBook(bookId: string) {
  const t = usePlayerStore.getState().track;
  return t?.kind === "audiobook" && t.bookId === bookId;
}

/** `startAt` is a position *within the starting file*, for a handoff where the
 *  other device already told us where it was; otherwise the server's own saved
 *  progress decides. */
export async function playBook(book: AudioBook, startFileId?: string, startAt?: number) {
  const store = usePlayerStore.getState();
  const current = store.track;
  if (current?.kind === "audiobook" && current.bookId === book.id && (!startFileId || current.fileId === startFileId)) {
    store.setPlaying(!store.playing);
    return;
  }
  const [files, progress, chapters] = await Promise.all([
    listBookFiles(book.id),
    getBookProgress(book.id),
    // A book with no chapter data is the normal case, not a failure.
    listBookChapters(book.id).catch(() => []),
  ]);
  if (files.length === 0) return;
  const urls = await Promise.all(files.map((f) => getBookFileStreamUrl(book.id, f.id)));

  let offset = 0;
  const queue = files.map((file, i) => {
    const fileOffset = offset;
    offset += file.duration_secs ?? 0;
    const isResumeFile = progress?.file_id === file.id;
    return {
      kind: "audiobook" as const,
      bookId: book.id,
      fileId: file.id,
      filePosition: file.position,
      title: file.title ?? translate("player.partTitle", { n: file.position }),
      bookTitle: book.title,
      imageUrl: book.cover_url ?? null,
      streamUrl: urls[i],
      durationSecs: file.duration_secs,
      resumePosition: isResumeFile && progress ? Math.max(0, progress.position_secs - fileOffset) : 0,
      bookPositionOffsetSecs: fileOffset,
      bookTotalDurationSecs: book.total_duration_secs,
    };
  });

  let start = 0;
  const wanted = startFileId ?? progress?.file_id;
  if (wanted) start = Math.max(0, files.findIndex((f) => f.id === wanted));
  if (startAt != null && queue[start]) queue[start] = { ...queue[start], resumePosition: Math.max(0, startAt) };
  store.setChapters(chapters);
  store.playQueue(queue, start);
}

export async function toPlayerTrack(t: MusicTrack): Promise<MusicPlayerTrack> {
  return {
    kind: "music",
    trackId: t.id,
    title: t.title,
    artist: t.artist,
    album: t.album,
    albumArtist: t.album_artist ?? t.artist,
    imageUrl: t.cover_url,
    // A downloaded song never asks the server for a link, which is what lets
    // a queue start with no connection at all.
    streamUrl: (await offlineStreamUrl(t.id)) ?? (await getTrackStreamUrl(t.id)),
    durationSecs: t.duration_secs,
    resumePosition: 0,
  };
}

export function isCurrentTrack(trackId: string) {
  const t = usePlayerStore.getState().track;
  return t?.kind === "music" && t.trackId === trackId;
}

export async function playTracks(tracks: MusicTrack[], index = 0, source: QueueSource | null = null) {
  if (tracks.length === 0) return;
  const store = usePlayerStore.getState();
  if (isCurrentTrack(tracks[index].id)) {
    store.setPlaying(!store.playing);
    return;
  }
  const queue = await Promise.all(tracks.map(toPlayerTrack));
  store.setChapters([]);
  store.playQueue(queue, index, source);
}

/** Start a list at a given song and position — picking up where it was left. */
export async function resumeTracks(tracks: MusicTrack[], index: number, position: number) {
  if (!tracks[index]) return;
  const queue = await Promise.all(tracks.map(toPlayerTrack));
  queue[index] = { ...queue[index], resumePosition: Math.max(0, position) };
  const store = usePlayerStore.getState();
  store.setChapters([]);
  store.playQueue(queue, index);
}

export async function shuffleTracks(tracks: MusicTrack[], source: QueueSource | null = null) {
  const shuffled = [...tracks];
  for (let i = shuffled.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [shuffled[i], shuffled[j]] = [shuffled[j], shuffled[i]];
  }
  const queue = await Promise.all(shuffled.map(toPlayerTrack));
  usePlayerStore.getState().setChapters([]);
  usePlayerStore.getState().playQueue(queue, 0, source);
}

export async function queueTrack(t: MusicTrack) {
  usePlayerStore.getState().addToQueue(await toPlayerTrack(t));
}

export function isCurrentEpisode(episodeId: string) {
  const t = usePlayerStore.getState().track;
  return t?.kind === "podcast" && t.epId === episodeId;
}

/** Episodes need a server-side copy before they can stream (client guide §12). */
export async function playEpisode(
  feedId: string,
  ep: PodcastEpisode,
  feed?: PodcastFeed | null,
  startAt?: number
): Promise<PodcastEpisode> {
  const store = usePlayerStore.getState();
  if (isCurrentEpisode(ep.id)) {
    store.setPlaying(!store.playing);
    return ep;
  }
  const ready = ep.has_local ? ep : await downloadEpisode(feedId, ep.id);
  const url = await getStreamUrl(feedId, ready.id);
  store.setChapters([]);
  store.play({
    kind: "podcast",
    epId: ep.id,
    feedId,
    title: ep.title,
    feedTitle: feed?.title ?? null,
    imageUrl: ep.image_url ?? feed?.image_url ?? null,
    streamUrl: url,
    durationSecs: ep.duration_secs,
    resumePosition: startAt ?? ep.progress_secs ?? 0,
    description: ep.description ?? null,
  });
  return ready;
}
