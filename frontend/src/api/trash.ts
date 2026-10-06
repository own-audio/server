// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";

/* The 30-day trash (docs/file-sync-plan.md §5.1, docs/android-client-guide.md
   §8b). Every delete route moves its item here; nothing is gone until it has
   waited 30 days or someone deletes it forever. */

/** `companion_file`: an image, booklet or lyrics file kept next to the audio. */
export type TrashKind = "audiobook" | "music_track" | "playlist" | "podcast_episode" | "companion_file";
export type TrashScope = "mine" | "family";

export interface TrashPerson {
  id: string;
  display_name: string | null;
}

export interface TrashItem {
  kind: TrashKind;
  id: string;
  title: string;
  owner: TrashPerson;
  /** Null once the account that deleted it is gone. */
  trashed_by: TrashPerson | null;
  trashed_at: string;
  purge_at: string;
  size_bytes: number;
  /** Everything deleted in one gesture shares it; restore them together. */
  batch: string | null;
  /** What restoring it now charges — the storage for its days in the trash. Zero on the day it was deleted. */
  restore_charge_micro: number;
}

export interface RestoreResult {
  restored: number;
  charged_micro: number;
}

/** Sent on every delete of one gesture, so the trash can undo it as one. */
export const TRASH_BATCH_HEADER = "X-Trash-Batch";

export function newTrashBatch(): string {
  return crypto.randomUUID();
}

export function batchHeaders(batch?: string) {
  return batch ? { headers: { [TRASH_BATCH_HEADER]: batch } } : undefined;
}

export async function listTrash(scope: TrashScope = "mine"): Promise<TrashItem[]> {
  const { data } = await api.get<TrashItem[]>("/trash", { params: { scope } });
  return data;
}

export async function restoreTrashItem(kind: TrashKind, id: string): Promise<RestoreResult> {
  const { data } = await api.post<RestoreResult>(`/trash/${kind}/${id}/restore`);
  return data;
}

export async function restoreTrashBatch(batch: string): Promise<RestoreResult> {
  const { data } = await api.post<RestoreResult>(`/trash/batches/${batch}/restore`);
  return data;
}

export async function purgeTrashItem(kind: TrashKind, id: string): Promise<void> {
  await api.delete(`/trash/${kind}/${id}`);
}

export async function emptyTrash(scope: TrashScope = "mine"): Promise<{ purged: number }> {
  const { data } = await api.post<{ purged: number }>("/trash/empty", undefined, { params: { scope } });
  return data;
}
