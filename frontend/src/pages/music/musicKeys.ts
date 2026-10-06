// SPDX-License-Identifier: AGPL-3.0-or-later
/** Every query key that a music mutation can invalidate. Its own module so the
 *  component files stay component-only (react-refresh).
 *
 *  "playlist-tracks" has to be here too: a playlist view reads that key, not
 *  "music-tracks", so identifying/editing a track from inside a playlist used
 *  to leave the playlist showing the pre-identify metadata until the user
 *  reloaded or navigated away and back. `invalidateQueries` matches by key
 *  prefix, so listing it once here invalidates every `["playlist-tracks", id]`
 *  variant regardless of which playlist is open. */
export const MUSIC_KEYS = [
  "music-tracks",
  "music-artists",
  "music-albums",
  "music-genres",
  "playlist-tracks",
];
