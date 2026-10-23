// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Button, Dialog, DialogContent, toast } from "../../components/ui";
import { mergeAlbum } from "../../api/organize";
import type { AlbumSummary } from "../../api/types";
import { apiErrorMessage } from "../../lib/apiError";
import { useT } from "../../i18n";

/**
 * Albums of one name and artist made one: songs identified one by one can land
 * on different editions of a release, and each edition is an album of its own.
 * The one with the most songs stays.
 */
export default function MergeAlbumsDialog({ albums, onClose, onMerged }: { albums: AlbumSummary[]; onClose: () => void; onMerged: (into: AlbumSummary) => void }) {
  const { t } = useT();
  const [merging, setMerging] = useState(false);
  const into = [...albums].sort((a, b) => b.track_count - a.track_count)[0];

  async function merge() {
    setMerging(true);
    try {
      for (const a of albums) {
        if (a.id && into.id && a.id !== into.id) await mergeAlbum(a.id, into.id);
      }
      toast.success(t("music.merge.done"));
      onMerged(into);
      onClose();
    } catch (e) {
      // A part merged before the error stays merged; the list shows what is left.
      onMerged(into);
      toast.error(t("music.merge.error"), apiErrorMessage(e, ""));
    } finally {
      setMerging(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        title={t("music.merge.title", { album: into.album })}
        description={t("music.merge.description", { count: albums.length, artist: into.artist })}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>{t("common.action.cancel")}</Button>
            <Button loading={merging} onClick={() => void merge()}>{t("music.merge.confirm")}</Button>
          </>
        }
      >
        <ul className="divide-y divide-border rounded-card border border-border text-sm">
          {[...albums].sort((a, b) => b.track_count - a.track_count).map((a) => (
            <li key={a.id} className="flex justify-between px-3 py-2">
              <span className="truncate">{a.album}</span>
              <span className="shrink-0 text-muted">{t("music.count.songs", { count: a.track_count })}</span>
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
