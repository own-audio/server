// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import * as D from "@radix-ui/react-dialog";
import { Search } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../ui/CloudIcon";
import { useQuery } from "@tanstack/react-query";
import { search } from "../../api/library";
import type { SearchResult } from "../../api/types";
import { useDebounce } from "../../lib/useDebounce";
import { cn } from "../../lib/cn";
import { useCommandPalette } from "../../lib/commandPalette";
import { useT, type PlainKey } from "../../i18n";

function resultPath(r: SearchResult): string {
  switch (r.kind) {
    case "Feed":
      return `/podcasts/${r.id}`;
    case "Episode":
      return `/podcasts/${r.feed_id}`;
    case "Book":
      return `/audiobooks/${r.id}`;
    case "Track":
      return r.artist ? `/music/artists/${encodeURIComponent(r.artist)}` : "/music";
  }
}

const kindIcon = { Feed: PodcastIcon, Episode: PodcastIcon, Book: BookIcon, Track: MusicIcon } as const;
/** Group headings. */
const kindLabel: Record<SearchResult["kind"], PlainKey> = {
  Feed: "common.kind.podcasts",
  Episode: "shell.search.episodes",
  Book: "common.kind.audiobooks",
  Track: "shell.search.songs",
};

/* Global search over /library/search, opened with Cmd/Ctrl+K anywhere. */
export default function CommandPalette() {
  const { open, setOpen } = useCommandPalette();
  const navigate = useNavigate();
  const { t } = useT();
  const [q, setQ] = useState("");
  const [cursor, setCursor] = useState(0);
  const debounced = useDebounce(q.trim(), 200);

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen(!useCommandPalette.getState().open);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  const { data: results = [], isFetching } = useQuery({
    queryKey: ["search", debounced],
    queryFn: () => search(debounced, 30),
    enabled: open && debounced.length >= 2,
  });

  const grouped = useMemo(() => {
    const order: SearchResult["kind"][] = ["Book", "Feed", "Episode", "Track"];
    return order
      .map((k) => ({ kind: k, items: results.filter((r) => r.kind === k) }))
      .filter((g) => g.items.length > 0);
  }, [results]);

  const flat = useMemo(() => grouped.flatMap((g) => g.items), [grouped]);

  function go(r: SearchResult) {
    setOpen(false);
    setQ("");
    navigate(resultPath(r));
  }

  return (
    <D.Root
      open={open}
      onOpenChange={(v) => {
        setOpen(v);
        if (!v) setQ("");
      }}
    >
      <D.Portal>
        <D.Overlay className="fixed inset-0 z-50 bg-overlay data-[state=open]:animate-fade-in data-[state=closed]:animate-fade-out" />
        <D.Content
          className="fixed left-1/2 top-[12vh] z-50 w-[min(640px,calc(100vw-2rem))] -translate-x-1/2 overflow-hidden rounded-sheet border border-border bg-card shadow-pop outline-none origin-top data-[state=open]:animate-pop-in data-[state=closed]:animate-pop-out"
          onKeyDown={(e) => {
            if (e.key === "ArrowDown") {
              e.preventDefault();
              setCursor((c) => Math.min(flat.length - 1, c + 1));
            }
            if (e.key === "ArrowUp") {
              e.preventDefault();
              setCursor((c) => Math.max(0, c - 1));
            }
            if (e.key === "Enter" && flat[cursor]) {
              e.preventDefault();
              go(flat[cursor]);
            }
          }}
        >
          <D.Title className="sr-only">{t("shell.search.title")}</D.Title>
          <D.Description className="sr-only">{t("shell.search.description")}</D.Description>

          <div className="flex items-center gap-3 border-b border-border px-4">
            <Search className="h-5 w-5 shrink-0 text-muted" />
            <input
              autoFocus
              value={q}
              onChange={(e) => { setQ(e.target.value); setCursor(0); }}
              placeholder={t("shell.search.placeholder")}
              className="h-14 w-full bg-transparent text-base text-fg outline-none placeholder:text-muted"
            />
            <kbd className="hidden rounded-md border border-border px-1.5 py-0.5 text-[11px] text-muted sm:inline">esc</kbd>
          </div>

          <div className="max-h-[50vh] overflow-y-auto p-2">
            {debounced.length < 2 && (
              <p className="px-3 py-6 text-center text-sm text-muted">{t("shell.search.minChars")}</p>
            )}
            {debounced.length >= 2 && !isFetching && flat.length === 0 && (
              <p className="px-3 py-6 text-center text-sm text-muted">{t("shell.search.noResults")}</p>
            )}
            {grouped.map((g) => (
              <div key={g.kind} className="mb-1">
                <p className="px-3 pb-1 pt-2 text-[11px] font-semibold uppercase tracking-wider text-muted">
                  {t(kindLabel[g.kind])}
                </p>
                {g.items.map((r) => {
                  const idx = flat.indexOf(r);
                  const Icon = kindIcon[r.kind];
                  const sub = r.kind === "Episode" ? r.feed_title : r.kind === "Track" ? r.artist : r.author;
                  return (
                    <button
                      key={`${r.kind}-${r.id}`}
                      onMouseEnter={() => setCursor(idx)}
                      onClick={() => go(r)}
                      className={cn(
                        "flex w-full items-center gap-3 rounded-[10px] px-3 py-2 text-left",
                        idx === cursor && "bg-bg-alt"
                      )}
                    >
                      <Icon className="h-4 w-4 shrink-0 text-muted" />
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-sm font-medium">{r.title}</span>
                        {sub && <span className="block truncate text-xs text-muted">{sub}</span>}
                      </span>
                    </button>
                  );
                })}
              </div>
            ))}
          </div>
        </D.Content>
      </D.Portal>
    </D.Root>
  );
}
