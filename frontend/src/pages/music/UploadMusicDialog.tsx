// SPDX-License-Identifier: AGPL-3.0-or-later
import { useRef, useState, type DragEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useQueryClient } from "@tanstack/react-query";
import { Upload } from "lucide-react";
import { uploadTrack } from "../../api/music";
import { Button, Dialog, DialogContent, toast } from "../../components/ui";
import { newUploadId, useUploadQueue } from "../../lib/uploadQueue";
import { byPath, filesFromDrop } from "../../lib/dropFiles";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

const AUDIO_EXTENSIONS = new Set(["mp3", "m4a", "m4b", "ogg", "opus", "flac", "wav", "aac", "wma", "webm"]);

function probeDuration(file: File): Promise<number | null> {
  return new Promise((resolve) => {
    const audio = document.createElement("audio");
    const url = URL.createObjectURL(file);
    audio.preload = "metadata";
    const done = (v: number | null) => { URL.revokeObjectURL(url); audio.remove(); resolve(v); };
    audio.onloadedmetadata = () => done(isFinite(audio.duration) ? Math.round(audio.duration) : null);
    audio.onerror = () => done(null);
    audio.src = url;
  });
}

/** One request per track — there is no batch music endpoint. */
export default function UploadMusicDialog() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { t } = useT();
  const input = useRef<HTMLInputElement>(null);
  const [dragging, setDragging] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [progress, setProgress] = useState<{ done: number; total: number; name: string } | null>(null);
  const [failed, setFailed] = useState<string[]>([]);

  async function upload(list: FileList | File[]) {
    const files = Array.from(list).filter((f) => f.type.startsWith("audio/") || AUDIO_EXTENSIONS.has(f.name.split(".").pop()?.toLowerCase() ?? ""));
    if (files.length === 0) return;
    const errors: string[] = [];
    const q = useUploadQueue.getState();
    for (let i = 0; i < files.length; i++) {
      const file = files[i];
      const jobId = newUploadId();
      q.add({ id: jobId, label: file.name, kind: "music", bytes: file.size });
      q.update(jobId, { status: "uploading" });
      setProgress({ done: i, total: files.length, name: file.name });
      try {
        await uploadTrack({ file, durationSecs: (await probeDuration(file)) ?? undefined });
        q.update(jobId, { status: "done", progress: 1 });
      } catch {
        errors.push(file.name);
        q.update(jobId, { status: "failed", error: t("music.upload.jobFailed") });
      }
    }
    setProgress(null);
    setFailed(errors);
    qc.invalidateQueries({ queryKey: ["music-tracks"] });
    qc.invalidateQueries({ queryKey: ["music-artists"] });
    qc.invalidateQueries({ queryKey: ["music-albums"] });
    if (errors.length === 0) {
      toast.success(t("music.upload.done", { count: files.length }));
      navigate("/music", { replace: true });
    }
  }

  async function onDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault();
    setDragging(false);
    // Both are read before the first await: the DataTransfer is neutered as
    // soon as this handler yields.
    const items = e.dataTransfer.items;
    const plain = Array.from(e.dataTransfer.files);
    if (items?.length) {
      // Walking a big folder takes long enough to look like nothing happened.
      setScanning(true);
      try {
        const found = await filesFromDrop(items);
        if (found.length) {
          // Album order, so the progress line reads sensibly and any
          // tag-less files keep the order they sit in on disk.
          return await upload(found.sort(byPath).map((f) => f.file));
        }
      } finally {
        setScanning(false);
      }
    }
    if (plain.length) void upload(plain);
  }

  return (
    <Dialog open onOpenChange={(v) => { if (!v && !progress) navigate("/music"); }}>
      <DialogContent title={t("music.upload.title")} description={t("music.upload.description")}>
        <div
          onDragOver={(e) => { e.preventDefault(); setDragging(true); }}
          onDragLeave={() => setDragging(false)}
          onDrop={(e) => void onDrop(e)}
          className={cn("rounded-card border-2 border-dashed px-5 py-10 text-center transition-colors", dragging ? "border-accent bg-accent/5" : "border-border bg-bg-alt")}
        >
          <Upload className="mx-auto h-7 w-7 text-muted" />
          <p className="mt-2 text-sm font-medium">{t("music.upload.drop")}</p>
          <p className="mt-1 text-xs text-muted">
            {t(scanning ? "music.upload.scanning" : "music.upload.hint")}
          </p>
          <Button type="button" variant="secondary" size="sm" className="mt-4" onClick={() => input.current?.click()} disabled={!!progress || scanning}>{t("music.upload.chooseFiles")}</Button>
          <input ref={input} type="file" accept="audio/*" multiple hidden onChange={(e) => e.target.files && void upload(e.target.files)} />
        </div>

        {progress && (
          <div className="mt-4 rounded-card bg-accent/10 px-3 py-2 text-sm text-accent">
            <p className="truncate">{t("music.upload.progress", { current: progress.done + 1, total: progress.total, name: progress.name })}</p>
            <div className="mt-2 h-1.5 rounded-pill bg-accent/20"><div className="h-full rounded-pill bg-accent transition-[width]" style={{ width: `${Math.max(6, (progress.done / progress.total) * 100)}%` }} /></div>
          </div>
        )}

        {failed.length > 0 && (
          <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">
            {t("music.upload.failed", { count: failed.length, names: failed.join(", ") })}
          </p>
        )}
      </DialogContent>
    </Dialog>
  );
}
