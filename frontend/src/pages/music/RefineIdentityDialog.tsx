// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FileAudio, Library, Search } from "lucide-react";
import {
  applyTrackMetadata,
  getTrackFileTags,
  searchTrackMetadata,
  type MetadataCandidate,
} from "../../api/music";
import type { MusicTrack } from "../../api/types";
import { MUSIC_KEYS } from "./musicKeys";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, Input, Skeleton, toast } from "../../components/ui";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";

/** A MusicBrainz recording id, as pasted from a musicbrainz.org URL or on its own. */
const MBID = /[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/i;

const FIELD_LABEL: Record<"title" | "artist" | "album", PlainKey> = {
  title: "music.field.title",
  artist: "music.field.artist",
  album: "music.field.album",
};

function Source({
  icon,
  label,
  note,
  values,
  fileName,
  onUse,
}: {
  icon: React.ReactNode;
  label: string;
  note: string;
  values: { title?: string | null; artist?: string | null; album?: string | null };
  fileName?: string | null;
  onUse?: () => void;
}) {
  const { t } = useT();
  const empty = !values.title && !values.artist && !values.album;
  return (
    <div className="rounded-card border border-border p-3">
      <p className="flex items-center gap-2 text-[13px] font-medium">
        {icon}
        {label}
      </p>
      <p className="mt-0.5 text-xs text-muted">{note}</p>
      {empty ? (
        <p className="mt-2 text-sm text-muted">{t("music.refine.nothingRecorded")}</p>
      ) : (
        <dl className="mt-2 space-y-0.5 text-sm">
          {(["title", "artist", "album"] as const).map((k) => (
            <div key={k} className="flex gap-2">
              <dt className="w-14 shrink-0 text-xs text-muted">{t(FIELD_LABEL[k])}</dt>
              <dd className="min-w-0 flex-1 truncate">{values[k] || <span className="text-muted">—</span>}</dd>
            </div>
          ))}
        </dl>
      )}
      {fileName && (
        <p className="mt-2 border-t border-border pt-2 text-xs text-muted">
          <span className="block">{t("music.refine.fileName")}</span>
          <span className="mt-0.5 block break-all font-mono text-[11px] text-fg">{fileName}</span>
        </p>
      )}
      {onUse && (
        <Button size="sm" variant="secondary" className="mt-2" onClick={onUse}>
          {t("music.refine.searchWithThese")}
        </Button>
      )}
    </div>
  );
}

/**
 * Correct a track whose identity came out wrong.
 *
 * A whole-album identify matches each recording on its own, so a run that gets
 * ten right can still get two badly wrong — and the damage compounds: the
 * search boxes are seeded from the track's *current* values, which after a bad
 * match are the wrong ones. So this shows both sources side by side and lets
 * either seed the search.
 *
 * The file's own tags matter here because applying a match never rewrites the
 * file. After a bad identify the library says one thing and the bytes still say
 * another — and the file is frequently the one that was right.
 */
export default function RefineIdentityDialog({ track, onClose }: { track: MusicTrack; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [title, setTitle] = useState(track.title);
  const [artist, setArtist] = useState(track.artist ?? "");
  const [album, setAlbum] = useState(track.album ?? "");
  const [mbid, setMbid] = useState("");
  const [results, setResults] = useState<MetadataCandidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const fileTags = useQuery({
    queryKey: ["file-tags", track.id],
    queryFn: () => getTrackFileTags(track.id),
    retry: false,
    staleTime: Infinity,
  });

  const search = useMutation({
    mutationFn: () =>
      searchTrackMetadata(track.id, {
        title: title.trim() || undefined,
        artist: artist.trim() || undefined,
        album: album.trim() || undefined,
        limit: 15,
      }),
    onSuccess: (r) => {
      setResults(r);
      setError(null);
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.error.lookup"))),
  });

  const apply = useMutation({
    mutationFn: (v: { recordingId: string; releaseId?: string }) =>
      applyTrackMetadata(track.id, {
        mb_recording_id: v.recordingId,
        mb_release_id: v.releaseId,
        fetch_cover: true,
      }),
    onSuccess: (updated) => {
      MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
      qc.invalidateQueries({ queryKey: ["file-tags", track.id] });
      toast.success(t("music.refine.corrected"), `${updated.title}${updated.album ? ` — ${updated.album}` : ""}`);
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.refine.applyFailed"))),
  });

  function useFileTags() {
    const tags = fileTags.data;
    if (!tags) return;
    setTitle(tags.title ?? "");
    setArtist(tags.artist ?? "");
    setAlbum(tags.album ?? "");
    setResults(null);
  }

  function submitMbid(e: FormEvent) {
    e.preventDefault();
    const match = mbid.match(MBID);
    // Accept a pasted musicbrainz.org URL as readily as a bare id — copying the
    // address bar is the obvious thing to do once you have found the right page.
    if (!match) {
      setError(t("music.refine.badId"));
      return;
    }
    apply.mutate({ recordingId: match[0] });
  }

  const differs =
    fileTags.data &&
    ((fileTags.data.album ?? "") !== (track.album ?? "") || (fileTags.data.title ?? "") !== track.title);

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.refine.title")}
        description={t("music.refine.description")}
        className="sm:max-w-3xl"
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.close")}
            </Button>
            <Button icon={<Search className="h-4 w-4" />} loading={search.isPending} onClick={() => search.mutate()}>
              {t("common.action.search")}
            </Button>
          </>
        }
      >
        <div className="grid gap-2 sm:grid-cols-2">
          <Source
            icon={<Library className="h-4 w-4 text-music" />}
            label={t("music.refine.inLibrary")}
            note={t(track.musicbrainz_recording_id ? "music.refine.fromMatch" : "music.refine.asUploaded")}
            values={{ title: track.title, artist: track.artist, album: track.album }}
          />
          {fileTags.isLoading ? (
            <Skeleton className="h-32" />
          ) : fileTags.isError ? (
            <div className="rounded-card border border-border p-3">
              <p className="flex items-center gap-2 text-[13px] font-medium">
                <FileAudio className="h-4 w-4 text-muted" />
                {t("music.refine.inFile")}
              </p>
              <p className="mt-2 text-sm text-muted">{t("music.refine.fileTagsFailed")}</p>
            </div>
          ) : (
            <Source
              icon={<FileAudio className="h-4 w-4 text-muted" />}
              label={t("music.refine.inFile")}
              note={t("music.refine.fileNote")}
              values={fileTags.data ?? {}}
              fileName={fileTags.data?.file_name}
              onUse={fileTags.data?.title || fileTags.data?.artist || fileTags.data?.album ? useFileTags : undefined}
            />
          )}
        </div>

        {differs && (
          <p className="mt-2 text-xs text-muted">
            {t("music.refine.differs")}
          </p>
        )}

        <div className="mt-4 grid gap-3 sm:grid-cols-3">
          <Input label={t("music.field.title")} value={title} onChange={(e) => setTitle(e.target.value)} />
          <Input label={t("music.field.artist")} value={artist} onChange={(e) => setArtist(e.target.value)} />
          <Input label={t("music.field.album")} value={album} onChange={(e) => setAlbum(e.target.value)} />
        </div>

        {search.isPending && <Skeleton className="mt-4 h-40" />}

        {results && results.length === 0 && (
          <p className="mt-4 text-sm text-muted">{t("music.error.noResults")}</p>
        )}

        {results && results.length > 0 && (
          <ul className="mt-4 max-h-72 space-y-2 overflow-y-auto">
            {results.map((c) => {
              const chosen = c.mb_recording_id === track.musicbrainz_recording_id;
              return (
                <li
                  key={c.mb_recording_id}
                  className={cn(
                    "flex items-center gap-3 rounded-card border p-2.5",
                    chosen ? "border-accent bg-accent/6" : "border-border"
                  )}
                >
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm font-medium">{c.title}</span>
                    <span className="block truncate text-xs text-muted">
                      {[c.artist, c.album, c.year].filter(Boolean).join(" · ") || t("music.refine.noReleaseDetails")}
                    </span>
                  </span>
                  {chosen && <span className="shrink-0 text-xs text-muted">{t("music.refine.current")}</span>}
                  <Button
                    size="sm"
                    variant={chosen ? "secondary" : "primary"}
                    loading={apply.isPending && apply.variables?.recordingId === c.mb_recording_id}
                    disabled={apply.isPending}
                    onClick={() => apply.mutate({ recordingId: c.mb_recording_id, releaseId: c.mb_release_id ?? undefined })}
                  >
                    {t(chosen ? "music.refine.reapply" : "music.action.useThis")}
                  </Button>
                </li>
              );
            })}
          </ul>
        )}

        <form onSubmit={submitMbid} className="mt-5 border-t border-border pt-4">
          <p className="text-[13px] font-medium">{t("music.refine.knowIt")}</p>
          <p className="mt-0.5 text-xs text-muted">{t("music.refine.pasteHint")}</p>
          <div className="mt-2 flex items-end gap-2">
            <Input
              label={t("music.refine.recordingId")}
              value={mbid}
              onChange={(e) => setMbid(e.target.value)}
              placeholder="b9ad642e-b012-41c7-b72a-42cf4911a704"
              className="flex-1"
            />
            <Button type="submit" variant="secondary" loading={apply.isPending && !!mbid} disabled={!mbid.trim()}>
              {t("music.action.apply")}
            </Button>
          </div>
        </form>

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
