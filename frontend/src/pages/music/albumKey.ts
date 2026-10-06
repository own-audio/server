// SPDX-License-Identifier: AGPL-3.0-or-later
/* Albums have no id of their own — they are (artist, album) pairs — so a URL
   needs a composite key. U+241F (symbol for unit separator) can't occur in a
   tag value, which a plain "-" or ":" easily can. */
const SEP = "\u241F";

export const albumKey = (artist: string, album: string) => `${artist}${SEP}${album}`;

export function parseAlbumKey(key: string): { artist: string; album: string } {
  const [artist, album] = key.split(SEP);
  return { artist: artist ?? "", album: album ?? "" };
}
