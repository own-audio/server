// SPDX-License-Identifier: AGPL-3.0-or-later
import type { MusicTrack } from "../../api/types";

export interface ArtistAlbum {
  key: string;
  title: string;
  count: number;
  cover: string | null;
}

/** An album's key within an artist's songs: its id, or its name for a song without one. */
export const artistAlbumKey = (tr: MusicTrack) => tr.album_id ?? `name:${tr.album ?? ""}`;

/** The albums an artist's songs are on, by name, each with how many of the songs it holds. */
export function albumsOf(tracks: MusicTrack[], unknown: string): ArtistAlbum[] {
  const byKey = new Map<string, ArtistAlbum>();
  for (const tr of tracks) {
    const key = artistAlbumKey(tr);
    const album = byKey.get(key) ?? { key, title: tr.album || unknown, count: 0, cover: null };
    album.count += 1;
    album.cover ??= tr.cover_url ?? null;
    byKey.set(key, album);
  }
  return [...byKey.values()].sort((a, b) => a.title.localeCompare(b.title));
}
