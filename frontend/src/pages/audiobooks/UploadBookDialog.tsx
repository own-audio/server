// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useRef, useState, type DragEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQueryClient } from "@tanstack/react-query";
import { FolderOpen, Upload, Languages, Sparkles } from "lucide-react";
import { createBookFromUploads, type FromUploadsFile } from "../../api/audiobooks";
import { uploadToStorage } from "../../api/uploads";
import { formatDuration } from "../../lib/format";
import { apiErrorMessage } from "../../lib/apiError";
import { newUploadId, useUploadQueue } from "../../lib/uploadQueue";
import { filesFromDrop } from "../../lib/dropFiles";
import { Button, Dialog, DialogContent, Input, Textarea, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

const AUDIO_EXTENSIONS = new Set(["mp3", "m4a", "m4b", "aac", "flac", "ogg", "opus", "wav", "mp4"]);
const IMAGE_EXTENSIONS = new Set(["jpg", "jpeg", "png", "webp"]);
const CONCURRENCY = 4;

interface Pending { file: File; relativePath: string; durationSecs: number | null }

const ext = (name: string) => name.split(".").pop()?.toLowerCase() ?? "";
const isAudio = (f: File) => f.type.startsWith("audio/") || AUDIO_EXTENSIONS.has(ext(f.name));
const isCoverName = (path: string) => /(^|\/)(cover|folder|front)\.[a-z]+$/i.test(path);

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

/* Files go straight to object storage with presigned PUTs; only the JSON
   manifest touches the API. Uploading through the API would cap a book at the
   proxy's 100 MB body limit. Play order is decided server-side from
   relative_path, so workers finishing out of order is fine. */
export default function UploadBookDialog() {
  const navigate = useNavigate();
  const qc = useQueryClient();
  const { t } = useT();
  const [files, setFiles] = useState<Pending[]>([]);
  const [cover, setCover] = useState<File | null>(null);
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [narrator, setNarrator] = useState("");
  const [description, setDescription] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [status, setStatus] = useState<{ text: string; done: number; total: number } | null>(null);
  const [dragging, setDragging] = useState(false);
  const fileInput = useRef<HTMLInputElement>(null);
  const folderInput = useRef<HTMLInputElement>(null);

  const inferredTitle = useMemo(() => {
    if (files.length === 0) return "";
    const first = files[0].relativePath;
    const root = first.split("/")[0] ?? "";
    if (files.length > 1 && files.every((f) => f.relativePath.startsWith(root))) return root;
    return files[0].file.name.replace(/\.[^.]+$/, "");
  }, [files]);

  async function accept(candidates: { file: File; relativePath: string }[]) {
    const audio = candidates.filter((c) => isAudio(c.file));
    if (audio.length === 0) { setError(t("audiobooks.upload.noAudio")); return; }
    const images = candidates.filter((c) => IMAGE_EXTENSIONS.has(ext(c.file.name)));
    setCover(candidates.find((c) => isCoverName(c.relativePath))?.file ?? images[0]?.file ?? null);
    setError(null);
    const withDuration = await Promise.all(audio.map(async (c) => ({ ...c, durationSecs: await probeDuration(c.file) })));
    setFiles(withDuration.sort((a, b) => a.relativePath.localeCompare(b.relativePath, undefined, { numeric: true, sensitivity: "base" })));
  }

  const queue = useUploadQueue();

  const upload = useMutation({
    mutationFn: async () => {
      const jobId = newUploadId();
      const label = (title.trim() || inferredTitle || t("audiobooks.upload.defaultTitle")).trim();
      queue.add({ id: jobId, label, kind: "audiobook", bytes: files.reduce((n, f) => n + f.file.size, 0) });
      queue.update(jobId, { status: "uploading" });
      const total = files.length + (cover ? 1 : 0);
      let done = 0;
      const step = (text: string) => {
        setStatus({ text, done, total });
        queue.update(jobId, { progress: done / total });
      };

      let coverKey: string | undefined;
      if (cover) {
        step(t("audiobooks.upload.statusCover"));
        coverKey = await uploadToStorage("audiobook_cover", cover);
        done += 1;
      }

      const uploaded: FromUploadsFile[] = new Array(files.length);
      let next = 0;
      step(t("audiobooks.upload.statusFiles", { count: files.length }));
      await Promise.all(
        Array.from({ length: Math.min(CONCURRENCY, files.length) }, async () => {
          while (next < files.length) {
            const i = next++;
            const item = files[i];
            const key = await uploadToStorage("audiobook_file", item.file, item.relativePath);
            uploaded[i] = { object_key: key, relative_path: item.relativePath, duration_secs: item.durationSecs ?? undefined };
            done += 1;
            step(t("audiobooks.upload.statusProgress", { done, total }));
          }
        })
      );

      step(t("audiobooks.upload.statusCreating"));
      queue.update(jobId, { status: "done", progress: 1 });
      return createBookFromUploads({
        title: label,
        author: author.trim() || undefined,
        narrator: narrator.trim() || undefined,
        description: description.trim() || undefined,
        cover_object_key: coverKey,
        files: uploaded,
      });
    },
    onSuccess: (book) => {
      qc.invalidateQueries({ queryKey: ["books"] });
      toast.success(t("audiobooks.upload.added"), book.title);
      navigate(`/audiobooks/${book.id}`, { replace: true });
    },
    onError: (err) => {
      setStatus(null);
      const failed = useUploadQueue.getState().items.find((i) => i.status === "uploading");
      if (failed) queue.update(failed.id, { status: "failed", error: t("audiobooks.upload.queueFailed") });
      setError(apiErrorMessage(err, t("audiobooks.upload.error")));
    },
  });

  async function onDrop(e: DragEvent<HTMLDivElement>) {
    e.preventDefault();
    setDragging(false);
    if (e.dataTransfer.items?.length) await accept(await filesFromDrop(e.dataTransfer.items));
  }

  async function onPick(list: FileList | null) {
    if (!list) return;
    await accept(Array.from(list).map((file) => ({ file, relativePath: (file as File & { webkitRelativePath?: string }).webkitRelativePath || file.name })));
  }

  return (
    <Dialog open onOpenChange={(v) => { if (!v) navigate("/audiobooks"); }}>
      <DialogContent
        title={t("audiobooks.upload.title")}
        description={t("audiobooks.upload.description")}
        className="sm:max-w-2xl"
        footer={
          <>
            <Button variant="ghost" onClick={() => navigate("/audiobooks")}>{t("common.action.cancel")}</Button>
            <Button onClick={() => upload.mutate()} loading={upload.isPending} disabled={files.length === 0}>
              {files.length > 0 ? t("audiobooks.upload.submit", { count: files.length }) : t("common.action.upload")}
            </Button>
          </>
        }
      >
        <div
          onDragOver={(e) => { e.preventDefault(); setDragging(true); }}
          onDragLeave={() => setDragging(false)}
          onDrop={(e) => void onDrop(e)}
          className={cn("rounded-card border-2 border-dashed px-5 py-8 text-center transition-colors", dragging ? "border-accent bg-accent/5" : "border-border bg-bg-alt")}
        >
          <Upload className="mx-auto h-7 w-7 text-muted" />
          <p className="mt-2 text-sm font-medium">{t("audiobooks.upload.dropTitle")}</p>
          <p className="mt-1 text-xs text-muted">{t("audiobooks.upload.dropHint")}</p>
          <div className="mt-4 flex justify-center gap-2">
            <Button type="button" variant="secondary" size="sm" onClick={() => fileInput.current?.click()}>{t("audiobooks.upload.chooseFiles")}</Button>
            <Button type="button" variant="secondary" size="sm" icon={<FolderOpen className="h-4 w-4" />} onClick={() => folderInput.current?.click()}>{t("audiobooks.upload.chooseFolder")}</Button>
          </div>
          <input ref={fileInput} type="file" multiple hidden accept="audio/*,.m4b,.m4a,.mp3,.flac,.ogg,.opus,.wav,.aac,.jpg,.jpeg,.png,.webp" onChange={(e) => void onPick(e.target.files)} />
          {/* @ts-expect-error Chromium directory picker */}
          <input ref={folderInput} type="file" multiple hidden webkitdirectory="" onChange={(e) => void onPick(e.target.files)} />
        </div>

        {/* Narration lives in its own wizard; offering it here is where someone with a book
            but no recording goes looking. It reads text, so it needs an ebook, not audio. */}
        {files.length === 0 && (
          <div className="mt-5">
            <p className="text-sm font-medium">{t("audiobooks.upload.orNarrate")}</p>
            <div className="mt-2 grid grid-cols-1 gap-2 sm:grid-cols-2">
              {[
                { to: "/generate", icon: <Sparkles className="h-5 w-5" />, title: t("audiobooks.upload.narrate.title"), body: t("audiobooks.upload.narrate.body") },
                { to: "/generate?translate=1", icon: <Languages className="h-5 w-5" />, title: t("audiobooks.upload.translate.title"), body: t("audiobooks.upload.translate.body") },
              ].map((o) => (
                <button
                  key={o.to}
                  type="button"
                  onClick={() => navigate(o.to)}
                  className="flex items-start gap-3 rounded-card border border-border p-3 text-left transition-colors hover:border-accent/60 hover:bg-accent/5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
                >
                  <span className="mt-0.5 text-accent-text">{o.icon}</span>
                  <span className="min-w-0">
                    <span className="block text-sm font-medium">{o.title}</span>
                    <span className="mt-0.5 block text-xs text-muted">{o.body}</span>
                  </span>
                </button>
              ))}
            </div>
          </div>
        )}

        {files.length > 0 && (
          <div className="mt-4 rounded-card border border-border">
            <div className="flex items-center justify-between border-b border-border px-3 py-2">
              <p className="text-sm font-medium">
                {cover
                  ? t("audiobooks.upload.readyWithCover", { count: files.length, name: cover.name })
                  : t("audiobooks.upload.ready", { count: files.length })}
              </p>
              <button className="text-xs text-muted hover:text-fg" onClick={() => { setFiles([]); setCover(null); setStatus(null); }}>{t("audiobooks.upload.clear")}</button>
            </div>
            <ul className="max-h-40 overflow-y-auto">
              {files.map((f) => (
                <li key={f.relativePath} className="flex items-center justify-between gap-3 border-b border-border px-3 py-1.5 text-xs last:border-b-0">
                  <span className="min-w-0 truncate">{f.relativePath}</span>
                  <span className="shrink-0 tabular-nums text-muted">{f.durationSecs != null ? formatDuration(f.durationSecs) : "—"}</span>
                </li>
              ))}
            </ul>
          </div>
        )}

        <div className="mt-4 grid gap-3 sm:grid-cols-3">
          <Input label={t("audiobooks.field.title")} value={title} onChange={(e) => setTitle(e.target.value)} placeholder={inferredTitle || t("audiobooks.upload.titlePlaceholder")} />
          <Input label={t("audiobooks.field.author")} value={author} onChange={(e) => setAuthor(e.target.value)} placeholder={t("audiobooks.upload.authorPlaceholder")} />
          <Input label={t("audiobooks.field.narrator")} value={narrator} onChange={(e) => setNarrator(e.target.value)} placeholder={t("audiobooks.upload.narratorPlaceholder")} />
        </div>
        <div className="mt-3">
          <Textarea label={t("audiobooks.field.description")} value={description} onChange={(e) => setDescription(e.target.value)} placeholder={t("audiobooks.optional")} />
        </div>

        {status && (
          <div className="mt-4 rounded-card bg-accent/10 px-3 py-2 text-sm text-accent">
            <p>{status.text}</p>
            <div className="mt-2 h-1.5 rounded-pill bg-accent/20">
              <div className="h-full rounded-pill bg-accent transition-[width]" style={{ width: `${Math.max(6, (status.done / Math.max(1, status.total)) * 100)}%` }} />
            </div>
          </div>
        )}
        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
