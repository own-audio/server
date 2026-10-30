// SPDX-License-Identifier: AGPL-3.0-or-later
import type { ArtistAlbum } from "./albumGroups";
import { MediaRow } from "../../components/library/MediaCard";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/**
 * The middle column of artist → albums → songs: an artist's albums, "All songs"
 * first. A row of chips on a phone, a column beside the songs on a wide screen.
 */
export default function ArtistAlbums({ albums, total, picked, onPick }: { albums: ArtistAlbum[]; total: number; picked: string | null; onPick: (key: string | null) => void }) {
  const { t } = useT();
  const chip = (key: string | null, label: string) => (
    <button
      key={key ?? "all"}
      type="button"
      aria-current={picked === key || undefined}
      onClick={() => onPick(key)}
      className={cn(
        "shrink-0 rounded-pill border px-3 py-1.5 text-sm",
        picked === key ? "border-accent bg-accent/10 text-fg" : "border-border text-muted hover:text-fg",
      )}
    >
      {label}
    </button>
  );
  const row = (a: ArtistAlbum) => (
    <MediaRow
      key={a.key}
      kind="music"
      title={a.title}
      subtitle={t("music.count.songs", { count: a.count })}
      cover={a.cover}
      selected={picked === a.key}
      onClick={() => onPick(a.key)}
    />
  );
  return (
    <nav aria-label={t("music.artist.albums")} className="mb-3 lg:sticky lg:top-0 lg:mb-0 lg:self-start">
      <div className="flex gap-2 overflow-x-auto pb-1 lg:hidden">
        {chip(null, t("music.detail.allSongs"))}
        {albums.map((a) => chip(a.key, a.title))}
      </div>
      <div className="hidden lg:block">
        <p className="px-2 pb-1 text-xs font-medium uppercase tracking-wide text-muted">{t("music.artist.albums")}</p>
        <MediaRow kind="music" title={t("music.detail.allSongs")} subtitle={t("music.count.songs", { count: total })} selected={picked === null} onClick={() => onPick(null)} />
        {albums.filter((a) => a.own).map(row)}
        {/* Compilations and the like: kept apart so "Albums" holds what the
            artist list counts as theirs. */}
        {albums.some((a) => !a.own) && (
          <>
            <p className="px-2 pb-1 pt-4 text-xs font-medium uppercase tracking-wide text-muted">{t("music.artist.appearsOn")}</p>
            {albums.filter((a) => !a.own).map(row)}
          </>
        )}
      </div>
    </nav>
  );
}
