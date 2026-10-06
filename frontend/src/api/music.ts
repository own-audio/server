// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import { batchHeaders } from "./trash";
import type { MusicTrack, MusicPlaylist, MusicPlaylistTrack, ArtistSummary, AlbumSummary, GenreSummary, Visibility } from "./types";

// ── Tracks ─────────────────────────────────────────────────────────────────

export async function listTracks(): Promise<MusicTrack[]> {
  const { data } = await api.get<MusicTrack[]>("/music/tracks");
  return data;
}

export async function getTrack(id: string): Promise<MusicTrack> {
  const { data } = await api.get<MusicTrack>(`/music/tracks/${id}`);
  return data;
}

export interface UpdateTrackRequest {
  title: string;
  artist?: string;
  album?: string;
  genre?: string;
  track_number?: number;
}

export async function updateTrack(
  id: string,
  req: UpdateTrackRequest
): Promise<MusicTrack> {
  const { data } = await api.put<MusicTrack>(`/music/tracks/${id}`, req);
  return data;
}

export async function setTrackVisibility(id: string, visibility: Visibility): Promise<void> {
  await api.put(`/music/tracks/${id}/visibility`, { visibility });
}

/** Moves the track to the trash; `batch` groups one gesture — see `api/trash.ts`. */
export async function deleteTrack(id: string, batch?: string): Promise<void> {
  await api.delete(`/music/tracks/${id}`, batchHeaders(batch));
}

export interface UploadTrackRequest {
  title?: string;
  artist?: string;
  album?: string;
  genre?: string;
  trackNumber?: number;
  durationSecs?: number;
  file: File;
  cover?: File | null;
}

export async function uploadTrack(req: UploadTrackRequest): Promise<MusicTrack> {
  const formData = new FormData();
  if (req.title?.trim()) formData.append("title", req.title.trim());
  if (req.artist?.trim()) formData.append("artist", req.artist.trim());
  if (req.album?.trim()) formData.append("album", req.album.trim());
  if (req.genre?.trim()) formData.append("genre", req.genre.trim());
  if (req.trackNumber != null) formData.append("track_number", String(req.trackNumber));
  if (req.durationSecs != null) formData.append("duration_secs", String(req.durationSecs));
  formData.append("file", req.file, req.file.name);
  if (req.cover) formData.append("cover", req.cover, req.cover.name);

  const { data } = await api.post<MusicTrack>("/music/tracks/upload", formData);
  return data;
}

export async function uploadTrackCover(trackId: string, file: File): Promise<void> {
  const formData = new FormData();
  formData.append("cover", file, file.name);
  await api.post(`/music/tracks/${trackId}/upload-cover`, formData);
}

export async function getTrackStreamUrl(trackId: string): Promise<string> {
  const { data } = await api.get<{ url: string }>(`/music/tracks/${trackId}/stream`);
  return data.url;
}

// ── Grouped browsing ───────────────────────────────────────────────────────

/** Ids of the songs you starred (the heart). */
export async function listStarred(): Promise<string[]> {
  const { data } = await api.get<string[]>("/music/starred");
  return data;
}

export async function starTrack(trackId: string): Promise<void> {
  await api.put(`/music/tracks/${trackId}/star`);
}

export async function unstarTrack(trackId: string): Promise<void> {
  await api.delete(`/music/tracks/${trackId}/star`);
}

/** The artist's photo: fetched by the server from Wikimedia on first request and cached; 404 when none is known. */
export function artistImageUrl(artist: string): string {
  return `/api/v1/music/artists/image?artist=${encodeURIComponent(artist)}`;
}

export async function listArtists(): Promise<ArtistSummary[]> {
  const { data } = await api.get<ArtistSummary[]>("/music/artists");
  return data;
}

export async function listAlbums(artist?: string): Promise<AlbumSummary[]> {
  const { data } = await api.get<AlbumSummary[]>("/music/albums", { params: artist ? { artist } : undefined });
  return data;
}

export async function listGenres(): Promise<GenreSummary[]> {
  const { data } = await api.get<GenreSummary[]>("/music/genres");
  return data;
}

// ── Track progress ─────────────────────────────────────────────────────────

export interface TrackProgressResponse {
  track_id: string;
  position_secs: number;
  completed: boolean;
  updated_at: string;
}

export async function getTrackProgress(
  trackId: string
): Promise<TrackProgressResponse | null> {
  try {
    const res = await api.get<TrackProgressResponse>(
      `/music/tracks/${trackId}/progress`
    );
    return res.data;
  } catch (err: unknown) {
    if ((err as { response?: { status?: number } })?.response?.status === 404) {
      return null;
    }
    return null;
  }
}

export async function saveTrackProgress(
  trackId: string,
  positionSecs: number,
  completed: boolean
): Promise<void> {
  await api.put(`/music/tracks/${trackId}/progress`, {
    position_secs: positionSecs,
    completed,
    device_kind: "web",
  });
}

// ── Playlists ──────────────────────────────────────────────────────────────

export async function listPlaylists(): Promise<MusicPlaylist[]> {
  const { data } = await api.get<MusicPlaylist[]>("/music/playlists");
  return data;
}

export async function getPlaylist(id: string): Promise<MusicPlaylist> {
  const { data } = await api.get<MusicPlaylist>(`/music/playlists/${id}`);
  return data;
}

export interface CreatePlaylistRequest {
  name: string;
  description?: string;
  /** Made by the smart-playlist generator; marks it so the Smart view can list it. */
  generated?: boolean;
  /** Fills it in the same request, in this order; ids you may not play are skipped. */
  track_ids?: string[];
}

export interface PlaylistAudienceEntry {
  user_id: string;
  display_name: string;
  display_label: string | null;
  can_listen: boolean;
  /** A family admin: sees whatever is shared with the family. */
  locked: boolean;
}

/** Who in the family can play your playlist now (everyone but you). */
export async function playlistAudience(id: string): Promise<PlaylistAudienceEntry[]> {
  const { data } = await api.get<PlaylistAudienceEntry[]>(`/music/playlists/${id}/audience`);
  return data;
}

/**
 * Share with chosen members: `live` (they play yours and see changes; nobody chosen makes it
 * private again) or `copy` (each gets their own). Your private songs in it are shared with them.
 */
export async function sharePlaylist(
  id: string,
  mode: "live" | "copy",
  userIds: string[]
): Promise<{ shared_tracks: number; copies: number }> {
  const { data } = await api.put<{ shared_tracks: number; copies: number }>(`/music/playlists/${id}/share`, { mode, user_ids: userIds });
  return data;
}

/** Keep a generated playlist (sets `kept_at`). */
export async function keepPlaylist(id: string): Promise<void> {
  await api.put(`/music/playlists/${id}/keep`);
}

export async function createPlaylist(
  req: CreatePlaylistRequest
): Promise<MusicPlaylist> {
  const { data } = await api.post<MusicPlaylist>("/music/playlists", req);
  return data;
}

export async function updatePlaylist(
  id: string,
  req: CreatePlaylistRequest
): Promise<MusicPlaylist> {
  const { data } = await api.put<MusicPlaylist>(`/music/playlists/${id}`, req);
  return data;
}

export async function setPlaylistVisibility(id: string, visibility: Visibility): Promise<void> {
  await api.put(`/music/playlists/${id}/visibility`, { visibility });
}

export async function uploadPlaylistCover(playlistId: string, file: File): Promise<void> {
  const formData = new FormData();
  formData.append("cover", file, file.name);
  await api.post(`/music/playlists/${playlistId}/upload-cover`, formData);
}

export async function deletePlaylist(id: string, batch?: string): Promise<void> {
  await api.delete(`/music/playlists/${id}`, batchHeaders(batch));
}

export async function listPlaylistTracks(
  playlistId: string
): Promise<MusicPlaylistTrack[]> {
  const { data } = await api.get<MusicPlaylistTrack[]>(
    `/music/playlists/${playlistId}/tracks`
  );
  return data;
}

export async function addTrackToPlaylist(
  playlistId: string,
  trackId: string
): Promise<void> {
  await api.post(`/music/playlists/${playlistId}/tracks`, {
    track_id: trackId,
  });
}

export async function removeTrackFromPlaylist(
  playlistId: string,
  entryId: string
): Promise<void> {
  await api.delete(`/music/playlists/${playlistId}/tracks/${entryId}`);
}

export async function reorderPlaylistTracks(
  playlistId: string,
  entryIds: string[]
): Promise<void> {
  await api.put(`/music/playlists/${playlistId}/tracks/reorder`, {
    entry_ids: entryIds,
  });
}

// ── MusicBrainz identification ────────────────────────────────────────────

export interface MetadataCandidate {
  mb_recording_id: string;
  title: string;
  artist: string | null;
  mb_artist_id: string | null;
  album: string | null;
  mb_release_id: string | null;
  duration_ms: number | null;
  genre: string | null;
  track_number: number | null;
  year: number | null;
  /** Speculative: built from the release id and never checked, so it may 404.
   *  A broken image here is normal, not an error. */
  cover_art_url: string | null;
  score: number;
}

/** The backend holds the MusicBrainz credentials and rate-limits process-wide,
 *  so no client needs its own throttling. At least one field is required. */
export async function searchTrackMetadata(
  trackId: string,
  req: { title?: string; artist?: string; album?: string; limit?: number }
): Promise<MetadataCandidate[]> {
  const { data } = await api.post<MetadataCandidate[]>(`/music/tracks/${trackId}/metadata/search`, req);
  return data;
}

export interface AlbumTrackMatch {
  track_id: string;
  mb_recording_id: string;
  title: string;
  disc: number;
  position: number;
}

/** One album a group of tracks could be. Editions of the same album are
 *  folded into one; `mb_release_id` is the edition that fits best. */
export interface AlbumCandidate {
  mb_release_id: string;
  mb_release_group_id: string;
  title: string;
  artist: string | null;
  year: number | null;
  /** Album, EP, Single, Broadcast, Other. */
  primary_type: string | null;
  /** Live, Compilation, Soundtrack, … — empty for a plain studio album. */
  secondary_types: string[];
  /** Official, Promotion, Bootleg, Pseudo-Release. */
  status: string | null;
  track_count: number;
  /** How many of the requested tracks are on it. */
  matched: number;
  editions: number;
  cover_art_url: string | null;
  tracks: AlbumTrackMatch[];
}

/** Which album a group of tracks is, best first. Empty when the tracks have
 *  no artist to go on — match them one by one instead. */
export async function identifyAlbum(trackIds: string[]): Promise<AlbumCandidate[]> {
  const { data } = await api.post<AlbumCandidate[]>("/music/albums/identify", { track_ids: trackIds });
  return data;
}

/** Only the ids go over the wire — the server re-fetches the recording rather
 *  than trusting client-cached search results. */
export async function applyTrackMetadata(
  trackId: string,
  req: { mb_recording_id: string; mb_release_id?: string; fetch_cover?: boolean }
): Promise<MusicTrack> {
  const { data } = await api.post<MusicTrack>(`/music/tracks/${trackId}/metadata/apply`, req);
  return data;
}

// ── Lyrics ────────────────────────────────────────────────────────────────

/** From the file's embedded tag. `null` means the file carries none. */
export async function getTrackLyrics(trackId: string): Promise<string | null> {
  const { data } = await api.get<{ lyrics: string | null }>(`/music/tracks/${trackId}/lyrics`);
  return data.lyrics;
}

export async function setTrackLyrics(trackId: string, lyrics: string | null): Promise<void> {
  await api.put(`/music/tracks/${trackId}/lyrics`, { lyrics: lyrics && lyrics.trim() ? lyrics : null });
}

// ── Duplicates ────────────────────────────────────────────────────────────

export interface DuplicateGroup {
  /** Always `identical` today — byte-identical files, matched by checksum.
   *  A "likely the same recording" tier is planned but not built. */
  tier: string;
  tracks: MusicTrack[];
}

/** Scoped to the caller's own tracks; a family member's copy never appears.
 *  The response is an envelope — `{ groups: [...] }`, not a bare array. */
export async function listDuplicates(): Promise<DuplicateGroup[]> {
  const { data } = await api.get<{ groups: DuplicateGroup[] }>("/music/duplicates");
  return data.groups ?? [];
}

/** What the audio file's own tags say — read fresh from storage, written
 *  nowhere. Diverges from the library record after a MusicBrainz match, which
 *  updates the database and leaves the file alone. */
export interface FileTags {
  title: string | null;
  artist: string | null;
  album: string | null;
  genre: string | null;
  track_number: number | null;
  has_cover: boolean;
  /** The uploaded file's own name — often the clearest clue when both the
   *  library values and the embedded tags are wrong. */
  file_name: string | null;
}

export async function getTrackFileTags(id: string): Promise<FileTags> {
  const { data } = await api.get<FileTags>(`/music/tracks/${id}/metadata/file-tags`);
  return data;
}

// ── Track feedback ─────────────────────────────────────────────────────────
//
// Two meanings, deliberately not one control: a dislike recovers over months,
// a ban does not until undone. Neither removes anything — the track stays in
// the family library and is untouched for everyone else. "Not for me" is a
// preference, not a permission, so it gets no lock icon and no deletion.

export type FeedbackKind = "dislike" | "banned";

export interface TrackFeedback {
  track_id: string;
  kind: FeedbackKind;
  updated_at: string;
}

export async function setTrackFeedback(trackId: string, kind: FeedbackKind): Promise<void> {
  await api.put(`/music/tracks/${trackId}/feedback`, { kind });
}

/** Idempotent: clearing feedback that was never set is success. */
export async function clearTrackFeedback(trackId: string): Promise<void> {
  await api.delete(`/music/tracks/${trackId}/feedback`);
}

/**
 * The undo list. Worth rendering somewhere in settings: nothing about a banned
 * track looks different in the library, so without this a mis-click is
 * unrecoverable.
 */
export async function listTrackFeedback(kind?: FeedbackKind): Promise<TrackFeedback[]> {
  const { data } = await api.get<TrackFeedback[]>("/music/feedback", {
    params: kind ? { kind } : undefined,
  });
  return data;
}

// ── Smart playlists ────────────────────────────────────────────────────────
//
// A saved query, not a list of tracks. The rule schema is closed: the server
// rejects anything it does not recognise, which is what makes a rule generated
// from a sentence safe to run.

export type PlaylistArc = "none" | "warmup-sustain-cooldown" | "build" | "unwind";
export type PlaylistSort = "weighted-random" | "weight" | "least-recently-played" | "random";

export interface RuleStringSet {
  include?: string[];
  exclude?: string[];
}

export interface RuleFilters {
  bpm?: [number, number];
  energy?: [number, number];
  genres?: RuleStringSet;
  artists?: RuleStringSet;
  starred?: boolean;
  min_rating?: number;
  not_played_days?: number;
  added_days?: number;
  played_by_others_not_me?: boolean;
}

export type RuleLimit = { kind: "count"; n: number } | { kind: "duration"; minutes: number };

export interface PlaylistRule {
  filters?: RuleFilters;
  limit?: RuleLimit;
  arc?: PlaylistArc;
  sequence?: { max_per_artist: number; no_consecutive_artist: boolean };
  sort?: PlaylistSort;
}

export interface SmartPlaylist {
  id: string;
  name: string;
  description: string | null;
  rule: PlaylistRule;
  mode: "dynamic" | "frozen";
  shared: boolean;
  owned: boolean;
  created_at: string;
  updated_at: string;
}

export interface SmartPlaylistPreset {
  slug: string;
  name: string;
  rule: PlaylistRule;
}

export interface ResolvedPlaylist {
  tracks: MusicTrack[];
  total_secs: number;
  /**
   * How many tracks matched before sequencing. Show it when a playlist comes
   * back short: the honest limit of this feature is the library, not the
   * algorithm, and a narrow rule over a half-measured library is not a bug.
   */
  candidates: number;
}

export async function listSmartPlaylistPresets(): Promise<SmartPlaylistPreset[]> {
  const { data } = await api.get<SmartPlaylistPreset[]>("/music/smart-playlists/presets");
  return data;
}

export async function listSmartPlaylists(): Promise<SmartPlaylist[]> {
  const { data } = await api.get<SmartPlaylist[]>("/music/smart-playlists");
  return data;
}

export async function createSmartPlaylist(req: {
  name: string;
  description?: string;
  rule: PlaylistRule;
  shared?: boolean;
}): Promise<SmartPlaylist> {
  const { data } = await api.post<SmartPlaylist>("/music/smart-playlists", req);
  return data;
}

export async function deleteSmartPlaylist(id: string): Promise<void> {
  await api.delete(`/music/smart-playlists/${id}`);
}

/**
 * Resolved against **the caller's** history, not the owner's. That is what
 * sharing a rule means, and a shared playlist showing different tracks to two
 * people is the feature rather than a bug — say so in the UI.
 */
export async function resolveSmartPlaylist(id: string): Promise<ResolvedPlaylist> {
  const { data } = await api.post<ResolvedPlaylist>(`/music/smart-playlists/${id}/resolve`, {});
  return data;
}

/** Run a rule without saving it — what a natural-language request lands on. */
export async function resolveRule(rule: PlaylistRule): Promise<ResolvedPlaylist> {
  const { data } = await api.post<ResolvedPlaylist>("/music/smart-playlists/resolve", { rule });
  return data;
}

/** Materialise a rule into an ordinary playlist that stops re-evaluating. */
export async function freezeSmartPlaylist(
  id: string,
  req: { name?: string; shared?: boolean; mark_source?: boolean } = {}
): Promise<MusicPlaylist> {
  const { data } = await api.post<MusicPlaylist>(`/music/smart-playlists/${id}/freeze`, req);
  return data;
}

// ── Natural language ───────────────────────────────────────────────────────

/**
 * Text in, **rule out** — never tracks. The caller resolves it, so the result
 * is inspectable and nothing is played until asked.
 *
 * What leaves the browser is the sentence and the library's genre names. Never
 * track titles, never listening history.
 */
export async function parseIntent(text: string): Promise<PlaylistRule> {
  const { data } = await api.post<{ rule: PlaylistRule }>("/music/intent", { text });
  return data.rule;
}
