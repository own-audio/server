// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { moveToTrash } from "../../lib/trash";
import { removeDownload } from "../../lib/offline/downloads";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Copy, Loader2, Search, Trash2 } from "lucide-react";
import {
  applyTrackMetadata,
  deleteTrack,
  getTrackLyrics,
  identifyAlbum,
  listDuplicates,
  searchTrackMetadata,
  setTrackLyrics,
  type AlbumCandidate,
  type MetadataCandidate,
} from "../../api/music";
import { formatBytes } from "../../api/billing";
import { formatDuration } from "../../lib/format";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, EmptyState, Input, Pill, Skeleton, Textarea, toast } from "../../components/ui";
import { Page } from "../../components/shell/SplitView";
import { cn } from "../../lib/cn";
import type { MusicTrack } from "../../api/types";
import { useT } from "../../i18n";

import { MUSIC_KEYS } from "./musicKeys";

// ── Identify ──────────────────────────────────────────────────────────────

function Candidate({
  candidate,
  onApply,
  applying,
}: {
  candidate: MetadataCandidate;
  onApply: () => void;
  applying: boolean;
}) {
  const { t } = useT();
  const [artOk, setArtOk] = useState(true);
  return (
    <li className="flex gap-3 rounded-card border border-border p-3">
      {/* The cover URL is speculative and may 404 — hide it quietly. */}
      {candidate.cover_art_url && artOk ? (
        <img
          src={candidate.cover_art_url}
          alt=""
          loading="lazy"
          onError={() => setArtOk(false)}
          className="h-14 w-14 shrink-0 rounded-lg object-cover"
        />
      ) : (
        <span className="h-14 w-14 shrink-0 rounded-lg bg-bg-alt" />
      )}
      <div className="min-w-0 flex-1">
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">{candidate.title}</p>
            <p className="truncate text-xs text-muted">
              {[candidate.artist, candidate.album].filter(Boolean).join(" — ") || t("music.identify.unknown")}
            </p>
          </div>
          <Button size="sm" loading={applying} onClick={onApply}>
            {t("music.action.useThis")}
          </Button>
        </div>
        <div className="mt-1.5 flex flex-wrap gap-1.5 text-[11px] text-muted">
          {candidate.year && <Pill>{candidate.year}</Pill>}
          {candidate.track_number != null && <Pill>{t("music.identify.trackNumber", { number: candidate.track_number })}</Pill>}
          {candidate.duration_ms != null && <Pill>{formatDuration(Math.round(candidate.duration_ms / 1000))}</Pill>}
          {candidate.genre && <Pill>{candidate.genre}</Pill>}
          <Pill tone={candidate.score >= 90 ? "success" : "neutral"}>{t("music.identify.score", { score: candidate.score })}</Pill>
        </div>
      </div>
    </li>
  );
}

export function IdentifyTrackSheet({ track, onClose }: { track: MusicTrack; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [title, setTitle] = useState(track.title);
  const [artist, setArtist] = useState(track.artist ?? "");
  const [album, setAlbum] = useState(track.album ?? "");
  const [results, setResults] = useState<MetadataCandidate[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const search = useMutation({
    mutationFn: () =>
      searchTrackMetadata(track.id, {
        title: title.trim() || undefined,
        artist: artist.trim() || undefined,
        album: album.trim() || undefined,
      }),
    onSuccess: setResults,
    onError: (err) => setError(apiErrorMessage(err, t("music.error.lookup"))),
  });

  const apply = useMutation({
    mutationFn: (c: MetadataCandidate) =>
      applyTrackMetadata(track.id, {
        mb_recording_id: c.mb_recording_id,
        mb_release_id: c.mb_release_id ?? undefined,
        fetch_cover: true,
      }),
    onSuccess: (updated) => {
      MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
      toast.success(t("music.toast.trackUpdated"), updated.title);
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("music.identify.applyFailed"))),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.identify.trackTitle")}
        description={t("music.identify.trackDescription")}
        className="sm:max-w-2xl"
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
        <div className="grid gap-3 sm:grid-cols-3">
          <Input label={t("music.field.title")} value={title} onChange={(e) => setTitle(e.target.value)} />
          <Input label={t("music.field.artist")} value={artist} onChange={(e) => setArtist(e.target.value)} />
          <Input label={t("music.field.album")} value={album} onChange={(e) => setAlbum(e.target.value)} />
        </div>

        {search.isPending && <Skeleton className="mt-4 h-40" />}

        {results && results.length === 0 && (
          <p className="mt-4 text-sm text-muted">{t("music.error.noResults")}</p>
        )}

        {results && results.length > 0 && (
          <ul className="mt-4 space-y-2">
            {results.map((c) => (
              <Candidate
                key={c.mb_recording_id}
                candidate={c}
                applying={apply.isPending && apply.variables?.mb_recording_id === c.mb_recording_id}
                onApply={() => apply.mutate(c)}
              />
            ))}
          </ul>
        )}

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}

// ── Lyrics ────────────────────────────────────────────────────────────────

export function LyricsSheet({ track, onClose }: { track: Pick<MusicTrack, "id" | "title">; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [draft, setDraft] = useState<string | null>(null);
  const { data: lyrics, isLoading } = useQuery({
    queryKey: ["lyrics", track.id],
    queryFn: () => getTrackLyrics(track.id),
    retry: false,
  });

  const value = draft ?? lyrics ?? "";

  const save = useMutation({
    mutationFn: () => setTrackLyrics(track.id, value),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["lyrics", track.id] });
      toast.success(t("music.lyrics.saved"));
      onClose();
    },
    onError: () => toast.error(t("music.lyrics.saveFailed")),
  });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={track.title}
        description={t("music.lyrics.description")}
        className="sm:max-w-xl"
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.close")}
            </Button>
            <Button loading={save.isPending} disabled={draft === null} onClick={() => save.mutate()}>
              {t("common.action.save")}
            </Button>
          </>
        }
      >
        {isLoading ? (
          <Skeleton className="h-64" />
        ) : (
          <>
            {lyrics === null && draft === null && (
              <p className="mb-3 text-sm text-muted">{t("music.lyrics.noTag")}</p>
            )}
            <Textarea
              value={value}
              onChange={(e) => setDraft(e.target.value)}
              className="min-h-64 font-[inherit] leading-relaxed"
              placeholder={t("music.lyrics.placeholder")}
            />
          </>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ── Duplicates ────────────────────────────────────────────────────────────

/**
 * Byte-identical copies of the same file, grouped by checksum.
 *
 * Deleting is one request per track and never automatic: a duplicate might be
 * in a playlist, and the app has no way to know which copy someone meant to
 * keep.
 */
export default function DuplicatesPage() {
  const qc = useQueryClient();
  const { t } = useT();
  const [busy, setBusy] = useState<string | null>(null);
  const { data: groups = [], isLoading } = useQuery({ queryKey: ["duplicates"], queryFn: listDuplicates, retry: false });

  async function remove(id: string) {
    setBusy(id);
    try {
      await moveToTrash({
        title: t("music.duplicates.trashTitle"),
        remove: async (batch) => {
          await deleteTrack(id, batch);
          await removeDownload(id).catch(() => {});
        },
        onChanged: () => {
          MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
          qc.invalidateQueries({ queryKey: ["duplicates"] });
        },
      });
    } catch {
      toast.error(t("music.duplicates.trashFailed"));
    } finally {
      setBusy(null);
    }
  }

  if (isLoading) {
    return (
      <Page title={t("music.duplicates.title")} back="/music" width="max-w-3xl">
        <Skeleton className="h-40" />
      </Page>
    );
  }

  return (
    <Page title={t("music.duplicates.title")} back="/music" width="max-w-3xl">
      {groups.length === 0 ? (
        <EmptyState
          icon={<Copy />}
          title={t("music.duplicates.emptyTitle")}
          description={t("music.duplicates.emptyDescription")}
        />
      ) : (
        <>
          <p className="mb-4 text-sm text-muted">
            {t("music.duplicates.summary", { count: groups.length })}
          </p>
          <div className="space-y-3">
            {groups.map((g, i) => (
              <div key={i} className="rounded-card border border-border">
                <div className="flex items-center gap-2 border-b border-border px-4 py-2">
                  <Pill tone="warning">{t("music.duplicates.copies", { count: g.tracks.length })}</Pill>
                  <span className="text-xs text-muted">
                    {g.tracks[0]?.title}
                    {g.tracks[0]?.artist ? ` · ${g.tracks[0].artist}` : ""}
                  </span>
                </div>
                {g.tracks.map((track, index) => (
                  <div key={track.id} className={cn("flex items-center gap-3 px-4 py-2.5", index > 0 && "border-t border-border")}>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm">{track.title}</span>
                      <span className="block truncate text-xs text-muted">
                        {[track.album, track.duration_secs ? formatDuration(track.duration_secs) : null, track.size_bytes ? formatBytes(track.size_bytes) : null]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>
                    <Button
                      size="sm"
                      variant="ghost"
                      className="text-error"
                      icon={<Trash2 className="h-4 w-4" />}
                      loading={busy === track.id}
                      onClick={() => remove(track.id)}
                    >
                      {t("music.duplicates.trashCopy")}
                    </Button>
                  </div>
                ))}
              </div>
            ))}
          </div>
        </>
      )}
    </Page>
  );
}

// ── Batch identify ────────────────────────────────────────────────────────

/** "Live", "Compilation · Bootleg" — what makes an album not the album. */
function albumKind(a: AlbumCandidate): string {
  const parts = [...a.secondary_types];
  if (parts.length === 0 && a.primary_type && a.primary_type !== "Album") parts.push(a.primary_type);
  if (a.status && a.status !== "Official") parts.push(a.status);
  return parts.join(" · ");
}

/**
 * Identify an album, artist or selection as a whole.
 *
 * Matching every track on its own let an album's songs scatter: each title's
 * best match was chosen without knowing about the others, so one landed on a
 * festival bootleg, another on a game soundtrack. This asks once which albums
 * the whole group could be, lets the user pick one — the studio album first
 * when several hold the songs equally — and matches every track to that one.
 * Matching one by one is still there for a mixed selection.
 */
export function IdentifyBatchSheet({ tracks, label, onClose }: { tracks: MusicTrack[]; label: string; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [oneByOne, setOneByOne] = useState(false);
  const [picked, setPicked] = useState<string | null>(null);
  const [album, setAlbum] = useState<AlbumCandidate | null>(null);
  const [unticked, setUnticked] = useState<Set<string>>(new Set());
  const [applying, setApplying] = useState(false);

  const albums = useQuery({
    queryKey: ["album-identify", tracks.map((t) => t.id).join(",")],
    queryFn: () => identifyAlbum(tracks.map((t) => t.id)),
    enabled: !oneByOne,
    staleTime: Infinity,
    retry: false,
  });

  if (oneByOne) return <IdentifyEachSheet tracks={tracks} label={label} onClose={onClose} />;

  const list = albums.data ?? [];
  const selected = list.find((a) => a.mb_release_group_id === picked) ?? list[0] ?? null;

  if (album) {
    const byTrack = new Map(album.tracks.map((m) => [m.track_id, m]));
    const multiDisc = album.tracks.some((m) => m.disc > 1);
    const toApply = tracks.filter((tr) => byTrack.has(tr.id) && !unticked.has(tr.id));

    const apply = async () => {
      setApplying(true);
      let done = 0;
      const failed: string[] = [];
      for (const tr of toApply) {
        try {
          await applyTrackMetadata(tr.id, {
            mb_recording_id: byTrack.get(tr.id)!.mb_recording_id,
            mb_release_id: album.mb_release_id,
            fetch_cover: true,
          });
          done++;
        } catch {
          failed.push(tr.title);
        }
      }
      MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
      setApplying(false);
      if (failed.length === 0) toast.success(t("music.identify.updated", { count: done }), album.title);
      else toast.error(t("music.identify.partialUpdated", { done, total: toApply.length }), t("music.identify.failedFor", { names: failed.slice(0, 3).join(", ") }));
      onClose();
    };

    return (
      <Dialog open onOpenChange={(v) => !v && onClose()}>
        <DialogContent
          title={album.title}
          description={t("music.identify.albumDescription", { details: [album.artist, album.year, albumKind(album)].filter(Boolean).join(" · ") })}
          className="sm:max-w-2xl"
          footer={
            <>
              <Button variant="ghost" onClick={() => setAlbum(null)}>
                {t("common.action.back")}
              </Button>
              <Button loading={applying} disabled={toApply.length === 0} onClick={() => void apply()}>
                {t("music.identify.applyMatches", { count: toApply.length })}
              </Button>
            </>
          }
        >
          <ul className="max-h-96 divide-y divide-border overflow-y-auto rounded-card border border-border">
            {tracks.map((tr) => {
              const m = byTrack.get(tr.id);
              return (
                <li key={tr.id} className="flex items-center gap-3 px-3 py-2">
                  {m ? (
                    <input
                      type="checkbox"
                      checked={!unticked.has(tr.id)}
                      onChange={() =>
                        setUnticked((prev) => {
                          const next = new Set(prev);
                          if (next.has(tr.id)) next.delete(tr.id);
                          else next.add(tr.id);
                          return next;
                        })
                      }
                      aria-label={t("music.identify.applyFor", { title: tr.title })}
                      className="h-4 w-4 shrink-0 rounded accent-[var(--accent)]"
                    />
                  ) : (
                    <span className="h-4 w-4 shrink-0" />
                  )}
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm">{tr.title}</span>
                    {m ? (
                      <span className="block truncate text-xs text-success-text">
                        → {multiDisc ? `${m.disc}.` : ""}
                        {m.position} {m.title}
                      </span>
                    ) : (
                      <span className="block truncate text-xs text-muted">{t("music.identify.notOnAlbum")}</span>
                    )}
                  </span>
                </li>
              );
            })}
          </ul>
        </DialogContent>
      </Dialog>
    );
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.identify.title", { label })}
        description={t("music.identify.pickAlbum", { count: tracks.length })}
        className="sm:max-w-2xl"
        footer={
          <>
            <Button variant="ghost" onClick={() => setOneByOne(true)}>
              {t("music.identify.oneByOne")}
            </Button>
            <Button disabled={!selected} onClick={() => selected && setAlbum(selected)}>
              {t("music.identify.useAlbum")}
            </Button>
          </>
        }
      >
        {albums.isLoading ? (
          <div className="flex items-center gap-2 py-10 text-sm text-muted">
            <Loader2 className="h-4 w-4 animate-spin" /> {t("music.identify.lookingForAlbum")}
          </div>
        ) : albums.isError ? (
          <p className="py-6 text-sm text-muted">{t("music.identify.albumLookupFailed")}</p>
        ) : list.length === 0 ? (
          <p className="py-6 text-sm text-muted">
            {t(tracks.some((tr) => tr.artist) ? "music.identify.noAlbum" : "music.identify.noAlbumNoArtist")}
          </p>
        ) : (
          <ul className="max-h-96 divide-y divide-border overflow-y-auto rounded-card border border-border" role="radiogroup" aria-label={t("music.mode.albums")}>
            {list.map((a) => {
              const kind = albumKind(a);
              const isSelected = selected?.mb_release_group_id === a.mb_release_group_id;
              return (
                <li key={a.mb_release_group_id}>
                  <button
                    type="button"
                    role="radio"
                    aria-checked={isSelected}
                    onClick={() => setPicked(a.mb_release_group_id)}
                    onDoubleClick={() => setAlbum(a)}
                    className={cn("flex w-full items-center gap-3 px-3 py-2 text-left", isSelected ? "bg-accent/8" : "hover:bg-bg-alt")}
                  >
                    {a.cover_art_url ? (
                      // The 250 px scan is plenty for a 44 px thumbnail.
                      <img src={a.cover_art_url.replace(/front-500$/, "front-250")} alt="" loading="lazy" className="h-11 w-11 shrink-0 rounded-md bg-bg-alt object-cover" />
                    ) : (
                      <span className="h-11 w-11 shrink-0 rounded-md bg-bg-alt" />
                    )}
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm font-medium">{a.title}</span>
                      <span className={cn("block truncate text-xs", kind ? "text-warning" : "text-muted")}>
                        {[a.artist, a.year, kind || a.primary_type].filter(Boolean).join(" · ")}
                        {a.editions > 1 ? ` · ${t("music.identify.editions", { count: a.editions })}` : ""}
                      </span>
                    </span>
                    <Pill tone={a.matched === tracks.length ? "success" : undefined}>
                      {t("music.count.ofTotal", { count: a.matched, total: tracks.length })}
                    </Pill>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </DialogContent>
    </Dialog>
  );
}

// ── Identify one by one ───────────────────────────────────────────────────

interface BatchRow {
  track: MusicTrack;
  state: "waiting" | "searching" | "matched" | "none" | "failed";
  candidate?: MetadataCandidate;
}

/**
 * Identify a whole album or artist in one pass.
 *
 * It searches every track, then shows what it found and applies only what you
 * confirm — an automatic apply would silently rewrite metadata on a bad match,
 * and MusicBrainz returns a plausible-looking wrong answer often enough that
 * this has to stay a review step.
 *
 * The backend rate-limits MusicBrainz to about one request a second across the
 * whole process, so the searches run one at a time rather than in parallel;
 * firing twenty at once would just queue behind each other anyway.
 */
function IdentifyEachSheet({ tracks, label, onClose }: { tracks: MusicTrack[]; label: string; onClose: () => void }) {
  const qc = useQueryClient();
  const { t } = useT();
  const [rows, setRows] = useState<BatchRow[]>(() => tracks.map((track) => ({ track, state: "waiting" })));
  const [running, setRunning] = useState(false);
  const [applying, setApplying] = useState(false);
  /** Which matches the user has unticked, or null while they haven't touched
   *  the list — in which case the default below applies. Derived rather than
   *  synced, so the default follows the scan results without an effect. */
  const [choice, setChoice] = useState<Set<string> | null>(null);

  const matched = rows.filter((r) => r.state === "matched" && r.candidate);

  /* Identifying one album, most tracks land on the same release — so the ones
     that don't are the suspicious ones. This is what catches a track matched
     to a live bootleg or a tribute record while its neighbours all agree on
     the real album: the score alone doesn't, since a wrong match can score
     high against the wrong release. */
  const majorityAlbum = (() => {
    const counts = new Map<string, number>();
    for (const r of matched) {
      const a = r.candidate!.album;
      if (a) counts.set(a, (counts.get(a) ?? 0) + 1);
    }
    let best: string | null = null;
    let n = 0;
    for (const [album, count] of counts) if (count > n) [best, n] = [album, count];
    // One agreeing pair proves nothing; require a real majority before calling
    // anything an outlier.
    return n >= 3 && n > matched.length / 2 ? best : null;
  })();

  const isOutlier = (r: BatchRow) =>
    majorityAlbum != null && r.candidate?.album != null && r.candidate.album !== majorityAlbum;

  /* Outliers start unticked. Applying one is how good metadata gets silently
     overwritten, and there is no undo — so the safe default leaves them out
     and lets the user opt them back in. */
  const defaultUnchecked = () => new Set(matched.filter(isOutlier).map((r) => r.track.id));
  const unchecked = choice ?? defaultUnchecked();
  const included = matched.filter((r) => !unchecked.has(r.track.id));

  function toggle(id: string) {
    setChoice((prev) => {
      const next = new Set(prev ?? defaultUnchecked());
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  async function scan() {
    setRunning(true);
    for (let i = 0; i < rows.length; i++) {
      setRows((prev) => prev.map((r, j) => (j === i ? { ...r, state: "searching" } : r)));
      try {
        const found = await searchTrackMetadata(rows[i].track.id, {
          title: rows[i].track.title,
          artist: rows[i].track.artist ?? undefined,
          album: rows[i].track.album ?? undefined,
          limit: 1,
        });
        setRows((prev) =>
          prev.map((r, j) =>
            j === i ? { ...r, state: found[0] ? "matched" : "none", candidate: found[0] } : r
          )
        );
      } catch {
        setRows((prev) => prev.map((r, j) => (j === i ? { ...r, state: "failed" } : r)));
      }
    }
    setRunning(false);
  }

  async function applyAll() {
    setApplying(true);
    let done = 0;
    const failed: string[] = [];
    for (const row of included) {
      try {
        await applyTrackMetadata(row.track.id, {
          mb_recording_id: row.candidate!.mb_recording_id,
          mb_release_id: row.candidate!.mb_release_id ?? undefined,
          fetch_cover: true,
        });
        done++;
      } catch {
        failed.push(row.track.title);
      }
    }
    MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
    setApplying(false);
    if (failed.length === 0) toast.success(t("music.identify.updated", { count: done }));
    else toast.error(t("music.identify.partialUpdated", { done, total: included.length }), t("music.identify.failedFor", { names: failed.slice(0, 3).join(", ") }));
    onClose();
  }

  const STATE_LABEL = {
    none: "music.identify.noMatch",
    failed: "music.identify.lookupFailed",
  } as const;

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("music.identify.title", { label })}
        description={t("music.identify.reviewEach", { count: tracks.length })}
        className="sm:max-w-2xl"
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.close")}
            </Button>
            {rows.every((r) => r.state === "waiting") ? (
              <Button icon={<Search className="h-4 w-4" />} loading={running} onClick={scan}>
                {t("music.identify.lookThemUp")}
              </Button>
            ) : (
              <Button loading={applying} disabled={running || included.length === 0} onClick={applyAll}>
                {included.length > 0 ? t("music.identify.applyMatches", { count: included.length }) : t("music.action.apply")}
              </Button>
            )}
          </>
        }
      >
        <ul className="max-h-96 divide-y divide-border overflow-y-auto rounded-card border border-border">
          {rows.map((r) => (
            <li key={r.track.id} className="flex items-center gap-3 px-3 py-2">
              {r.state === "matched" && r.candidate && (
                <input
                  type="checkbox"
                  checked={!unchecked.has(r.track.id)}
                  onChange={() => toggle(r.track.id)}
                  aria-label={t("music.identify.applyFor", { title: r.track.title })}
                  className="h-4 w-4 shrink-0 rounded accent-[var(--accent)]"
                />
              )}
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm">{r.track.title}</span>
                {r.state === "matched" && r.candidate ? (
                  <>
                    <span className={cn("block truncate text-xs", isOutlier(r) ? "text-warning" : "text-success-text")}>
                      → {r.candidate.title}
                      {r.candidate.artist ? ` · ${r.candidate.artist}` : ""}
                      {r.candidate.album ? ` · ${r.candidate.album}` : ""}
                    </span>
                    {isOutlier(r) && (
                      <span className="mt-0.5 flex items-center gap-1 text-xs text-warning">
                        <AlertTriangle className="h-3 w-3 shrink-0" />
                        {t("music.identify.outlier")}
                      </span>
                    )}
                  </>
                ) : (
                  <span className="block truncate text-xs text-muted">{r.track.artist ?? t("music.identify.unknownArtist")}</span>
                )}
              </span>
              {r.state === "searching" && <Loader2 className="h-4 w-4 shrink-0 animate-spin text-muted" />}
              {r.state === "matched" && r.candidate && <Pill tone="success">{r.candidate.score}%</Pill>}
              {(r.state === "none" || r.state === "failed") && <Pill>{t(STATE_LABEL[r.state])}</Pill>}
            </li>
          ))}
        </ul>
      </DialogContent>
    </Dialog>
  );
}
