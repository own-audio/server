// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type PointerEvent, type RefObject } from "react";
import { cn } from "../../lib/cn";
import type { ColumnWidthState } from "../../lib/columnWidth";
import { useT } from "../../i18n";

/** Arrow-key nudge, and the step the grip reports to assistive tech. */
const KEY_STEP = 16;

/**
 * The drag handle between a column and what follows it. Drag, or focus and
 * use the arrow keys; a double-click restores the default. `columnRef` is the
 * column it sizes: the width is measured from that column's left edge.
 */
export default function ResizeGrip({
  column,
  columnRef,
  className,
}: {
  column: ColumnWidthState;
  columnRef: RefObject<HTMLElement | null>;
  className?: string;
}) {
  const { width, setWidth, initial, min, max } = column;
  const [dragging, setDragging] = useState(false);
  const { t } = useT();

  const onPointerDown = (e: PointerEvent<HTMLDivElement>) => {
    e.preventDefault();
    const grip = e.currentTarget;
    grip.setPointerCapture(e.pointerId);
    setDragging(true);
    const left = columnRef.current?.getBoundingClientRect().left ?? 0;
    // Track the pointer rather than accumulating deltas: a drag that runs
    // past the limit and comes back stays under the cursor either way.
    const move = (ev: globalThis.PointerEvent) => setWidth(ev.clientX - left);
    const up = () => {
      setDragging(false);
      grip.removeEventListener("pointermove", move);
      grip.removeEventListener("pointerup", up);
      grip.removeEventListener("pointercancel", up);
    };
    grip.addEventListener("pointermove", move);
    grip.addEventListener("pointerup", up);
    grip.addEventListener("pointercancel", up);
  };

  return (
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label={t("shell.split.resize")}
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={onPointerDown}
      onDoubleClick={() => setWidth(initial)}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft") setWidth(width - KEY_STEP);
        else if (e.key === "ArrowRight") setWidth(width + KEY_STEP);
        else if (e.key === "Home") setWidth(min);
        else if (e.key === "End") setWidth(max);
        else return;
        e.preventDefault();
      }}
      className={cn(
        "relative w-1 shrink-0 cursor-col-resize touch-none",
        "after:absolute after:inset-y-0 after:-left-1 after:-right-1 after:content-['']",
        "hover:bg-accent/50 focus-visible:bg-accent focus-visible:outline-none",
        dragging && "bg-accent",
        className
      )}
    />
  );
}
