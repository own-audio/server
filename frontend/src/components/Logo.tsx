// SPDX-License-Identifier: AGPL-3.0-or-later
import { cn } from "../lib/cn";

/**
 * The own.audio mark — the animated one from `audio2-www/brand/brand.svg`:
 * a pulsing core, two rotating acoustic arcs, and five family members
 * waltzing around the middle.
 *
 * Two deliberate differences from that file:
 *
 * - **The tuned member colours, not raw RGB primaries.** `brand.svg` still
 *   carries `#FF0000` / `#FFFF00` / `#00FF00`; `audio2-www/src/lib/mark.ts` is
 *   the declared source of truth (favicons, app icons and the site itself all
 *   render from it) and says in as many words: never raw primaries. These are
 *   its values, and they match the brand brief's palette table.
 * - **Theme-aware arcs.** `brand.svg` hardcodes `#E5E5E7`, which disappears on
 *   a light background. They use `--mark-arc`, which is a step stronger than
 *   the hairline border token — at 30px the border value vanished entirely,
 *   and the arcs are half the design.
 */

/** Clockwise from the top. */
const FAMILY = ["#FABB05", "#29ABE2", "#FF3B30", "#FFCC00", "#34C759"] as const;
const CORE = "#6E44FF";

/** One member: a head and two shoulders, rotated into place. */
function Member({ angle, fill }: { angle: number; fill: string }) {
  return (
    <g transform={`rotate(${angle}, 50, 50)`}>
      <circle cx="50" cy="14" r="5" fill={fill} />
      <circle cx="41.5" cy="15" r="2.5" fill={fill} />
      <circle cx="58.5" cy="15" r="2.5" fill={fill} />
    </g>
  );
}

export interface LogoProps {
  size?: number;
  /** `auto` drops the arcs below 26px, where they turn to mud. */
  variant?: "auto" | "detailed" | "compact";
  /** Motion is on by default; `prefers-reduced-motion` still wins. */
  animated?: boolean;
  className?: string;
}

export default function Logo({ size = 28, variant = "auto", animated = true, className }: LogoProps) {
  const detailed = variant === "detailed" || (variant === "auto" && size >= 26);

  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 100 100"
      fill="none"
      role="img"
      aria-label="own.audio"
      className={cn("own-mark", animated && "own-mark--animated", className)}
    >
      <circle cx="50" cy="50" r="11" fill={CORE} className="mark-core" />

      {detailed && (
        <g className="mark-arcs">
          <path d="M 35 62 A 20 20 0 0 0 65 62" stroke="var(--mark-arc)" strokeWidth="3.5" strokeLinecap="round" />
          <path d="M 28 70 A 30 30 0 0 0 72 70" stroke="var(--mark-arc)" strokeWidth="2.5" strokeLinecap="round" opacity="0.6" />
        </g>
      )}

      <g className="mark-family">
        {FAMILY.map((fill, i) => (
          <Member key={fill} angle={i * 72} fill={fill} />
        ))}
      </g>
    </svg>
  );
}

export function Wordmark({ className }: { className?: string }) {
  return (
    <span className={className} aria-hidden="true">
      <span className="font-semibold tracking-tight">own</span>
      <span className="text-accent font-semibold">.</span>
      <span className="font-semibold tracking-tight">audio</span>
    </span>
  );
}
