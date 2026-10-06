// SPDX-License-Identifier: AGPL-3.0-or-later
import { useRef, useState, type MouseEvent } from "react";
import { formatClock } from "../../lib/time";
import { useT } from "../../i18n";

export default function SeekBar({
  currentTime,
  duration,
  onSeek,
}: {
  currentTime: number;
  duration: number;
  onSeek: (t: number) => void;
}) {
  const barRef = useRef<HTMLDivElement>(null);
  const { t } = useT();
  const [hover, setHover] = useState<{ x: number; t: number } | null>(null);
  const pct = duration > 0 ? (currentTime / duration) * 100 : 0;

  function posFrom(e: MouseEvent): { x: number; t: number } | null {
    const rect = barRef.current?.getBoundingClientRect();
    if (!rect || duration <= 0) return null;
    const x = Math.max(0, Math.min(e.clientX - rect.left, rect.width));
    return { x, t: (x / rect.width) * duration };
  }

  return (
    <div
      ref={barRef}
      role="slider"
      tabIndex={0}
      aria-label={t("player.seek")}
      aria-valuemin={0}
      aria-valuemax={Math.round(duration) || 0}
      aria-valuenow={Math.round(currentTime)}
      aria-valuetext={formatClock(currentTime)}
      className="group/seek relative h-1 w-full cursor-pointer bg-border transition-[height] hover:h-2 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
      onMouseMove={(e) => setHover(posFrom(e))}
      onMouseLeave={() => setHover(null)}
      onClick={(e) => {
        const p = posFrom(e);
        if (p) onSeek(p.t);
      }}
      onKeyDown={(e) => {
        if (e.key === "ArrowRight") onSeek(currentTime + 10);
        if (e.key === "ArrowLeft") onSeek(currentTime - 10);
      }}
    >
      <div className="absolute left-0 top-0 h-full bg-accent transition-[width] duration-100" style={{ width: `${pct}%` }} />
      {hover && (
        <div
          className="pointer-events-none absolute -top-8 rounded-md bg-fg px-2 py-0.5 text-xs tabular-nums text-bg shadow-pop"
          style={{ left: hover.x, transform: "translateX(-50%)" }}
        >
          {formatClock(hover.t)}
        </div>
      )}
    </div>
  );
}
