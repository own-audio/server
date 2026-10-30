// SPDX-License-Identifier: AGPL-3.0-or-later
import type { MusicTrack } from "../../api/types";

export interface ArtistAlbum {
  key: string;
  title: string;
  count: number;
  cover: string | null;
  /** The artist's own album, as opposed to one they only appear on (a
   *  compilation). The artist list's `album_count` counts only these. */
  own: boolean;
}

const sameName = (a: string, b: string) => a.localeCompare(b, undefined, { sensitivity: "accent" }) === 0;

/** Whether `artist` is among a song's album artists ("A; B" names two). */
function albumIsBy(tr: MusicTrack, artist: string): boolean {
  const line = tr.album_artist ?? tr.artist ?? "";
  return line.split(/\s*;\s*/).some((name) => sameName(name, artist));
}

/** An album's key within an artist's songs: its id, or its name for a song without one. */
export const artistAlbumKey = (tr: MusicTrack) => tr.album_id ?? `name:${tr.album ?? ""}`;

/** The albums an artist's songs are on, each with how many of the songs it
 *  holds: the artist's own first, then the ones they appear on. Without an
 *  `artist` every album counts as their own. */
export function albumsOf(tracks: MusicTrack[], unknown: string, artist?: string): ArtistAlbum[] {
  const byKey = new Map<string, ArtistAlbum>();
  for (const tr of tracks) {
    const key = artistAlbumKey(tr);
    const album = byKey.get(key) ?? { key, title: tr.album || unknown, count: 0, cover: null, own: !artist };
    album.count += 1;
    album.own ||= !!artist && albumIsBy(tr, artist);
    album.cover ??= tr.cover_url ?? null;
    byKey.set(key, album);
  }
  return [...byKey.values()].sort((a, b) => Number(b.own) - Number(a.own) || a.title.localeCompare(b.title));
}
