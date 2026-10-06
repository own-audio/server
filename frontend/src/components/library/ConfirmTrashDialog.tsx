// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Button, Dialog, DialogContent, toast } from "../ui";
import { useT } from "../../i18n";

/**
 * The one delete that asks first: a family admin removing an item that belongs
 * to someone else (docs/file-sync-plan.md §2 item 13). Your own items go to
 * the trash straight away with Undo; this one touches another person's things,
 * and they get a notification, so it is worth a moment.
 */
export function ConfirmTrashDialog({
  title,
  onConfirm,
  onClose,
}: {
  title: string;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const { t } = useT();

  async function confirm() {
    setBusy(true);
    try {
      await onConfirm();
      onClose();
    } catch {
      toast.error(t("library.confirmTrash.failed"));
      setBusy(false);
    }
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("library.confirmTrash.title", { title })}
        description={t("library.confirmTrash.description")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button variant="danger" loading={busy} onClick={() => void confirm()}>
              {t("common.action.moveToTrash")}
            </Button>
          </>
        }
      >
        {null}
      </DialogContent>
    </Dialog>
  );
}
