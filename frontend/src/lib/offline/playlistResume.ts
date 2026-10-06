// SPDX-License-Identifier: AGPL-3.0-or-later
/*
 * Where a playlist's Home Screen player was left: the queue as it was playing
 * (shuffled or not), the song, and the position in it. Kept in this browser
 * only — each icon is its own device, and this is about picking up on it.
 */

export interface PlaylistResume {
  queue: string[];
  trackId: string;
  position: number;
  updatedAt: number;
}

const key = (playlistId: string) => `own-audio-resume:${playlistId}`;

export function loadResume(playlistId: string): PlaylistResume | null {
  try {
    const raw = localStorage.getItem(key(playlistId));
    return raw ? (JSON.parse(raw) as PlaylistResume) : null;
  } catch {
    return null;
  }
}

export function saveResume(playlistId: string, resume: PlaylistResume): void {
  try {
    localStorage.setItem(key(playlistId), JSON.stringify(resume));
  } catch {
    // Storage refused (private mode, full): resuming is a convenience.
  }
}
