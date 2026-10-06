// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useLayoutEffect, useRef, useState, type CSSProperties, type PointerEvent, type ReactNode } from "react";
import { ArrowLeft } from "lucide-react";
import { useNavigate } from "react-router-dom";
import { cn } from "../../lib/cn";
import { IconButton } from "../ui/Button";
import { usePersistedState } from "../../lib/persistedState";
import { useT } from "../../i18n";

/* The product's navigation shape: a content column (collections) and a detail
   column (the one selected thing). On wide screens both show; below the
   breakpoint the detail column replaces the content column while something
   is selected, with a back control. Pages never push routes for drill-down. */

const DEFAULT_WIDTH = { narrow: 340, medium: 400, wide: 460 } as const;
export type ColumnWidth = keyof typeof DEFAULT_WIDTH;

const MIN_WIDTH = 260;
const MAX_WIDTH = 720;
/** Arrow-key nudge, and the step the grip reports to assistive tech. */
const KEY_STEP = 16;

const clamp = (n: number) => Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Math.round(n)));

export function SplitView({
  content,
  detail,
  hasDetail,
  onBack,
  contentWidth = "medium",
  /** Widths are remembered per section, so Music and Audiobooks can differ. */
  widthKey,
  className,
}: {
  content: ReactNode;
  detail: ReactNode;
  hasDetail: boolean;
  onBack?: () => void;
  contentWidth?: ColumnWidth;
  widthKey?: string;
  className?: string;
}) {
  const [width, setWidth] = usePersistedState<number>(
    `own-audio-col-${widthKey ?? contentWidth}`,
    DEFAULT_WIDTH[contentWidth]
  );
  const [dragging, setDragging] = useState(false);
  const { t } = useT();
  const columnRef = useRef<HTMLElement>(null);

  const onPointerDown = useCallback(
    (e: PointerEvent<HTMLDivElement>) => {
      e.preventDefault();
      const grip = e.currentTarget;
      grip.setPointerCapture(e.pointerId);
      setDragging(true);

      const left = columnRef.current?.getBoundingClientRect().left ?? 0;
      // Track the pointer rather than accumulating deltas: a drag that runs
      // past the clamp and comes back stays under the cursor either way.
      const move = (ev: globalThis.PointerEvent) => setWidth(clamp(ev.clientX - left));
      const up = () => {
        setDragging(false);
        grip.removeEventListener("pointermove", move);
        grip.removeEventListener("pointerup", up);
        grip.removeEventListener("pointercancel", up);
      };
      grip.addEventListener("pointermove", move);
      grip.addEventListener("pointerup", up);
      grip.addEventListener("pointercancel", up);
    },
    [setWidth]
  );

  return (
    <div className={cn("flex h-full min-h-0", className)}>
      <section
        ref={columnRef}
        // The width is a CSS variable, not a generated class: Tailwind scans
        // source text, so `lg:w-[...]` built at runtime produces no CSS.
        style={{ "--content-w": `${width}px` } as CSSProperties}
        className={cn(
          "scroll-subtle min-h-0 shrink-0 overflow-y-auto border-r border-border",
          "w-full lg:w-[var(--content-w)]",
          hasDetail && "hidden lg:block"
        )}
      >
        {content}
      </section>

      {/* Drag, or focus and use the arrow keys. Double-click restores the
          section's default. Hidden below `lg`, where there is only one column. */}
      <div
        role="separator"
        aria-orientation="vertical"
        aria-label={t("shell.split.resize")}
        aria-valuenow={width}
        aria-valuemin={MIN_WIDTH}
        aria-valuemax={MAX_WIDTH}
        tabIndex={0}
        onPointerDown={onPointerDown}
        onDoubleClick={() => setWidth(DEFAULT_WIDTH[contentWidth])}
        onKeyDown={(e) => {
          if (e.key === "ArrowLeft") setWidth(clamp(width - KEY_STEP));
          else if (e.key === "ArrowRight") setWidth(clamp(width + KEY_STEP));
          else if (e.key === "Home") setWidth(MIN_WIDTH);
          else if (e.key === "End") setWidth(MAX_WIDTH);
          else return;
          e.preventDefault();
        }}
        className={cn(
          "relative -ml-px hidden w-1 shrink-0 cursor-col-resize touch-none lg:block",
          "after:absolute after:inset-y-0 after:-left-1 after:-right-1 after:content-['']",
          "hover:bg-accent/50 focus-visible:bg-accent focus-visible:outline-none",
          dragging && "bg-accent",
          hasDetail ? "" : "lg:hidden"
        )}
      />

      <section className={cn("scroll-subtle relative min-h-0 min-w-0 flex-1 overflow-y-auto", !hasDetail && "hidden lg:block")}>
        {hasDetail && onBack && (
          <div className="sticky top-0 z-10 flex h-11 items-center bg-bg/80 px-2 backdrop-blur lg:hidden">
            <IconButton label={t("common.action.back")} onClick={onBack}>
              <ArrowLeft className="h-5 w-5" />
            </IconButton>
          </div>
        )}
        {detail}
      </section>
    </div>
  );
}

/** Sticky header row for a column: title on the left, controls on the right. */
/**
 * With `actions`, the header stacks: the title and its actions on one line,
 * then `children` as full-width rows (filter field, chips) — the shape that
 * fits a phone. Without it, controls wrap beside the title as before.
 *
 * It is sticky from `sm` up and scrolls away on a phone, where its rows would
 * cover a fifth of the screen. Its height is published as
 * `--column-header-h` so section headers can stick just below it.
 */
export function ColumnHeader({ title, actions, children, className }: { title: ReactNode; actions?: ReactNode; children?: ReactNode; className?: string }) {
  const ref = useRef<HTMLElement>(null);
  useLayoutEffect(() => {
    const el = ref.current;
    const column = el?.parentElement;
    if (!el || !column) return;
    const ro = new ResizeObserver(() => column.style.setProperty("--column-header-h", `${el.offsetHeight}px`));
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  return (
    <header
      ref={ref}
      className={cn(
        "top-0 z-10 flex min-h-14 flex-wrap items-center gap-3 border-b border-border bg-bg/80 px-5 py-2.5 backdrop-blur",
        actions ? "gap-y-2.5 max-sm:static sm:sticky" : "sticky",
        className
      )}
    >
      <h1 className="text-xl font-semibold tracking-tight">{title}</h1>
      {actions ? (
        <>
          <div className="ml-auto flex items-center gap-1.5">{actions}</div>
          {children && <div className="flex min-w-0 basis-full flex-col gap-2.5">{children}</div>}
        </>
      ) : (
        <div className="ml-auto flex flex-wrap items-center gap-2">{children}</div>
      )}
    </header>
  );
}

/**
 * Plain page container for screens without a split (Home, Settings).
 *
 * `back` is for a page reached from another page rather than from the sidebar (Duplicates from
 * Music, Playback from Settings): the sidebar highlights its parent, so without the arrow there
 * was no visible way back on a phone, where the sidebar is hidden.
 */
export function Page({
  title,
  actions,
  back,
  onBack,
  children,
  width = "max-w-5xl",
}: {
  title?: ReactNode;
  actions?: ReactNode;
  /** Where the back arrow goes. */
  back?: string;
  /** Instead of `back`, for a page with steps of its own: the arrow goes one step back. */
  onBack?: () => void;
  children: ReactNode;
  width?: string;
}) {
  const navigate = useNavigate();
  const { t } = useT();
  return (
    <div className="scroll-subtle h-full overflow-y-auto">
      <div className={cn("mx-auto px-4 py-6 sm:px-6", width)}>
        {(title || actions) && (
          <div className="mb-6 flex flex-wrap items-center gap-3">
            {(back || onBack) && (
              <IconButton label={t("common.action.back")} onClick={() => (onBack ? onBack() : navigate(back!))} className="-ml-2">
                <ArrowLeft className="h-5 w-5" />
              </IconButton>
            )}
            {title && <h1 className="text-2xl font-semibold tracking-tight">{title}</h1>}
            {actions && <div className="ml-auto flex items-center gap-2">{actions}</div>}
          </div>
        )}
        {children}
      </div>
    </div>
  );
}
