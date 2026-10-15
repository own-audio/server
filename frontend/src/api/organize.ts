// SPDX-License-Identifier: AGPL-3.0-or-later
// The metadata organizer (revision 15, docs/metadata-organizer-plan.md): your
// songs as a table, saves as batches of the edit history, undo, and MusicBrainz
// recordings and releases to compare and apply. On /api/v2.
import api, { apiBaseUrl } from "./client";

const v2 = { baseURL: apiBaseUrl.replace(/\/api\/v1\/?$/, "/api/v2") };

export interface OrganizeTrack {
  id: string;
  title: string;
  artist?: string | null;
  album?: string | null;
  album_artist?: string | null;
  genre?: string | null;
  track_number?: number | null;
  disc_number?: number | null;
  disc_total?: number | null;
  is_compilation: boolean;
  duration_secs?: number | null;
  album_id?: string | null;
  year?: string | null;
  musicbrainz_recording_id?: string | null;
  musicbrainz_release_id?: string | null;
  identity?: "identify" | "mbid_tag" | null;
  has_cover: boolean;
  folder_id?: string | null;
  path: string;
  content_type: string;
  size_bytes?: number | null;
  edited: boolean;
  updated_at: string;
}

export interface OrganizeFilter {
  q?: string;
  folder?: string;
  path?: string;
  artist?: string;
  album_id?: string;
  unidentified?: boolean;
  no_cover?: boolean;
  no_year?: boolean;
  no_genre?: boolean;
  edited?: boolean;
  duplicates?: boolean;
  sort?: string;
  desc?: boolean;
}

export interface OrganizePage {
  items: OrganizeTrack[];
  next?: string | null;
  total: number;
}

export async function listOrganize(filter: OrganizeFilter, cursor?: string | null): Promise<OrganizePage> {
  const params: Record<string, string> = { limit: "200" };
  for (const [k, v] of Object.entries(filter)) {
    if (v === undefined || v === "" || v === false) continue;
    params[k] = String(v);
  }
  if (cursor) params.cursor = cursor;
  const { data } = await api.get<OrganizePage>("/organize/tracks", { ...v2, params });
  return data;
}

export interface Facets {
  folders: { id?: string | null; path?: string | null; tracks: number }[];
  artists: { name: string; tracks: number }[];
  albums: { id: string; title: string; artist: string; tracks: number }[];
  counts: { all: number; unidentified: number; no_cover: number; no_year: number; no_genre: number; edited: number; duplicates: number };
}

export async function getFacets(): Promise<Facets> {
  const { data } = await api.get<Facets>("/organize/facets", v2);
  return data;
}

/** The fields a save may change; absent stays, `null` clears. */
export type TrackFields = Partial<Pick<OrganizeTrack, "title" | "artist" | "album" | "album_artist" | "genre" | "track_number" | "disc_number" | "is_compilation">>;
export const EDITABLE: (keyof TrackFields)[] = ["title", "artist", "album", "album_artist", "genre", "track_number", "disc_number"];

export interface Saved {
  updated: number;
  batch_id: string;
}

export async function saveTracks(tracks: ({ id: string } & TrackFields)[]): Promise<Saved> {
  const { data } = await api.patch<Saved>("/tracks", { tracks }, v2);
  return data;
}

// ── History ────────────────────────────────────────────────────────────────

export interface EditBatch {
  batch_id: string;
  user_id?: string | null;
  source: "manual" | "bulk" | "identify" | "run" | "revert" | "undo" | string;
  at: string;
  items: number;
  first_label?: string | null;
  undone: boolean;
}

export interface MetadataEdit {
  id: string;
  item_kind: "track" | "album" | "contributor";
  item_id: string;
  item_label?: string | null;
  source: string;
  batch_id: string;
  before: Record<string, unknown>;
  after: Record<string, unknown>;
  at: string;
  undone_at?: string | null;
}

export async function listBatches(cursor?: string | null): Promise<{ items: EditBatch[]; next?: string | null }> {
  const { data } = await api.get<{ items: EditBatch[]; next?: string | null }>("/edits/batches", { ...v2, params: { limit: 50, cursor: cursor ?? undefined } });
  return data;
}

export async function listEdits(params: { batch_id?: string; item_kind?: string; item_id?: string }): Promise<MetadataEdit[]> {
  const { data } = await api.get<{ items: MetadataEdit[] }>("/edits", { ...v2, params: { ...params, limit: 500 } });
  return data.items;
}

export async function undoBatch(id: string): Promise<{ items: number; batch_id: string }> {
  const { data } = await api.post<{ items: number; batch_id: string }>(`/edits/batches/${id}/undo`, {}, v2);
  return data;
}

export async function undoEdit(id: string): Promise<{ items: number; batch_id: string }> {
  const { data } = await api.post<{ items: number; batch_id: string }>(`/edits/${id}/undo`, {}, v2);
  return data;
}

// ── MusicBrainz ────────────────────────────────────────────────────────────

export interface RecordingCandidate {
  mb_recording_id: string;
  title: string;
  artist?: string | null;
  album?: string | null;
  mb_release_id?: string | null;
  duration_ms?: number | null;
  track_number?: number | null;
  year?: number | null;
  genre?: string | null;
  score: number;
}

export async function searchRecordings(trackId: string, words: { title?: string; artist?: string; album?: string }): Promise<RecordingCandidate[]> {
  const { data } = await api.post<{ items: RecordingCandidate[] }>(`/tracks/${trackId}/identity/search`, words, v2);
  return data.items;
}

export async function identifyTrack(trackId: string, recording: string, release?: string | null): Promise<void> {
  await api.put(`/tracks/${trackId}/identity`, { recording, release: release ?? null }, v2);
}

export interface ReleaseCandidate {
  mb_release_id: string;
  mb_release_group_id: string;
  title: string;
  artist?: string | null;
  year?: number | null;
  primary_type?: string | null;
  secondary_types: string[];
  status?: string | null;
  track_count: number;
  matched: number;
  cover_art_url?: string | null;
  tracks: { track_id: string; mb_recording_id: string; title: string; disc: number; position: number; duration_ms?: number | null }[];
}

export async function searchReleases(trackIds: string[], words: { artist?: string; album?: string }): Promise<ReleaseCandidate[]> {
  const { data } = await api.post<{ items: ReleaseCandidate[] }>("/organize/releases", { track_ids: trackIds, ...words }, v2);
  return data.items;
}

export async function applyRelease(req: {
  release: string;
  tracks: { id: string; recording: string; disc?: number | null }[];
  fetch_cover?: boolean;
  use_genre?: boolean;
}): Promise<Saved> {
  const { data } = await api.post<Saved>("/organize/apply-release", req, v2);
  return data;
}

/** Set an image as the cover of a song (v1), in `batch` of the history so
 *  several songs' cover is one change to undo. */
export async function uploadCover(trackId: string, file: File, batch: string): Promise<void> {
  const form = new FormData();
  form.append("cover", file);
  await api.post(`/music/tracks/${trackId}/upload-cover`, form, { params: { batch } });
}

// ── Artists ────────────────────────────────────────────────────────────────

export interface Contributor {
  id: string;
  name: string;
  sort_name?: string | null;
  kind: string;
  musicbrainz_artist_id?: string | null;
}

export async function findContributors(q: string): Promise<Contributor[]> {
  const { data } = await api.get<{ items: Contributor[] }>("/contributors", { ...v2, params: { q, limit: 20 } });
  return data.items;
}

/** Name, sort name and who they are on MusicBrainz (an id or a link; `null` forgets it). */
export async function updateContributor(id: string, patch: { name?: string; sort_name?: string | null; musicbrainz_artist?: string | null }): Promise<Contributor> {
  const { data } = await api.patch<Contributor>(`/contributors/${id}`, patch, v2);
  return data;
}

/** `id` goes; `into` keeps everything it had (a family admin's). Not undoable. */
export async function mergeContributor(id: string, into: string): Promise<Contributor> {
  const { data } = await api.post<Contributor>(`/contributors/${id}/merge`, { into }, v2);
  return data;
}
