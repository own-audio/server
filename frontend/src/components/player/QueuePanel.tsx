// SPDX-License-Identifier: AGPL-3.0-or-later
import { X } from "lucide-react";
import { usePlayerStore, type PlayerTrack } from "../../store/playerStore";
import { IconButton } from "../ui/Button";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

function subtitleFor(item: PlayerTrack): string | null {
  if (item.kind === "podcast") return item.feedTitle;
  if (item.kind === "music") return item.artist;
  return item.bookTitle;
}

function keyFor(item: PlayerTrack): string {
  if (item.kind === "podcast") return item.epId;
  if (item.kind === "audiobook") return `${item.bookId}:${item.fileId}`;
  return item.trackId;
}

export default function QueuePanel() {
  const { queue, queueIndex, track, showQueue, setShowQueue, removeFromQueue, clearQueue, playIndex } = usePlayerStore();
  const { t } = useT();
  if (!showQueue || !track) return null;

  return (
    <div className="absolute bottom-full right-4 z-50 mb-2 max-h-96 w-80 overflow-y-auto rounded-card border border-border bg-card shadow-pop">
      <div className="sticky top-0 flex items-center justify-between border-b border-border bg-card px-4 py-2.5">
        <span className="text-sm font-semibold">{t("player.queue.title")}</span>
        <div className="flex items-center gap-1">
          <button onClick={clearQueue} className="text-xs text-muted hover:text-fg">
            {t("common.action.clear")}
          </button>
          <IconButton size="sm" label={t("player.queue.close")} onClick={() => setShowQueue(false)}>
            <X className="h-4 w-4" />
          </IconButton>
        </div>
      </div>

      {queue.length === 0 && <p className="px-4 py-6 text-center text-xs text-muted">{t("player.queue.empty")}</p>}

      {queue.map((item, i) => (
        <div
          key={`${keyFor(item)}-${i}`}
          className={cn("flex items-center gap-3 px-4 py-2 text-sm", i === queueIndex ? "text-accent" : "text-fg hover:bg-bg-alt")}
        >
          <span className="w-5 shrink-0 text-right text-xs tabular-nums text-muted">{i === queueIndex ? "▶" : i + 1}</span>
          <button
            type="button"
            onClick={() => playIndex(i)}
            className="min-w-0 flex-1 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
          >
            <span className="block truncate text-xs font-medium leading-tight">{item.title}</span>
            <span className="block truncate text-xs text-muted">{subtitleFor(item)}</span>
          </button>
          {i !== queueIndex && (
            <IconButton size="sm" label={t("player.queue.remove", { title: item.title })} onClick={() => removeFromQueue(i)}>
              <X className="h-3.5 w-3.5" />
            </IconButton>
          )}
        </div>
      ))}
    </div>
  );
}
