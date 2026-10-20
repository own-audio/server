// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState, type ReactNode } from "react";

/**
 * A long list rendered a part at a time, the next part as the end comes near.
 * 9,000 songs rendered at once kept the browser busy for seconds and fetched
 * every cover; this renders what can be seen and a little more. Give it a
 * `key` that changes with the filter or sort, so a new list starts at the top.
 */
export default function Progressive<T>({ items, render, step = 200 }: { items: T[]; render: (item: T, index: number) => ReactNode; step?: number }) {
  const [count, setCount] = useState(step);
  const end = useRef<HTMLDivElement>(null);
  const more = count < items.length;

  useEffect(() => {
    const el = end.current;
    if (!el || !more) return;
    // Observed again after every part: if the end is still in reach, the next
    // part follows at once.
    const io = new IntersectionObserver(([e]) => e.isIntersecting && setCount((c) => c + step), { rootMargin: "1500px" });
    io.observe(el);
    return () => io.disconnect();
  }, [count, more, step]);

  return (
    <>
      {items.slice(0, count).map(render)}
      {more && <div ref={end} aria-hidden className="h-px w-full" />}
    </>
  );
}
