// SPDX-License-Identifier: AGPL-3.0-or-later
/* An album link carries its name and, when known, the album's id. Two albums
   can share a name and an artist (two editions, or a folder whose files are
   tagged differently), and only the id tells them apart; links made from a
   song (home, player) have the name alone. U+241F (symbol for unit separator)
   can't occur in a tag value, which a plain "-" or ":" easily can. */
const SEP = "\u241F";

export const albumKey = (artist: string, album: string, id?: string | null) =>
  id ? `${artist}${SEP}${album}${SEP}${id}` : `${artist}${SEP}${album}`;

export function parseAlbumKey(key: string): { artist: string; album: string; id?: string } {
  const [artist, album, id] = key.split(SEP);
  return { artist: artist ?? "", album: album ?? "", ...(id ? { id } : {}) };
}
