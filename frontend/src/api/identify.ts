// SPDX-License-Identifier: AGPL-3.0-or-later
// Identify runs (`features.identify_library`, revision 10), on /api/v2.
import api, { apiBaseUrl } from "./client";
import { intlLocale } from "../i18n";

/** The v2 API beside v1: same origin, same session. */
const v2 = { baseURL: apiBaseUrl.replace(/\/api\/v1\/?$/, "/api/v2") };

export type RunStatus = "queued" | "running" | "waiting" | "ready" | "failed" | "cancelled";
export type ProposalClass = "sure" | "review" | "none";
export type Decision = "pending" | "accepted" | "rejected";
export type ScopeKind = "library" | "folder" | "album" | "playlist" | "artist" | "tracks";

export interface IdentifyRun {
  id: string;
  status: RunStatus;
  error?: string | null;
  scope: { kind: ScopeKind; id?: string | null; label?: string | null };
  tracks: number;
  groups_total: number;
  groups_done: number;
  counts: { sure: number; review: number; none: number; pending: number; accepted: number; rejected: number };
  created_at: string;
  started_at?: string | null;
  finished_at?: string | null;
}

export interface ProposedTrack {
  mb_recording_id: string;
  title: string;
  artist?: string | null;
  album?: string | null;
  album_artist?: string | null;
  mb_release_id?: string | null;
  mb_release_group_id?: string | null;
  track_number?: number | null;
  disc_number?: number | null;
  year?: number | null;
  length_ms?: number | null;
  score: number;
}

export interface Proposal {
  id: string;
  group: { id: string; kind: "album" | "track"; label: string };
  track: {
    id: string;
    title: string;
    artist?: string | null;
    album?: string | null;
    disc_number?: number | null;
    track_number?: number | null;
    duration_secs?: number | null;
  };
  class: ProposalClass;
  reasons: string[];
  proposal?: ProposedTrack | null;
  keep_album: boolean;
  decision: Decision;
  /** The owner picked this recording (revision 13). */
  chosen_by_owner?: boolean;
  /** Other recordings the server keeps for it; absent before revision 13. */
  alternatives?: number;
}

/** A MusicBrainz recording a search found, best first. */
export interface RecordingCandidate {
  mb_recording_id: string;
  title: string;
  artist?: string | null;
  album?: string | null;
  mb_release_id?: string | null;
  duration_ms?: number | null;
  year?: number | null;
  score: number;
}

export interface RecordingSearch {
  title?: string;
  artist?: string;
  album?: string;
}

export interface Edition {
  index: number;
  mb_release_id: string;
  mb_release_group_id?: string | null;
  title: string;
  artist?: string | null;
  year?: number | null;
  primary_type?: string | null;
  secondary_types: string[];
  status?: string | null;
  track_count: number;
  matched: number;
  cover_art_url?: string | null;
}

export interface IdentifyGroup {
  id: string;
  kind: "album" | "track";
  label: string;
  chosen?: number | null;
  editions: Edition[];
}

interface Page<T> {
  items: T[];
  next?: string;
}

export async function listRuns(): Promise<IdentifyRun[]> {
  const { data } = await api.get<Page<IdentifyRun>>("/identify/runs", v2);
  return data.items;
}

export async function createRun(scope: RunScope): Promise<IdentifyRun> {
  const { data } = await api.post<IdentifyRun>("/identify/runs", { scope }, v2);
  return data;
}

export type RunScope = { kind: ScopeKind; id?: string; label?: string };

/** What a run over the scope would look up and the most it could charge (revision 11). */
export async function quoteRun(scope: RunScope): Promise<{ tracks: number; charge_micro: number }> {
  const { data } = await api.post<{ tracks: number; charge_micro: number }>("/identify/quote", { scope }, v2);
  return data;
}

/** A price in micro-USD, to the tenth of a cent a track costs. */
export function formatCharge(micro: number): string {
  return new Intl.NumberFormat(intlLocale(), { style: "currency", currency: "USD", minimumFractionDigits: 2, maximumFractionDigits: 3 })
    .format(Math.ceil(micro / 1_000) / 1_000);
}

export async function getRun(id: string): Promise<IdentifyRun> {
  const { data } = await api.get<IdentifyRun>(`/identify/runs/${id}`, v2);
  return data;
}

export async function deleteRun(id: string): Promise<void> {
  await api.delete(`/identify/runs/${id}`, v2);
}

export async function revertRun(id: string): Promise<number> {
  const { data } = await api.post<{ reverted: number }>(`/identify/runs/${id}/revert`, {}, v2);
  return data.reverted;
}

export async function acceptSure(id: string): Promise<number> {
  const { data } = await api.post<{ accepted: number }>(`/identify/runs/${id}/accept`, { class: "sure" }, v2);
  return data.accepted;
}

export async function listProposals(
  runId: string,
  filter: { class?: ProposalClass; decision?: Decision },
  cursor?: string,
): Promise<Page<Proposal>> {
  const { data } = await api.get<Page<Proposal>>(`/identify/runs/${runId}/proposals`, {
    ...v2,
    params: { ...filter, cursor, limit: 200 },
  });
  return data;
}

export async function acceptProposal(id: string): Promise<Proposal> {
  const { data } = await api.post<Proposal>(`/identify/proposals/${id}/accept`, {}, v2);
  return data;
}

export async function rejectProposal(id: string): Promise<Proposal> {
  const { data } = await api.post<Proposal>(`/identify/proposals/${id}/reject`, {}, v2);
  return data;
}

export async function setKeepAlbum(id: string, keep_album: boolean): Promise<Proposal> {
  const { data } = await api.patch<Proposal>(`/identify/proposals/${id}`, { keep_album }, v2);
  return data;
}

export async function proposalAlternatives(id: string): Promise<RecordingCandidate[]> {
  const { data } = await api.get<{ items: RecordingCandidate[] }>(`/identify/proposals/${id}/alternatives`, v2);
  return data.items;
}

/** Search again in the owner's words; the results become the proposal's alternatives. */
export async function searchProposal(id: string, search: RecordingSearch): Promise<RecordingCandidate[]> {
  const { data } = await api.post<{ items: RecordingCandidate[] }>(`/identify/proposals/${id}/search`, search, v2);
  return data.items;
}

/** A recording by id or musicbrainz.org link becomes the proposal, accepted when `accept`. */
export async function chooseRecording(id: string, recording: string, release: string | null | undefined, accept: boolean): Promise<Proposal> {
  const { data } = await api.put<Proposal>(`/identify/proposals/${id}/recording`, { recording, release: release ?? null, accept }, v2);
  return data;
}

/** Search an album's editions again with the owner's words (revision 14). */
export async function searchGroup(id: string, search: { album?: string; artist?: string }): Promise<IdentifyGroup> {
  const { data } = await api.post<IdentifyGroup>(`/identify/groups/${id}/search`, search, v2);
  return data;
}

export async function getGroup(id: string): Promise<IdentifyGroup> {
  const { data } = await api.get<IdentifyGroup>(`/identify/groups/${id}`, v2);
  return data;
}

export async function chooseEdition(groupId: string, index: number): Promise<IdentifyGroup> {
  const { data } = await api.put<IdentifyGroup>(`/identify/groups/${groupId}/edition`, { index }, v2);
  return data;
}

export interface LibraryFolder {
  id: string;
  path: string;
  kind: string;
}

/** The read-only library folders (v1), for scoping a run to one. */
export async function listLibraryFolders(): Promise<LibraryFolder[]> {
  const { data } = await api.get<{ folders?: LibraryFolder[] }>("/library/folders");
  return (data.folders ?? []).filter((f) => f.kind === "music");
}

/** A v2 problem's message, else v1's `error`, else the fallback. */
export function identifyError(err: unknown, fallback: string): string {
  const data = (err as { response?: { data?: { detail?: string; title?: string; error?: string } } })?.response?.data;
  return data?.detail ?? data?.title ?? data?.error ?? fallback;
}

/** Still working: poll it. */
export function isActive(run?: IdentifyRun): boolean {
  return !!run && (run.status === "queued" || run.status === "running" || run.status === "waiting");
}
