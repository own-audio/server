// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { ListMusic, LocateFixed, Plus, Trash2 } from "lucide-react";
import * as Popover from "@radix-ui/react-popover";
import { saveTrackChapters } from "../../api/music";
import { apiErrorMessage } from "../../lib/apiError";
import { usePlayerStore } from "../../store/playerStore";
import { formatChapterTime, parseChapterTime, type TrackChapter } from "../../lib/chapters";
import { useSectionStyle } from "../../lib/sectionTheme";
import { cn } from "../../lib/cn";
import { Button, IconButton, Input } from "../ui";
import { Dialog, DialogContent } from "../ui/Dialog";
import { useT } from "../../i18n";

/** The songs of the concert playing — chapters inside one track — to jump to one; its owner
 *  edits them from here (docs/concerts-plan.md). */
export function ConcertSongsPopover({ current, position, onSeek }: { current: number | null; position: number; onSeek: (secs: number) => void }) {
  const { track, trackChapters } = usePlayerStore();
  const [editing, setEditing] = useState(false);
  const sectionStyle = useSectionStyle();
  const { t } = useT();
  if (track?.kind !== "music" || trackChapters.length === 0) return null;

  return (
    <>
      <Popover.Root>
        <Popover.Trigger asChild>
          <IconButton size="sm" label={t("player.concert.songs")}>
            <ListMusic className="h-4 w-4" />
          </IconButton>
        </Popover.Trigger>
        <Popover.Portal>
          <Popover.Content
            side="top" sideOffset={10} align="end" style={sectionStyle}
            className="z-[60] max-h-96 w-80 overflow-y-auto rounded-card border border-border bg-card p-1.5 shadow-pop outline-none animate-pop-in"
          >
            <div className="flex items-center justify-between px-2 py-1.5">
              <p className="text-xs font-semibold uppercase tracking-wide text-muted">
                {current != null ? t("player.concert.songOf", { n: current + 1, total: trackChapters.length }) : t("player.concert.songs")}
              </p>
              {track.isOwner && (
                <Popover.Close asChild>
                  <Button size="sm" variant="ghost" onClick={() => setEditing(true)}>{t("common.action.edit")}</Button>
                </Popover.Close>
              )}
            </div>
            {trackChapters.map((chapter, i) => (
              // Picking a song closes the list, as in the apps.
              <Popover.Close asChild key={`${chapter.startSecs}-${i}`}>
                <button
                  onClick={() => onSeek(chapter.startSecs)}
                  className={cn(
                    "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors",
                    i === current ? "bg-accent/10 text-accent" : "hover:bg-bg-alt"
                  )}
                >
                  <span className="w-6 shrink-0 text-right text-xs tabular-nums text-muted">{i + 1}</span>
                  <span className="min-w-0 flex-1 truncate">{chapter.title}</span>
                  <span className="shrink-0 text-xs tabular-nums text-muted">{formatChapterTime(chapter.startSecs)}</span>
                </button>
              </Popover.Close>
            ))}
          </Popover.Content>
        </Popover.Portal>
      </Popover.Root>
      {editing && <ConcertSongsEditor trackId={track.trackId} position={position} onClose={() => setEditing(false)} />}
    </>
  );
}

interface Row {
  key: number;
  title: string;
  time: string;
}

/** Rename, move to where the player is, add there, delete; saved as a whole. From then on the
 *  server keeps these, whatever the file's marks say. */
function ConcertSongsEditor({ trackId, position, onClose }: { trackId: string; position: number; onClose: () => void }) {
  const { trackChapters, setTrackChapters } = usePlayerStore();
  const { t } = useT();
  const [rows, setRows] = useState<Row[]>(() =>
    trackChapters.map((c, i) => ({ key: i, title: c.title, time: formatChapterTime(c.startSecs) }))
  );
  const [nextKey, setNextKey] = useState(trackChapters.length);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const update = (key: number, change: Partial<Row>) => setRows((rs) => rs.map((r) => (r.key === key ? { ...r, ...change } : r)));

  async function save() {
    const chapters: TrackChapter[] = [];
    for (const row of rows) {
      const start = parseChapterTime(row.time);
      if (start == null) return setError(t("player.concert.badTime", { time: row.time }));
      if (!row.title.trim()) return setError(t("player.concert.needsName"));
      chapters.push({ title: row.title.trim(), startSecs: start });
    }
    chapters.sort((a, b) => a.startSecs - b.startSecs);
    if (chapters.some((c, i) => i > 0 && c.startSecs === chapters[i - 1].startSecs)) return setError(t("player.concert.sameStart"));
    setSaving(true);
    setError(null);
    try {
      setTrackChapters(await saveTrackChapters(trackId, chapters));
      onClose();
    } catch (err) {
      setError(apiErrorMessage(err, t("player.concert.saveFailed")));
    } finally {
      setSaving(false);
    }
  }

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("player.concert.editSongs")}
        description={t("player.concert.editHint")}
        className="max-w-xl"
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>{t("common.action.cancel")}</Button>
            <Button onClick={() => void save()} loading={saving}>{t("common.action.save")}</Button>
          </>
        }
      >
        <ol className="flex flex-col gap-2">
          {rows.map((row) => (
            <li key={row.key} className="flex items-center gap-2">
              <Input
                aria-label={t("player.concert.time")}
                value={row.time}
                onChange={(e) => update(row.key, { time: e.target.value })}
                error={parseChapterTime(row.time) == null ? " " : undefined}
                className="w-24 shrink-0 tabular-nums"
              />
              <Input aria-label={t("player.concert.song")} value={row.title} onChange={(e) => update(row.key, { title: e.target.value })} className="min-w-0 flex-1" />
              <IconButton size="sm" label={t("player.concert.startHere")} onClick={() => update(row.key, { time: formatChapterTime(position) })}>
                <LocateFixed className="h-4 w-4" />
              </IconButton>
              <IconButton size="sm" label={t("common.action.delete")} onClick={() => setRows((rs) => rs.filter((r) => r.key !== row.key))}>
                <Trash2 className="h-4 w-4" />
              </IconButton>
            </li>
          ))}
        </ol>
        <Button
          variant="ghost" size="sm" className="mt-3" icon={<Plus className="h-4 w-4" />}
          onClick={() => { setRows((rs) => [...rs, { key: nextKey, title: "", time: formatChapterTime(position) }]); setNextKey((k) => k + 1); }}
        >
          {t("player.concert.addHere")}
        </Button>
        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
