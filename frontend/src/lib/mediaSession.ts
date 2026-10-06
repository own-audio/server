// SPDX-License-Identifier: AGPL-3.0-or-later
import type { PlayerTrack } from "../store/playerStore";
import { useAuthStore } from "../store/authStore";
import { mediaUrl } from "../api/client";
import { offlineCoverUrl } from "./offline/downloads";

/**
 * OS media keys, the lock screen, and the browser's own media widget, via the
 * Media Session API.
 *
 * Artwork is the awkward part: audiobook and music covers need the bearer
 * token, and the browser fetches `artwork` URLs itself with no headers. So the
 * image is fetched here and handed over as a blob URL. Podcast art is public
 * and can be passed straight through.
 */

let currentArtworkUrl: string | null = null;

function revokeArtwork() {
  if (currentArtworkUrl) {
    URL.revokeObjectURL(currentArtworkUrl);
    currentArtworkUrl = null;
  }
}

async function artworkFor(track: PlayerTrack): Promise<MediaImage[]> {
  if (!track.imageUrl) return [];
  // Podcast art is served publicly; no fetch dance needed.
  if (track.kind === "podcast") return [{ src: mediaUrl(track.imageUrl) as string }];

  // Kept on the device for offline listening: no request needed.
  const stored = await offlineCoverUrl(track.imageUrl);
  if (stored) {
    revokeArtwork();
    currentArtworkUrl = stored;
    return [{ src: stored }];
  }
  const token = useAuthStore.getState().token;
  try {
    if (!token) throw new Error("signed out");
    const res = await fetch(mediaUrl(track.imageUrl) as string, { headers: { Authorization: `Bearer ${token}` } });
    if (!res.ok) throw new Error(`HTTP ${res.status}`);
    if (!(res.headers.get("content-type") ?? "").startsWith("image/")) throw new Error("not an image");
    const blob = await res.blob();
    revokeArtwork();
    currentArtworkUrl = URL.createObjectURL(blob);
    return [{ src: currentArtworkUrl, type: blob.type }];
  } catch {
    return [];
  }
}

export interface MediaSessionHandlers {
  play: () => void;
  pause: () => void;
  previous: () => void;
  next: () => void;
  seekTo: (seconds: number) => void;
  seekBy: (delta: number) => void;
}

export function isMediaSessionSupported(): boolean {
  return typeof navigator !== "undefined" && "mediaSession" in navigator;
}

export async function setMediaSessionMetadata(track: PlayerTrack | null): Promise<void> {
  if (!isMediaSessionSupported()) return;
  if (!track) {
    navigator.mediaSession.metadata = null;
    revokeArtwork();
    return;
  }

  const artist =
    track.kind === "podcast" ? (track.feedTitle ?? "") : track.kind === "music" ? (track.artist ?? "") : track.bookTitle;
  const album = track.kind === "music" ? "" : track.kind === "audiobook" ? track.bookTitle : "";

  navigator.mediaSession.metadata = new MediaMetadata({
    title: track.title,
    artist,
    album,
    artwork: await artworkFor(track),
  });
}

/**
 * `seekButtons` decides what the lock screen offers, because it can't offer
 * both: iOS (and Chrome's compact notification) shows ±skip buttons instead of
 * previous/next whenever seek handlers are registered. Music wants the tracks;
 * spoken word wants the skips.
 */
export function setMediaSessionHandlers(h: MediaSessionHandlers, { seekButtons }: { seekButtons: boolean }): void {
  if (!isMediaSessionSupported()) return;
  const ms = navigator.mediaSession;
  ms.setActionHandler("play", () => h.play());
  ms.setActionHandler("pause", () => h.pause());
  ms.setActionHandler("previoustrack", () => h.previous());
  ms.setActionHandler("nexttrack", () => h.next());
  // Not every browser implements every action; a rejected one is not an error.
  try {
    ms.setActionHandler("seekbackward", seekButtons ? (d) => h.seekBy(-(d.seekOffset ?? 10)) : null);
    ms.setActionHandler("seekforward", seekButtons ? (d) => h.seekBy(d.seekOffset ?? 10) : null);
    ms.setActionHandler("seekto", (d) => {
      if (typeof d.seekTime === "number") h.seekTo(d.seekTime);
    });
  } catch {
    // ignore
  }
}

export function setMediaSessionPlaybackState(playing: boolean): void {
  if (!isMediaSessionSupported()) return;
  navigator.mediaSession.playbackState = playing ? "playing" : "paused";
}

export function setMediaSessionPosition(position: number, duration: number, rate: number): void {
  if (!isMediaSessionSupported() || !("setPositionState" in navigator.mediaSession)) return;
  if (!isFinite(duration) || duration <= 0 || position > duration) return;
  try {
    navigator.mediaSession.setPositionState({ duration, position: Math.max(0, position), playbackRate: rate });
  } catch {
    // Firefox throws on some rate/duration combinations rather than ignoring them.
  }
}

export function clearMediaSession(): void {
  if (!isMediaSessionSupported()) return;
  navigator.mediaSession.metadata = null;
  navigator.mediaSession.playbackState = "none";
  revokeArtwork();
}
