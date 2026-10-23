// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Button, Dialog, DialogContent, Input, toast } from "../../components/ui";
import { saveTracks, undoBatch, type TrackFields } from "../../api/organize";
import type { MusicTrack } from "../../api/types";
import { apiErrorMessage } from "../../lib/apiError";
import { useT } from "../../i18n";

/** The server takes this many songs a save (`PATCH /api/v2/tracks`). */
const SAVE_LIMIT = 500;

/** The same value on every song, or "" when they differ. */
function common(tracks: MusicTrack[], pick: (t: MusicTrack) => string | null | undefined): string {
  const values = new Set(tracks.map((t) => pick(t) ?? ""));
  return values.size === 1 ? [...values][0] : "";
}

/**
 * One album name (and album artist) for many songs at once: a folder whose
 * files spell their album three ways, or songs that belong together. Only the
 * caller's own songs change, as one undoable batch of the edit history.
 */
export default function EditAlbumDialog({ tracks, onClose, onSaved }: { tracks: MusicTrack[]; onClose: () => void; onSaved: () => void }) {
  const { t } = useT();
  const own = tracks.filter((tr) => tr.is_owner);
  const [album, setAlbum] = useState(() => common(own, (tr) => tr.album));
  const [albumArtist, setAlbumArtist] = useState(() => common(own, (tr) => tr.album_artist));
  const [saving, setSaving] = useState(false);

  const fields: TrackFields = {};
  if (album.trim()) fields.album = album.trim();
  if (albumArtist.trim()) fields.album_artist = albumArtist.trim();
  const nothing = Object.keys(fields).length === 0 || own.length === 0;

  async function save() {
    setSaving(true);
    const batches: string[] = [];
    try {
      for (let i = 0; i < own.length; i += SAVE_LIMIT) {
        const saved = await saveTracks(own.slice(i, i + SAVE_LIMIT).map((tr) => ({ id: tr.id, ...fields })));
        batches.push(saved.batch_id);
      }
      onSaved();
      onClose();
      toast.withAction(t("music.editAlbum.saved", { count: own.length }), {
        label: t("common.action.undo"),
        onClick: () => void Promise.all(batches.map((b) => undoBatch(b))).then(onSaved),
      });
    } catch (e) {
      // A part saved before the error stays undoable as one.
      if (batches.length) onSaved();
      toast.error(t("music.editAlbum.error"), apiErrorMessage(e, ""));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogContent
        title={t("music.editAlbum.title", { count: own.length })}
        description={t("music.editAlbum.description")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>{t("common.action.cancel")}</Button>
            <Button disabled={nothing} loading={saving} onClick={() => void save()}>{t("common.action.save")}</Button>
          </>
        }
      >
        <div className="grid gap-3">
          <Input label={t("music.editAlbum.album")} value={album} placeholder={t("music.editAlbum.keep")} onChange={(e) => setAlbum(e.target.value)} autoFocus />
          <Input label={t("music.editAlbum.albumArtist")} value={albumArtist} placeholder={t("music.editAlbum.keep")} onChange={(e) => setAlbumArtist(e.target.value)} />
          {own.length < tracks.length && (
            <p className="text-xs text-muted">{t("music.editAlbum.notOwned", { count: tracks.length - own.length })}</p>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
