// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ReactNode, MouseEvent } from "react";
import { Pause, Play } from "lucide-react";
import { Cover, type MediaKind } from "../ui/Cover";
import { FamilyBadge, FavoriteBadge, TranslatableBadge } from "../ui/Badge";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/*
 * Cards and rows carry three separate actions: open the thing, play it, and a
 * menu. The obvious shape — a clickable wrapper with buttons inside it — is
 * nested interactive content: a screen reader announces one control and hides
 * the rest, and the inner buttons can't be tabbed to.
 *
 * So the title is the real button — the thing keyboard and screen-reader users
 * reach — and the cover is a plain div with a click handler, which is a mouse
 * affordance ARIA never sees. Play and the menu are ordinary buttons inside
 * that inert div, so nothing is nested inside a control.
 */

export interface MediaCardProps {
  kind: MediaKind;
  title: string;
  subtitle?: string | null;
  meta?: string | null;
  cover?: string | null;
  coverAuth?: boolean;
  /** 0–1 */
  progress?: number | null;
  completed?: boolean;
  family?: boolean;
  favorite?: boolean;
  /** A podcast whose episodes can be translated. */
  translatable?: boolean;
  /** A person (an artist) rather than a release: the picture is a circle. */
  roundCover?: boolean;
  /** This item is what the player is on. */
  active?: boolean;
  playing?: boolean;
  selected?: boolean;
  onClick?: () => void;
  onPlay?: () => void;
  /** Slot for a context menu trigger, shown top-right on hover. */
  menu?: ReactNode;
  className?: string;
}

export function MediaCard({
  kind,
  title,
  subtitle,
  meta,
  cover,
  coverAuth,
  progress,
  completed,
  family,
  favorite,
  translatable,
  roundCover,
  active,
  playing,
  selected,
  onClick,
  onPlay,
  menu,
  className,
}: MediaCardProps) {
  const pct = progress != null ? Math.round(Math.min(1, Math.max(0, progress)) * 100) : null;
  const { t } = useT();

  return (
    <div
      className={cn(
        "group relative flex flex-col rounded-[14px] p-1.5 transition-colors",
        // A 10% tint drops muted subtitle text to 4.38:1 for every accent
        // colour, purple included. 6% clears AA, and the ring carries the
        // "selected" meaning without depending on the tint being noticed.
        selected ? "bg-accent/6 ring-1 ring-inset ring-accent/30" : "hover:bg-bg-alt",
        className
      )}
    >
      {/* Mouse affordance only: the accessible control is the title below. */}
      <div className={cn("relative", onClick && "cursor-pointer")} onClick={onClick}>
        <Cover
          kind={kind}
          src={cover}
          alt=""
          auth={coverAuth}
          round={roundCover}
          className={cn(
            "transition-transform duration-[var(--duration-cover)] group-hover:-translate-y-0.5 group-hover:shadow-card",
            selected && "ring-2 ring-accent"
          )}
        />

        {onPlay && (
          <button
            type="button"
            onClick={(e: MouseEvent) => {
              e.stopPropagation();
              onPlay();
            }}
            aria-label={playing ? t("library.card.pause", { title }) : t("library.card.play", { title })}
            className={cn(
              "absolute bottom-2 right-2 flex h-9 w-9 items-center justify-center rounded-pill bg-fg text-bg shadow-card transition-all",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
              // A touch screen has no hover to reveal it, so there it is always shown.
              active ? "opacity-100" : "opacity-0 translate-y-1 group-hover:translate-y-0 group-hover:opacity-100 focus-visible:translate-y-0 focus-visible:opacity-100 pointer-coarse:translate-y-0 pointer-coarse:opacity-100"
            )}
          >
            {playing ? <Pause className="h-4 w-4 fill-current" /> : <Play className="h-4 w-4 translate-x-px fill-current" />}
          </button>
        )}

        {(family || favorite || translatable) && (
          <div className="pointer-events-none absolute left-2 top-2 flex gap-1">
            {family && <FamilyBadge />}
            {favorite && <FavoriteBadge />}
            {translatable && <TranslatableBadge />}
          </div>
        )}

        {menu && (
          <div className="absolute right-2 top-2 opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100 pointer-coarse:opacity-100">
            {menu}
          </div>
        )}

        {pct != null && pct > 0 && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-1 bg-bg/50">
            <div className={cn("h-full", completed ? "bg-success" : "bg-accent")} style={{ width: `${completed ? 100 : pct}%` }} />
          </div>
        )}
      </div>

      <div className="mt-2 min-w-0 px-0.5">
        <button
          type="button"
          onClick={onClick}
          disabled={!onClick}
          title={title}
          className="block w-full min-w-0 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:cursor-default"
        >
          <span className={cn("block truncate text-[13px] font-medium leading-tight", active && "text-accent")}>{title}</span>
          {subtitle && <span className="mt-0.5 block truncate text-xs text-muted">{subtitle}</span>}
          {meta && <span className="mt-0.5 block truncate text-[11px] text-muted">{meta}</span>}
        </button>
      </div>
    </div>
  );
}

export function MediaGrid({ children, dense, className }: { children: ReactNode; dense?: boolean; className?: string }) {
  return (
    <div
      className={cn(
        "grid gap-2 p-3",
        dense ? "grid-cols-[repeat(auto-fill,minmax(120px,1fr))]" : "grid-cols-[repeat(auto-fill,minmax(140px,1fr))]",
        className
      )}
    >
      {children}
    </div>
  );
}

export interface MediaRowProps {
  kind: MediaKind;
  title: string;
  subtitle?: string | null;
  trailing?: ReactNode;
  cover?: string | null;
  coverAuth?: boolean;
  progress?: number | null;
  completed?: boolean;
  family?: boolean;
  favorite?: boolean;
  translatable?: boolean;
  roundCover?: boolean;
  active?: boolean;
  playing?: boolean;
  selected?: boolean;
  onClick?: () => void;
  onPlay?: () => void;
  menu?: ReactNode;
  /** A control that is always shown at the row's end (a menu hides until hover on a mouse). */
  action?: ReactNode;
  index?: number;
}

export function MediaRow({
  kind,
  title,
  subtitle,
  trailing,
  cover,
  coverAuth,
  progress,
  completed,
  family,
  favorite,
  translatable,
  roundCover,
  active,
  playing,
  selected,
  onClick,
  onPlay,
  menu,
  action,
  index,
}: MediaRowProps) {
  const pct = progress != null ? Math.round(Math.min(1, Math.max(0, progress)) * 100) : null;
  const { t } = useT();

  return (
    <div
      className={cn(
        "group flex items-center gap-3 rounded-[10px] px-2 py-1.5 transition-colors",
        selected ? "bg-accent/6 ring-1 ring-inset ring-accent/30" : "hover:bg-bg-alt"
      )}
    >
      {index != null && <span className="w-6 shrink-0 text-right text-xs tabular-nums text-muted">{index}</span>}

      <div className="relative shrink-0">
        <Cover kind={kind} src={cover} alt="" auth={coverAuth} aspect="square" round={roundCover} className={cn("h-10 w-10", !roundCover && "rounded-md")} />
        {onPlay && (
          <button
            type="button"
            onClick={onPlay}
            aria-label={playing ? t("library.card.pause", { title }) : t("library.card.play", { title })}
            className={cn(
              "absolute inset-0 flex items-center justify-center rounded-md bg-black/60 text-white transition-opacity",
              "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent",
              active ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100"
            )}
          >
            {playing ? <Pause className="h-4 w-4 fill-current" /> : <Play className="h-4 w-4 fill-current" />}
          </button>
        )}
      </div>

      <button
        type="button"
        onClick={onClick}
        onDoubleClick={onPlay}
        disabled={!onClick}
        className="min-w-0 flex-1 rounded-md text-left focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent disabled:cursor-default"
      >
        <span className={cn("block truncate text-sm font-medium leading-tight", active && "text-accent")}>{title}</span>
        {subtitle && <span className="block truncate text-xs text-muted">{subtitle}</span>}
        {/* On a phone the trailing text would squeeze the title to a few letters, so it goes under it. */}
        {trailing && <span className="block truncate text-xs tabular-nums text-muted sm:hidden">{trailing}</span>}
        {pct != null && pct > 0 && (
          <span className="mt-1 block h-0.5 w-32 rounded-pill bg-border">
            <span className={cn("block h-full rounded-pill", completed ? "bg-success" : "bg-accent")} style={{ width: `${completed ? 100 : pct}%` }} />
          </span>
        )}
      </button>

      <div className="flex shrink-0 items-center gap-1.5">
        {family && <FamilyBadge className="bg-bg-alt" />}
        {favorite && <FavoriteBadge className="bg-bg-alt" />}
        {translatable && <TranslatableBadge className="bg-bg-alt" />}
        {trailing && <span className="text-xs tabular-nums text-muted max-sm:hidden">{trailing}</span>}
        {menu && (
          <span className="opacity-0 transition-opacity focus-within:opacity-100 group-hover:opacity-100 data-[state=open]:opacity-100 pointer-coarse:opacity-100">{menu}</span>
        )}
        {action}
      </div>
    </div>
  );
}
