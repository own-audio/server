// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from "react";
import { Bookmark, Gauge, List, Moon, Trash2, Volume2, VolumeX } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import * as Popover from "@radix-ui/react-popover";
import {
  createBookmark,
  deleteBookmark,
  listBookBookmarks,
  updateBookmark,
  type BookmarkResponse,
} from "../../api/playback";
import { usePlayerStore, SPEED_MAX, SPEED_MIN, type SleepTimer } from "../../store/playerStore";
import { Button, IconButton, Input, toast } from "../ui";
import { cn } from "../../lib/cn";
import { formatClock } from "../../lib/time";
import { canSetVolume } from "../../lib/volume";
import { useSectionStyle } from "../../lib/sectionTheme";
import { t as translate, useT } from "../../i18n";

const panel =
  "z-[60] rounded-card border border-border bg-card p-3 shadow-pop outline-none animate-pop-in";

function Panel({ children, className }: { children: React.ReactNode; className?: string }) {
  const sectionStyle = useSectionStyle();
  return (
    <Popover.Portal>
      <Popover.Content side="top" sideOffset={10} align="end" className={cn(panel, className)} style={sectionStyle}>
        {children}
      </Popover.Content>
    </Popover.Portal>
  );
}

// ── Volume ────────────────────────────────────────────────────────────────

export function VolumeControl() {
  const { volume, muted, setVolume, toggleMute } = usePlayerStore();
  const { t } = useT();
  return (
    <div className="flex items-center gap-1">
      <IconButton size="sm" label={muted ? t("player.unmute") : t("player.mute")} onClick={toggleMute}>
        {muted || volume === 0 ? <VolumeX className="h-4 w-4" /> : <Volume2 className="h-4 w-4" />}
      </IconButton>
      {canSetVolume() && (
        <input
          type="range"
          min={0}
          max={1}
          step={0.01}
          value={muted ? 0 : volume}
          onChange={(e) => setVolume(Number(e.target.value))}
          className="h-1 w-20 cursor-pointer accent-[var(--accent)]"
          aria-label={t("player.volume")}
        />
      )}
    </div>
  );
}

// ── Speed ─────────────────────────────────────────────────────────────────

const PRESETS = [0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];

export function SpeedPopover() {
  const { speed, setSpeed } = usePlayerStore();
  const { t } = useT();
  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          aria-label={t("player.speed.current", { speed })}
          title={t("player.speed.title")}
          className={cn(
            "inline-flex h-8 min-w-11 items-center justify-center gap-1 rounded-pill px-2 text-xs font-semibold tabular-nums transition-colors",
            speed === 1 ? "text-muted hover:bg-bg-alt hover:text-fg" : "bg-accent/10 text-accent"
          )}
        >
          <Gauge className="h-3.5 w-3.5" />
          {t("player.speed.value", { speed })}
        </button>
      </Popover.Trigger>
      <Panel className="w-56">
        <p className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">{t("player.speed.heading")}</p>
        <div className="grid grid-cols-4 gap-1">
          {PRESETS.map((v) => (
            <button
              key={v}
              onClick={() => setSpeed(v)}
              className={cn(
                "rounded-lg py-1.5 text-xs font-medium tabular-nums transition-colors",
                v === speed ? "bg-accent text-on-accent" : "bg-bg-alt text-fg hover:bg-border"
              )}
            >
              {t("player.speed.value", { speed: v })}
            </button>
          ))}
        </div>
        <input
          type="range"
          min={SPEED_MIN}
          max={SPEED_MAX}
          step={0.05}
          value={speed}
          onChange={(e) => setSpeed(Number(e.target.value))}
          className="mt-3 w-full accent-[var(--accent)]"
          aria-label={t("player.speed.title")}
        />
      </Panel>
    </Popover.Root>
  );
}

// ── Sleep timer ───────────────────────────────────────────────────────────

const SLEEP_MINUTES = [5, 10, 15, 30, 45, 60];

function sleepLabel(timer: SleepTimer, now: number): string | null {
  if (timer.mode === "off") return null;
  if (timer.mode === "endOfTrack") return translate("player.sleep.end");
  const left = Math.max(0, Math.round((timer.endsAt - now) / 1000));
  return formatClock(left);
}

export function SleepTimerPopover() {
  const { sleepTimer, setSleepTimer } = usePlayerStore();
  const [now, setNow] = useState(() => Date.now());
  const { t } = useT();

  useEffect(() => {
    if (sleepTimer.mode !== "at") return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [sleepTimer.mode]);

  const active = sleepTimer.mode !== "off";
  const label = sleepLabel(sleepTimer, now);

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          aria-label={active ? t("player.sleep.current", { remaining: label ?? "" }) : t("player.sleep.title")}
          title={t("player.sleep.title")}
          className={cn(
            "inline-flex h-8 items-center gap-1 rounded-pill px-2 text-xs font-semibold tabular-nums transition-colors",
            active ? "bg-accent/10 text-accent" : "text-muted hover:bg-bg-alt hover:text-fg"
          )}
        >
          <Moon className="h-3.5 w-3.5" />
          {label}
        </button>
      </Popover.Trigger>
      <Panel className="w-56">
        <p className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">{t("player.sleep.heading")}</p>
        <div className="grid grid-cols-3 gap-1">
          {SLEEP_MINUTES.map((m) => (
            <button
              key={m}
              onClick={() => setSleepTimer({ mode: "at", endsAt: Date.now() + m * 60_000 })}
              className="rounded-lg bg-bg-alt py-1.5 text-xs font-medium hover:bg-border"
            >
              {t("common.duration.minutes", { m })}
            </button>
          ))}
        </div>
        <button
          onClick={() => setSleepTimer({ mode: "endOfTrack" })}
          className={cn(
            "mt-1 w-full rounded-lg py-1.5 text-xs font-medium transition-colors",
            sleepTimer.mode === "endOfTrack" ? "bg-accent text-on-accent" : "bg-bg-alt hover:bg-border"
          )}
        >
          {t("player.sleep.endOfTrack")}
        </button>
        {active && (
          <Button variant="ghost" size="sm" className="mt-2 w-full" onClick={() => setSleepTimer({ mode: "off" })}>
            {t("player.sleep.off")}
          </Button>
        )}
      </Panel>
    </Popover.Root>
  );
}

// ── Chapters / parts ──────────────────────────────────────────────────────

export function ChaptersPopover() {
  const { queue, queueIndex, chapters, playIndex } = usePlayerStore();
  const { t } = useT();
  // In a multi-file book each chapter *is* a file, so the queue already lists
  // them; a separate chapter list only says something different when the two
  // counts differ (a single M4B, or several chapters inside one file).
  const parts = queue.filter((item) => item.kind === "audiobook");
  if (parts.length <= 1 && chapters.length <= 1) return null;

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <IconButton size="sm" label={t("player.chapters")}>
          <List className="h-4 w-4" />
        </IconButton>
      </Popover.Trigger>
      <Panel className="max-h-80 w-80 overflow-y-auto p-1.5">
        <p className="px-2 py-1.5 text-xs font-semibold uppercase tracking-wide text-muted">
          {chapters.length > parts.length ? t("player.chapters") : t("player.parts")}
        </p>
        {queue.map((item, i) => {
          if (item.kind !== "audiobook") return null;
          return (
            <button
              key={`${item.fileId}-${i}`}
              onClick={() => playIndex(i)}
              className={cn(
                "flex w-full items-center gap-2 rounded-lg px-2 py-1.5 text-left text-sm transition-colors",
                i === queueIndex ? "bg-accent/10 text-accent" : "hover:bg-bg-alt"
              )}
            >
              <span className="w-6 shrink-0 text-right text-xs tabular-nums text-muted">{item.filePosition}</span>
              <span className="min-w-0 flex-1 truncate">{item.title}</span>
              {item.durationSecs != null && (
                <span className="shrink-0 text-xs tabular-nums text-muted">{formatClock(item.durationSecs)}</span>
              )}
            </button>
          );
        })}
      </Panel>
    </Popover.Root>
  );
}

// ── Bookmarks ─────────────────────────────────────────────────────────────

export function BookmarksPopover({
  bookId,
  fileId,
  positionInBook,
  onSeek,
}: {
  bookId: string;
  fileId: string;
  /** Position within the whole book, which is what a bookmark stores. */
  positionInBook: number;
  onSeek: (positionInBook: number) => void;
}) {
  const qc = useQueryClient();
  const [editing, setEditing] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const { t } = useT();

  const { data: bookmarks = [] } = useQuery({
    queryKey: ["book-bookmarks", bookId],
    queryFn: () => listBookBookmarks(bookId),
  });

  const invalidate = () => qc.invalidateQueries({ queryKey: ["book-bookmarks", bookId] });

  const add = useMutation({
    mutationFn: () => createBookmark({ bookId, fileId, positionSecs: positionInBook }),
    onSuccess: () => {
      invalidate();
      toast.success(t("player.bookmarks.added"), formatClock(positionInBook));
    },
    onError: () => toast.error(t("player.bookmarks.addFailed")),
  });

  const rename = useMutation({
    mutationFn: ({ id, text }: { id: string; text: string }) => updateBookmark(id, text || null),
    onSuccess: () => {
      invalidate();
      setEditing(null);
    },
  });

  const remove = useMutation({
    mutationFn: (id: string) => deleteBookmark(id),
    onSuccess: invalidate,
  });

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <IconButton size="sm" label={t("player.bookmarks.title")} active={bookmarks.length > 0}>
          <Bookmark className="h-4 w-4" />
        </IconButton>
      </Popover.Trigger>
      <Panel className="w-80">
        <div className="mb-2 flex items-center justify-between">
          <p className="text-xs font-semibold uppercase tracking-wide text-muted">{t("player.bookmarks.title")}</p>
          <Button size="sm" onClick={() => add.mutate()} loading={add.isPending}>
            {t("player.bookmarks.addAt", { time: formatClock(positionInBook) })}
          </Button>
        </div>

        {bookmarks.length === 0 && <p className="py-4 text-center text-sm text-muted">{t("player.bookmarks.empty")}</p>}

        <ul className="max-h-64 space-y-0.5 overflow-y-auto">
          {bookmarks.map((b: BookmarkResponse) => (
            <li key={b.id} className="group flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-bg-alt">
              {editing === b.id ? (
                <form
                  className="flex flex-1 gap-1"
                  onSubmit={(e) => {
                    e.preventDefault();
                    rename.mutate({ id: b.id, text: label.trim() });
                  }}
                >
                  <Input autoFocus value={label} onChange={(e) => setLabel(e.target.value)} className="h-7 text-xs" />
                  <Button size="sm" type="submit">
                    {t("common.action.save")}
                  </Button>
                </form>
              ) : (
                <>
                  <button onClick={() => onSeek(b.position_secs)} className="min-w-0 flex-1 text-left">
                    <span className="block truncate text-sm">{b.label || t("player.bookmarks.untitled")}</span>
                    <span className="block text-xs tabular-nums text-muted">{formatClock(b.position_secs)}</span>
                  </button>
                  <button
                    onClick={() => {
                      setEditing(b.id);
                      setLabel(b.label ?? "");
                    }}
                    className="text-xs text-muted opacity-0 transition-opacity hover:text-fg group-hover:opacity-100"
                  >
                    {t("common.action.rename")}
                  </button>
                  <IconButton size="sm" label={t("player.bookmarks.delete")} onClick={() => remove.mutate(b.id)}>
                    <Trash2 className="h-3.5 w-3.5" />
                  </IconButton>
                </>
              )}
            </li>
          ))}
        </ul>
      </Panel>
    </Popover.Root>
  );
}
