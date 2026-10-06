// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { UserRound, Users } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../../components/ui/CloudIcon";
import { listPrivateItems } from "../../api/library";
import { listBooks, setBookVisibility } from "../../api/audiobooks";
import { setFeedVisibility } from "../../api/podcasts";
import { listTracks, setTrackVisibility } from "../../api/music";
import { Page } from "../../components/shell/SplitView";
import { FilterChips } from "../../components/library/BrowseControls";
import { MediaRow } from "../../components/library/MediaCard";
import { Button, EmptyState, IconButton, SearchField, Skeleton } from "../../components/ui";
import { toast } from "../../lib/toast";
import { runWithLimit } from "../../lib/uploadQueue";
import { playBook, playTracks } from "../../lib/play";
import { cn } from "../../lib/cn";
import type { PrivateItem, Visibility } from "../../api/types";
import { useT, type PlainKey } from "../../i18n";

/* Deliberately no padlock and no protection wording anywhere on this page.
   "Private" here means "not shared with the family" — it is not a security
   boundary, and the clients all say so the same way.

   What the page is for: seeing what the rest of the family does not see, and
   fixing that in a tap — one item, an album, or a whole kind at once. */

type Kind = PrivateItem["kind"];
type Filter = "all" | Kind;

const KIND: Record<Kind, { plural: PlainKey; icon: React.ReactNode; text: string; href: (i: PrivateItem) => string }> = {
  audiobook: { plural: "common.kind.audiobooks", icon: <BookIcon className="h-4 w-4" />, text: "text-book", href: (i) => `/audiobooks/${i.id}` },
  podcast: { plural: "common.kind.podcasts", icon: <PodcastIcon className="h-4 w-4" />, text: "text-podcast", href: (i) => `/podcasts/${i.id}` },
  /* There is no route for a single track, so a song points at its artist —
     a page that actually contains it, rather than the music root. */
  music: {
    plural: "common.kind.music",
    icon: <MusicIcon className="h-4 w-4" />,
    text: "text-music",
    href: (i) => (i.subtitle ? `/music/artists/${encodeURIComponent(i.subtitle)}` : "/music"),
  },
};

const ORDER: Kind[] = ["audiobook", "podcast", "music"];

const SET_VISIBILITY: Record<Kind, (id: string, v: Visibility) => Promise<void>> = {
  audiobook: setBookVisibility,
  podcast: setFeedVisibility,
  music: setTrackVisibility,
};

/** What each kind's own lists are cached under — they change when an item is shared. */
const LIST_KEYS: Record<Kind, string[]> = {
  audiobook: ["books"],
  podcast: ["feeds"],
  music: ["music-tracks"],
};

export default function PrivatePage() {
  const { t } = useT();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [filter, setFilter] = useState("");
  const [kind, setKind] = useState<Filter>("all");
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const { data: items, isLoading, isError } = useQuery({ queryKey: ["private-library"], queryFn: listPrivateItems });

  const hasBooks = !!items?.some((i) => i.kind === "audiobook");
  const hasMusic = !!items?.some((i) => i.kind === "music");
  // Only for the play buttons: playing needs the full objects the lists already cache.
  const { data: books = [] } = useQuery({ queryKey: ["books"], queryFn: listBooks, enabled: hasBooks });
  const { data: tracks = [] } = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks, enabled: hasMusic });

  const counts = useMemo(() => {
    const c: Record<Kind, number> = { audiobook: 0, podcast: 0, music: 0 };
    for (const i of items ?? []) c[i.kind] += 1;
    return c;
  }, [items]);

  const groups = useMemo(() => {
    const q = filter.trim().toLowerCase();
    const matching = (items ?? []).filter(
      (i) =>
        (kind === "all" || i.kind === kind) &&
        (!q || i.title.toLowerCase().includes(q) || (i.subtitle ?? "").toLowerCase().includes(q) || (i.album ?? "").toLowerCase().includes(q))
    );
    return ORDER.map((k) => ({ kind: k, rows: matching.filter((i) => i.kind === k) })).filter((g) => g.rows.length > 0);
  }, [items, filter, kind]);

  const total = items?.length ?? 0;

  /** Share these with the family; the toast's Undo makes them private again. */
  async function share(list: PrivateItem[]) {
    if (list.length === 0) return;
    const keys = list.map((i) => `${i.kind}:${i.id}`);
    setBusy((b) => new Set([...b, ...keys]));
    const refresh = () => {
      qc.invalidateQueries({ queryKey: ["private-library"] });
      for (const k of new Set(list.map((i) => i.kind))) qc.invalidateQueries({ queryKey: LIST_KEYS[k] });
    };
    const { failed } = await runWithLimit(list, 4, (i) => SET_VISIBILITY[i.kind](i.id, "family"));
    setBusy((b) => new Set([...b].filter((k) => !keys.includes(k))));
    refresh();
    const done = list.filter((i) => !failed.some((f) => f.item === i));
    if (failed.length > 0) toast.error(t("private.shareFailed"), failed[0].item.title);
    if (done.length === 0) return;
    toast.withAction(
      t("private.shared", { count: done.length }),
      {
        label: t("common.action.undo"),
        onClick: () => void runWithLimit(done, 4, (i) => SET_VISIBILITY[i.kind](i.id, "private")).then(refresh),
      },
      done.length === 1 ? done[0].title : undefined
    );
  }

  function playFor(item: PrivateItem): (() => void) | undefined {
    if (item.kind === "audiobook") {
      const book = books.find((b) => b.id === item.id);
      return book ? () => void playBook(book) : undefined;
    }
    if (item.kind === "music") {
      const track = tracks.find((tr) => tr.id === item.id);
      return track ? () => void playTracks([track], 0) : undefined;
    }
    return undefined;
  }

  const row = (item: PrivateItem) => (
    <MediaRow
      key={`${item.kind}-${item.id}`}
      kind={item.kind === "audiobook" ? "audiobook" : item.kind}
      title={item.title}
      subtitle={item.subtitle}
      cover={item.cover_url}
      coverAuth={item.kind !== "podcast"}
      onClick={() => navigate(KIND[item.kind].href(item))}
      onPlay={playFor(item)}
      action={
        <IconButton size="sm" label={t("private.share")} disabled={busy.has(`${item.kind}:${item.id}`)} onClick={() => void share([item])}>
          <Users className="h-4 w-4" />
        </IconButton>
      }
    />
  );

  /** Songs grouped by album: a hundred loose songs are hard to scan, and an album is what gets shared. */
  const musicRows = (rows: PrivateItem[]) => {
    const albums = new Map<string, PrivateItem[]>();
    for (const r of rows) {
      const key = r.album ?? "";
      if (!albums.has(key)) albums.set(key, []);
      albums.get(key)!.push(r);
    }
    return [...albums.entries()].map(([album, songs]) => (
      <div key={album || "-"} className="mb-2">
        <div className="flex items-center gap-2 px-2 pt-2">
          <p className="min-w-0 flex-1 truncate text-[13px] font-medium text-muted">{album || t("private.noAlbum")}</p>
          {songs.length > 1 && (
            <Button size="sm" variant="ghost" onClick={() => void share(songs)}>
              {t("private.shareAlbum")}
            </Button>
          )}
        </div>
        {songs.map(row)}
      </div>
    ));
  };

  return (
    <Page title={t("private.title")} width="max-w-3xl">
      <p className="-mt-3 mb-4 text-sm text-muted">{t("private.intro")}</p>

      {total > 0 && (
        <div className="mb-5 space-y-3">
          <SearchField value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t("private.find")} className="w-full" aria-label={t("private.find")} />
          <FilterChips<Filter>
            value={kind}
            onChange={setKind}
            chips={[
              { value: "all", label: `${t("private.all")} ${total}` },
              ...ORDER.filter((k) => counts[k] > 0).map((k) => ({ value: k, label: `${t(KIND[k].plural)} ${counts[k]}` })),
            ]}
          />
        </div>
      )}

      {isLoading ? (
        <div className="space-y-2">
          <Skeleton className="h-14" />
          <Skeleton className="h-14" />
          <Skeleton className="h-14" />
        </div>
      ) : isError ? (
        <EmptyState icon={<UserRound />} title={t("private.loadFailed.title")} description={t("private.loadFailed.description")} />
      ) : total === 0 ? (
        <EmptyState icon={<UserRound />} title={t("private.empty.title")} description={t("private.empty.description")} />
      ) : groups.length === 0 ? (
        <EmptyState icon={<UserRound />} title={t("private.noMatch.title")} description={t("private.noMatch.description")} />
      ) : (
        <div className="space-y-8">
          {groups.map(({ kind: k, rows }) => (
            <section key={k}>
              <div className="mb-1 flex items-center gap-2">
                <h2 className={cn("flex min-w-0 flex-1 items-center gap-2 text-xs font-semibold uppercase tracking-wider", KIND[k].text)}>
                  {KIND[k].icon}
                  {t(KIND[k].plural)}
                  <span className="text-muted">{rows.length}</span>
                </h2>
                <Button size="sm" variant="secondary" icon={<Users className="h-4 w-4" />} onClick={() => void share(rows)}>
                  {t("private.shareAll", { count: rows.length })}
                </Button>
              </div>
              <div className="-mx-2">{k === "music" ? musicRows(rows) : rows.map(row)}</div>
            </section>
          ))}
        </div>
      )}
    </Page>
  );
}
