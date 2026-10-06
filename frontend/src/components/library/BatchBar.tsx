// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Trash2, Users, UserX, X } from "lucide-react";
import { Button, IconButton, Dialog, DialogContent, toast } from "../ui";
import { runWithLimit } from "../../lib/uploadQueue";
import { newTrashBatch } from "../../api/trash";
import { undoTrash } from "../../lib/trash";
import type { Visibility } from "../../api/types";
import { t as translate, useT } from "../../i18n";

/**
 * The action bar for a multi-selection.
 *
 * **Every batch is N client-side requests** — the API has no bulk endpoints
 * beyond episode progress. So each action runs with a concurrency limit and
 * reports per-item failures: a batch of 12 with one failure must land the other
 * 11 and say which one didn't, not fail as a unit.
 */

const CONCURRENCY = 4;

/** The first few that failed, by name, and how many more. */
function failedList(failed: { item: BatchTarget }[]): string {
  const names = failed.map((f) => f.item.label).slice(0, 3).join(", ");
  return failed.length > 3
    ? translate("library.batch.failedForMore", { names, more: failed.length - 3 })
    : translate("library.batch.failedFor", { names });
}

export interface BatchTarget {
  id: string;
  label: string;
  /** False for family-shared items owned by someone else — those can't be changed. */
  isOwner: boolean;
  /** May go to the trash: yours, or shared with the family when you are its
   *  admin. Defaults to `isOwner`. */
  canDelete?: boolean;
}

export function BatchBar({
  selected,
  onClear,
  onSetVisibility,
  onDelete,
  onTrash,
  afterTrash,
  extraActions,
}: {
  selected: BatchTarget[];
  onClear: () => void;
  onSetVisibility?: (id: string, v: Visibility) => Promise<void>;
  /** A permanent delete (unfollowing a podcast). Books, tracks and playlists
   *  use `onTrash` instead. */
  onDelete?: (id: string) => Promise<void>;
  /** Move one item to the trash; every item of the batch gets the same `batch`
   *  so one Undo restores them all. */
  onTrash?: (id: string, batch: string) => Promise<void>;
  /** Refresh lists after the batch goes to the trash and after an Undo. */
  afterTrash?: () => void;
  extraActions?: React.ReactNode;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const { t } = useT();

  if (selected.length === 0) return null;

  const owned = selected.filter((s) => s.isOwner);
  const notOwned = selected.length - owned.length;
  const trashable = selected.filter((s) => s.canDelete ?? s.isOwner);
  const othersTrashable = trashable.filter((s) => !s.isOwner).length;

  async function trash() {
    if (!onTrash) return;
    setBusy("delete");
    const batch = newTrashBatch();
    try {
      const { failed } = await runWithLimit(trashable, CONCURRENCY, (item) => onTrash(item.id, batch));
      const ok = trashable.length - failed.length;
      afterTrash?.();
      if (failed.length === 0) {
        toast.withAction(
          t("library.batch.movedToTrash", { count: ok }),
          { label: t("common.action.undo"), onClick: () => void undoTrash(batch, () => afterTrash?.()) }
        );
      } else {
        toast.error(t("library.batch.partlyMoved", { ok, total: trashable.length }), failedList(failed));
      }
      onClear();
    } finally {
      setBusy(null);
      setConfirmDelete(false);
    }
  }

  async function run(name: string, action: (item: BatchTarget) => Promise<void>, doneMessage: (n: number) => string) {
    setBusy(name);
    try {
      const { failed } = await runWithLimit(owned, CONCURRENCY, action);
      const ok = owned.length - failed.length;
      if (failed.length === 0) {
        toast.success(doneMessage(ok));
      } else {
        toast.error(t("library.batch.partlyDone", { ok, total: owned.length }), failedList(failed));
      }
      onClear();
    } finally {
      setBusy(null);
      setConfirmDelete(false);
    }
  }

  return (
    <>
      <div className="sticky bottom-0 z-20 mx-3 mb-3 flex flex-wrap items-center gap-2 rounded-card border border-border bg-card px-3 py-2 shadow-pop">
        <span className="text-sm font-medium">
          {t("library.batch.selected", { count: selected.length })}
          {notOwned > 0 && <span className="ml-1 text-xs font-normal text-muted">{t("library.batch.notYours", { count: notOwned })}</span>}
        </span>

        <div className="ml-auto flex flex-wrap items-center gap-1.5">
          {extraActions}
          {onSetVisibility && (
            <>
              <Button
                size="sm"
                variant="secondary"
                icon={<Users className="h-4 w-4" />}
                disabled={owned.length === 0 || !!busy}
                loading={busy === "share"}
                onClick={() => run("share", (item) => onSetVisibility(item.id, "family"), (n) => t("library.batch.shared", { count: n }))}
              >
                {t("common.action.share")}
              </Button>
              <Button
                size="sm"
                variant="secondary"
                icon={<UserX className="h-4 w-4" />}
                disabled={owned.length === 0 || !!busy}
                loading={busy === "private"}
                onClick={() => run("private", (item) => onSetVisibility(item.id, "private"), (n) => t("library.batch.madePrivate", { count: n }))}
              >
                {t("library.batch.makePrivate")}
              </Button>
            </>
          )}
          {(onDelete || onTrash) && (
            <Button
              size="sm"
              variant="danger"
              icon={<Trash2 className="h-4 w-4" />}
              disabled={(onTrash ? trashable.length : owned.length) === 0 || !!busy}
              onClick={() => setConfirmDelete(true)}
            >
              {onTrash ? t("common.action.moveToTrash") : t("common.action.delete")}
            </Button>
          )}
          <IconButton size="sm" label={t("library.batch.clearSelection")} onClick={onClear}>
            <X className="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      {onTrash ? (
        <Dialog open={confirmDelete} onOpenChange={setConfirmDelete}>
          <DialogContent
            title={t("library.batch.trashTitle", { count: trashable.length })}
            description={
              othersTrashable > 0
                ? t("library.batch.trashBodyOthers", { count: othersTrashable })
                : t("library.batch.trashBody")
            }
            footer={
              <>
                <Button variant="ghost" onClick={() => setConfirmDelete(false)}>
                  {t("common.action.cancel")}
                </Button>
                <Button variant="danger" loading={busy === "delete"} onClick={() => void trash()}>
                  {t("common.action.moveToTrash")}
                </Button>
              </>
            }
          >
            <ul className="max-h-48 space-y-1 overflow-y-auto text-sm">
              {trashable.map((item) => (
                <li key={item.id} className="truncate text-muted">
                  {item.label}
                </li>
              ))}
            </ul>
          </DialogContent>
        </Dialog>
      ) : (
        <Dialog open={confirmDelete} onOpenChange={setConfirmDelete}>
          <DialogContent
            title={t("library.batch.deleteTitle", { count: owned.length })}
            description={t("library.batch.deleteBody")}
            footer={
              <>
                <Button variant="ghost" onClick={() => setConfirmDelete(false)}>
                  {t("common.action.cancel")}
                </Button>
                <Button
                  variant="danger"
                  loading={busy === "delete"}
                  onClick={() => onDelete && run("delete", (item) => onDelete(item.id), (n) => t("library.batch.deleted", { count: n }))}
                >
                  {t("common.action.delete")}
                </Button>
              </>
            }
          >
            <ul className="max-h-48 space-y-1 overflow-y-auto text-sm">
              {owned.map((item) => (
                <li key={item.id} className="truncate text-muted">
                  {item.label}
                </li>
              ))}
            </ul>
          </DialogContent>
        </Dialog>
      )}
    </>
  );
}
