// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { moveToTrash } from "../../lib/trash";
import { removeDownload } from "../../lib/offline/downloads";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { ImageUp, Trash2 } from "lucide-react";
import { deleteTrack, setTrackVisibility, updateTrack, uploadTrackCover } from "../../api/music";
import { MUSIC_KEYS } from "./musicKeys";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, Input, toast } from "../../components/ui";
import { VisibilityField } from "../../components/library/VisibilityField";
import AudienceList from "../../components/library/AudienceList";
import type { MusicTrack, Visibility } from "../../api/types";
import { useT } from "../../i18n";


export default function EditTrackSheet({ track, onClose }: { track: MusicTrack; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [form, setForm] = useState({
    title: track.title,
    artist: track.artist ?? "",
    album: track.album ?? "",
    genre: track.genre ?? "",
    trackNumber: track.track_number != null ? String(track.track_number) : "",
  });
  const [cover, setCover] = useState<File | null>(null);
  const [error, setError] = useState<string | null>(null);

  const invalidate = () => MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));

  const save = useMutation({
    mutationFn: async () => {
      const n = form.trackNumber.trim() ? parseInt(form.trackNumber.trim(), 10) : undefined;
      await updateTrack(track.id, {
        title: form.title.trim(),
        artist: form.artist.trim() || undefined,
        album: form.album.trim() || undefined,
        genre: form.genre.trim() || undefined,
        track_number: Number.isFinite(n) ? n : undefined,
      });
      if (cover) await uploadTrackCover(track.id, cover);
    },
    onSuccess: () => {
      invalidate();
      toast.success(t("music.toast.trackUpdated"));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.error.saveChanges"))),
  });

  const changeVisibility = useMutation({
    mutationFn: (v: Visibility) => setTrackVisibility(track.id, v),
    onSuccess: (_d, v) => {
      invalidate();
      toast.success(t(v === "family" ? "music.toast.sharedWithFamily" : "music.toast.madePrivate"));
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.error.visibility"))),
  });

  const [trashing, setTrashing] = useState(false);
  async function trash() {
    setTrashing(true);
    try {
      await moveToTrash({
        title: track.title,
        remove: async (batch) => {
          await deleteTrack(track.id, batch);
          await removeDownload(track.id).catch(() => {});
        },
        onChanged: invalidate,
      });
      onClose();
    } catch (err) {
      setError(apiErrorMessage(err, t("music.error.trashTrack")));
      setTrashing(false);
    }
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.edit.title")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!form.title.trim()}>
              {t("common.action.save")}
            </Button>
          </>
        }
      >
        <div className="grid gap-3 sm:grid-cols-2">
          <Input label={t("music.field.title")} value={form.title} onChange={(e) => setForm({ ...form, title: e.target.value })} />
          <Input label={t("music.field.artist")} value={form.artist} onChange={(e) => setForm({ ...form, artist: e.target.value })} />
          <Input label={t("music.field.album")} value={form.album} onChange={(e) => setForm({ ...form, album: e.target.value })} />
          <Input label={t("music.field.genre")} value={form.genre} onChange={(e) => setForm({ ...form, genre: e.target.value })} />
          <Input
            label={t("music.field.trackNumber")}
            inputMode="numeric"
            value={form.trackNumber}
            onChange={(e) => setForm({ ...form, trackNumber: e.target.value })}
          />
          <label className="block">
            <span className="mb-1.5 block text-[13px] font-medium text-fg">{t("music.field.cover")}</span>
            <label className="flex h-10 cursor-pointer items-center gap-2 rounded-[10px] border border-border bg-card px-3 text-sm text-muted hover:text-fg">
              <ImageUp className="h-4 w-4" />
              <span className="truncate">{cover ? cover.name : t("music.field.chooseImage")}</span>
              <input type="file" accept="image/*" hidden onChange={(e) => setCover(e.target.files?.[0] ?? null)} />
            </label>
          </label>
        </div>

        <div className="mt-4">
          <VisibilityField
            value={track.visibility}
            disabled={!track.is_owner || changeVisibility.isPending}
            onChange={(v) => changeVisibility.mutate(v)}
          />
        </div>

        <div className="mt-4">
          <p className="mb-1.5 text-[13px] font-medium">{t("music.edit.audience")}</p>
          <AudienceList kind="music" itemId={track.id} />
        </div>

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}

        {track.is_owner && (
          <div className="mt-6 border-t border-border pt-4">
            <Button size="sm" variant="ghost" icon={<Trash2 className="h-4 w-4" />} className="text-error" loading={trashing} onClick={() => void trash()}>
              {t("music.action.moveToTrash")}
            </Button>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
