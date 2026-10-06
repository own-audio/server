// SPDX-License-Identifier: AGPL-3.0-or-later
import { cn } from "../../lib/cn";

/**
 * Three bars that bounce while something is playing and rest flat when it is
 * paused — the same signal the Mac app puts next to the section that owns the
 * current item, so you can tell at a glance where the sound is coming from.
 */
export default function NowPlayingGlyph({ playing, className }: { playing: boolean; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn("flex h-3 w-3 shrink-0 items-end justify-between", !playing && "eq--paused", className)}
    >
      {[0, 1, 2].map((i) => (
        <span key={i} className="eq-bar w-[3px] rounded-sm bg-current" style={{ height: "100%" }} />
      ))}
    </span>
  );
}
