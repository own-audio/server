// SPDX-License-Identifier: AGPL-3.0-or-later
import { useLayoutEffect, useRef, useState } from "react";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/** A long description cut to a few lines, with More/Less only when it is actually cut. */
export function ClampedText({ text, lines = 4, className }: { text: string; lines?: 3 | 4; className?: string }) {
  const { t } = useT();
  const ref = useRef<HTMLParagraphElement>(null);
  const [open, setOpen] = useState(false);
  const [clipped, setClipped] = useState(false);
  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const measure = () => setClipped(el.scrollHeight > el.clientHeight + 1);
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, [text]);
  return (
    <div className={cn("max-w-2xl text-left", className)}>
      <p
        ref={ref}
        className={cn("whitespace-pre-line text-sm leading-relaxed text-muted", !open && (lines === 3 ? "line-clamp-3" : "line-clamp-4"))}
      >
        {text}
      </p>
      {(clipped || open) && (
        <button
          type="button"
          onClick={() => setOpen(!open)}
          className="mt-1 py-1 text-sm font-medium text-accent-text focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent pointer-coarse:py-2"
        >
          {open ? t("library.text.less") : t("library.text.more")}
        </button>
      )}
    </div>
  );
}
