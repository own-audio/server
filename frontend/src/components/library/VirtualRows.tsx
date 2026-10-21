// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useLayoutEffect, useRef, useState, type ReactNode } from "react";

/**
 * A long list of rows of one height, of which only those on screen (and a
 * screen's worth around them) exist in the page. Rendering a part at a time
 * (`Progressive`) kept every row it had rendered: after scrolling 9,000 songs,
 * 6,400 rows and over 500 MB. Here the page holds a few dozen rows however far
 * you scroll.
 *
 * The rows' height is measured from the first one; the list scrolls with
 * whatever scrolling container it sits in. Each row is a list item that says
 * where it is in the whole list, for screen readers.
 */
export default function VirtualRows<T>({ items, render, keyOf, label }: { items: T[]; render: (item: T, index: number) => ReactNode; keyOf: (item: T, index: number) => string; label?: string }) {
  const box = useRef<HTMLDivElement>(null);
  const [rowHeight, setRowHeight] = useState(52);
  const [range, setRange] = useState({ start: 0, end: 40 });

  // The nearest ancestor that scrolls, or the page.
  const scroller = useRef<HTMLElement | null>(null);

  useLayoutEffect(() => {
    let el = box.current?.parentElement ?? null;
    while (el && !/(auto|scroll)/.test(getComputedStyle(el).overflowY)) el = el.parentElement;
    scroller.current = el;
  }, []);

  useEffect(() => {
    const el = box.current;
    if (!el) return;
    let frame = 0;
    const update = () => {
      frame = 0;
      const view = scroller.current;
      const top = el.getBoundingClientRect().top - (view ? view.getBoundingClientRect().top : 0);
      const height = view ? view.clientHeight : window.innerHeight;
      // One screen above and below stays rendered, so a fast flick shows rows, not gaps.
      const first = Math.max(0, Math.floor(-top / rowHeight) - Math.ceil(height / rowHeight));
      const last = Math.min(items.length, Math.ceil((height - top) / rowHeight) + Math.ceil(height / rowHeight));
      setRange((r) => (r.start === first && r.end === last ? r : { start: first, end: last }));
    };
    const schedule = () => {
      if (!frame) frame = requestAnimationFrame(update);
    };
    update();
    const target: HTMLElement | Window = scroller.current ?? window;
    target.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      target.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [items.length, rowHeight]);

  // The real height of a row, from the first one rendered.
  const measure = useCallback((el: HTMLDivElement | null) => {
    const h = el?.offsetHeight;
    if (h) setRowHeight((old) => (Math.abs(h - old) > 0.5 ? h : old));
  }, []);

  const start = Math.min(range.start, items.length);
  const end = Math.min(range.end, items.length);
  return (
    <div ref={box} role="list" aria-label={label} style={{ paddingTop: start * rowHeight, paddingBottom: (items.length - end) * rowHeight }}>
      {items.slice(start, end).map((item, i) => (
        <div key={keyOf(item, start + i)} role="listitem" aria-setsize={items.length} aria-posinset={start + i + 1} ref={i === 0 ? measure : undefined}>
          {render(item, start + i)}
        </div>
      ))}
    </div>
  );
}
