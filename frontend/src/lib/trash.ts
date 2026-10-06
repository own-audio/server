// SPDX-License-Identifier: AGPL-3.0-or-later
import type { QueryClient } from "@tanstack/react-query";
import { newTrashBatch, restoreTrashBatch, type TrashItem } from "../api/trash";
import type { Visibility } from "../api/types";
import { toast } from "./toast";
import { t } from "../i18n";

/**
 * May this viewer delete this item? The owner always; a family admin also for
 * anything shared with the family (docs/file-sync-plan.md §2 item 4).
 *
 * Hiding the button is UX — the server decides (403/404) either way.
 */
export function canDelete(item: { is_owner: boolean; visibility: Visibility }, isFamilyAdmin: boolean): boolean {
  return item.is_owner || (isFamilyAdmin && item.visibility === "family");
}

/**
 * Move one thing to the trash and offer Undo — no confirmation for your own
 * items, because nothing is gone for 30 days and Undo on the same day is free
 * (plan §2 item 13). `remove` gets the batch id to send; `onChanged` refreshes
 * whatever lists the item appeared in, after the delete and after an Undo.
 */
export async function moveToTrash({
  title,
  remove,
  onChanged,
}: {
  title: string;
  remove: (batch: string) => Promise<void>;
  onChanged: () => void;
}): Promise<void> {
  const batch = newTrashBatch();
  await remove(batch);
  onChanged();
  toast.withAction(t("common.trash.moved"), { label: t("common.action.undo"), onClick: () => void undoTrash(batch, onChanged) }, title);
}

export async function undoTrash(batch: string, onChanged: () => void): Promise<void> {
  try {
    await restoreTrashBatch(batch);
    onChanged();
    toast.success(t("common.trash.restored"));
  } catch {
    toast.error(t("common.trash.restoreFailed"), t("common.trash.stillInTrash"));
  }
}

/** A restore can bring an item back into any list, so refresh them all. */
export function refreshEverything(qc: QueryClient): () => void {
  return () => void qc.invalidateQueries();
}

/** Whole days until the purge, never negative. */
export function daysLeft(purgeAt: string, now: Date = new Date()): number {
  const ms = new Date(purgeAt).getTime() - now.getTime();
  return Math.max(0, Math.ceil(ms / 86_400_000));
}

export interface TrashGroup {
  key: string;
  batch: string | null;
  trashedAt: string;
  items: TrashItem[];
}

/**
 * One group per deletion — everything that shares a batch — newest first.
 * An item without a batch is its own group.
 */
export function groupByDeletion(items: TrashItem[]): TrashGroup[] {
  const groups = new Map<string, TrashGroup>();
  for (const item of items) {
    const key = item.batch ?? `${item.kind}:${item.id}`;
    const g = groups.get(key);
    if (g) {
      g.items.push(item);
      if (item.trashed_at > g.trashedAt) g.trashedAt = item.trashed_at;
    } else {
      groups.set(key, { key, batch: item.batch, trashedAt: item.trashed_at, items: [item] });
    }
  }
  return [...groups.values()].sort((a, b) => b.trashedAt.localeCompare(a.trashedAt));
}
