// SPDX-License-Identifier: AGPL-3.0-or-later
// The metadata organizer (docs/metadata-organizer-plan.md, O2–O4): your songs
// as a table to filter and edit, MusicBrainz beside it to compare and apply,
// and the history of every save with Undo — in Picard's spirit, except that
// nothing is ever written to a file.
import { useMemo, useState, type ReactNode } from "react";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Check, History, ImageUp, Loader2, Pencil, Search, Undo2, X } from "lucide-react";
import {
  EDITABLE,
  applyRelease,
  getFacets,
  identifyTrack,
  listBatches,
  listEdits,
  listOrganize,
  saveTracks,
  searchRecordings,
  searchReleases,
  undoBatch,
  uploadCover,
  type EditBatch,
  type OrganizeFilter,
  type OrganizeTrack,
  type RecordingCandidate,
  type ReleaseCandidate,
  type TrackFields,
} from "../../api/organize";
import { identifyError } from "../../api/identify";
import { Button, Dialog, DialogContent, Input, Pill, SegmentedControl, Skeleton, toast } from "../../components/ui";
import { useServerFeatures } from "../../lib/features";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

type View = "songs" | "history";
type Drafts = Record<string, TrackFields>;

const QUICK = ["unidentified", "no_cover", "no_year", "no_genre", "edited", "duplicates"] as const;

/** The table's columns: what each shows, and whether it can be edited in place. */
const COLUMNS: { key: keyof OrganizeTrack; sort?: string; editable?: boolean; numeric?: boolean; width: string }[] = [
  { key: "track_number", sort: "track", editable: true, numeric: true, width: "w-12" },
  { key: "title", sort: "title", editable: true, width: "min-w-[12rem]" },
  { key: "artist", sort: "artist", editable: true, width: "min-w-[9rem]" },
  { key: "album", sort: "album", editable: true, width: "min-w-[9rem]" },
  { key: "album_artist", sort: "album_artist", editable: true, width: "min-w-[8rem]" },
  { key: "genre", sort: "genre", editable: true, width: "min-w-[6rem]" },
  { key: "disc_number", editable: true, numeric: true, width: "w-12" },
  { key: "year", sort: "year", width: "w-14" },
  { key: "path", sort: "path", width: "min-w-[12rem]" },
];

function clock(secs?: number | null): string {
  if (!secs && secs !== 0) return "";
  return `${Math.floor(secs / 60)}:${String(Math.floor(secs % 60)).padStart(2, "0")}`;
}

type Loose = (key: string, params?: Record<string, unknown>) => string;

/** For keys built at run time (a column's, a source's): every one exists in
 *  i18n/strings/music.json, which the type cannot see through a template. */
function loose(t: unknown): Loose {
  return t as Loose;
}

/** `no_cover` → `noCover`: the catalogue's keys from the API's names. */
function camel(name: string): string {
  return name.replace(/_([a-z])/g, (_, c: string) => c.toUpperCase());
}

function errorText(e: unknown, fallback: string): string {
  return identifyError(e, fallback);
}

export default function OrganizePage() {
  const { t } = useT();
  const qc = useQueryClient();
  const [view, setView] = useState<View>("songs");
  const [filter, setFilter] = useState<OrganizeFilter>({});
  const [search, setSearch] = useState("");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [drafts, setDrafts] = useState<Drafts>({});
  const [bulkOpen, setBulkOpen] = useState(false);
  const cover = useMutation({
    mutationFn: async (file: File) => {
      const batch = crypto.randomUUID();
      for (const r of chosen) await uploadCover(r.id, file, batch);
      return batch;
    },
    onSuccess: (batch) => {
      refresh();
      toast.withAction(t("music.organize.coverSet", { count: chosen.length }), {
        label: t("music.organize.undo"),
        onClick: () => void undoBatch(batch).then(refresh),
      });
    },
    onError: (e) => toast.error(errorText(e, t("music.organize.saveFailed"))),
  });

  const facets = useQuery({ queryKey: ["organize-facets"], queryFn: getFacets });
  const effective = useMemo(() => ({ ...filter, q: search.trim() || undefined }), [filter, search]);
  const songs = useInfiniteQuery({
    queryKey: ["organize", effective],
    queryFn: ({ pageParam }) => listOrganize(effective, pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next ?? null,
  });
  const rows = useMemo(() => (songs.data?.pages ?? []).flatMap((p) => p.items), [songs.data]);
  const total = songs.data?.pages[0]?.total ?? 0;
  const chosen = rows.filter((r) => selected.has(r.id));

  function refresh() {
    qc.invalidateQueries({ queryKey: ["organize"] });
    qc.invalidateQueries({ queryKey: ["organize-facets"] });
    qc.invalidateQueries({ queryKey: ["organize-batches"] });
  }

  const draftCount = Object.keys(drafts).length;
  const save = useMutation({
    mutationFn: () => saveTracks(Object.entries(drafts).map(([id, f]) => ({ id, ...f }))),
    onSuccess: (s) => {
      setDrafts({});
      refresh();
      toast.withAction(t("music.organize.saved", { count: s.updated }), {
        label: t("music.organize.undo"),
        onClick: () => void undoBatch(s.batch_id).then(refresh),
      });
    },
    onError: (e) => toast.error(errorText(e, t("music.organize.saveFailed"))),
  });

  function setDraft(id: string, field: keyof TrackFields, value: unknown) {
    setDrafts((d) => {
      const row = rows.find((r) => r.id === id);
      const next = { ...(d[id] ?? {}) } as Record<string, unknown>;
      if (row && (row[field] ?? null) === (value ?? null)) delete next[field];
      else next[field] = value;
      const out = { ...d };
      if (Object.keys(next).length) out[id] = next as TrackFields;
      else delete out[id];
      return out;
    });
  }

  function pick(next: OrganizeFilter) {
    setFilter(next);
    setSelected(new Set());
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <div className="flex flex-wrap items-center gap-3 border-b border-border px-4 py-3">
        <h1 className="text-lg font-semibold tracking-tight">{t("music.organize.title")}</h1>
        <SegmentedControl<View>
          size="sm"
          value={view}
          onChange={setView}
          segments={[
            { value: "songs", label: t("music.organize.songs") },
            { value: "history", label: t("music.organize.history") },
          ]}
        />
        <span className="ml-auto text-xs text-muted">{t("music.organize.filesUntouched")}</span>
      </div>

      {view === "history" ? (
        <HistoryView onChanged={refresh} />
      ) : (
        <div className="grid min-h-0 flex-1 grid-cols-1 lg:grid-cols-[15rem_minmax(0,1fr)_22rem]">
          <FiltersPane facets={facets.data} loading={facets.isLoading} filter={filter} onPick={pick} />

          <section className="flex min-h-0 flex-col border-border lg:border-x">
            <div className="flex flex-wrap items-center gap-2 border-b border-border p-2">
              <div className="min-w-[12rem] flex-1">
                <Input className="h-9" value={search} onChange={(e) => setSearch(e.target.value)} placeholder={t("music.organize.search")} aria-label={t("music.organize.search")} />
              </div>
              <span className="text-xs text-muted">{t("music.organize.count", { shown: rows.length, total })}</span>
              {chosen.length > 0 && (
                <>
                  <Button size="sm" variant="secondary" icon={<Pencil className="h-4 w-4" />} onClick={() => setBulkOpen(true)}>
                    {t("music.organize.editSelected", { count: chosen.length })}
                  </Button>
                  <label className={cn("inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-[10px] px-3 text-sm hover:bg-bg-alt", cover.isPending && "opacity-60")}>
                    {cover.isPending ? <Loader2 className="h-4 w-4 animate-spin" /> : <ImageUp className="h-4 w-4" />}
                    {t("music.organize.setCover")}
                    <input
                      type="file"
                      accept="image/*"
                      className="sr-only"
                      disabled={cover.isPending}
                      onChange={(e) => {
                        const file = e.target.files?.[0];
                        e.target.value = "";
                        if (file) cover.mutate(file);
                      }}
                    />
                  </label>
                </>
              )}
              {draftCount > 0 && (
                <>
                  <Button size="sm" variant="ghost" icon={<X className="h-4 w-4" />} onClick={() => setDrafts({})}>
                    {t("music.organize.discard")}
                  </Button>
                  <Button size="sm" icon={<Check className="h-4 w-4" />} loading={save.isPending} onClick={() => save.mutate()}>
                    {t("music.organize.save", { count: draftCount })}
                  </Button>
                </>
              )}
            </div>
            <SongTable
              rows={rows}
              loading={songs.isLoading}
              drafts={drafts}
              selected={selected}
              setSelected={setSelected}
              setDraft={setDraft}
              sort={filter.sort}
              desc={!!filter.desc}
              onSort={(s) => setFilter((f) => ({ ...f, sort: s, desc: f.sort === s ? !f.desc : false }))}
            />
            {songs.hasNextPage && (
              <div className="border-t border-border p-2 text-center">
                <Button size="sm" variant="secondary" loading={songs.isFetchingNextPage} onClick={() => songs.fetchNextPage()}>
                  {t("music.organize.more")}
                </Button>
              </div>
            )}
          </section>

          <MusicBrainzPane chosen={chosen} onApplied={refresh} />
        </div>
      )}

      <BulkEditDialog
        open={bulkOpen}
        onOpenChange={setBulkOpen}
        count={chosen.length}
        onApply={(fields) => {
          for (const r of chosen) for (const [k, v] of Object.entries(fields)) setDraft(r.id, k as keyof TrackFields, v);
          setBulkOpen(false);
        }}
      />
    </div>
  );
}

// ── Left: what to show ──────────────────────────────────────────────────────

function FiltersPane({ facets, loading, filter, onPick }: {
  facets?: Awaited<ReturnType<typeof getFacets>>;
  loading: boolean;
  filter: OrganizeFilter;
  onPick: (f: OrganizeFilter) => void;
}) {
  const { t } = useT();
  const tx = loose(t);
  const [artistQuery, setArtistQuery] = useState("");
  const keep = { sort: filter.sort, desc: filter.desc };
  const isAll = !QUICK.some((q) => filter[q]) && !filter.folder && !filter.artist && !filter.album_id;
  if (loading || !facets) return <aside className="p-3"><Skeleton className="h-40" /></aside>;
  const artists = facets.artists.filter((a) => a.name.toLowerCase().includes(artistQuery.toLowerCase())).slice(0, 200);
  return (
    <aside className="scroll-subtle min-h-0 overflow-y-auto p-2 text-sm">
      <FilterItem active={isAll} label={t("music.organize.filter.all")} count={facets.counts.all} onClick={() => onPick(keep)} />
      {QUICK.map((q) => (
        <FilterItem
          key={q}
          active={!!filter[q]}
          label={tx(`music.organize.filter.${camel(q)}`)}
          count={q === "duplicates" ? undefined : facets.counts[q]}
          onClick={() => onPick({ ...keep, [q]: true })}
        />
      ))}

      <h3 className="mb-1 mt-4 px-2 text-xs font-semibold uppercase tracking-wide text-muted">{t("music.organize.folders")}</h3>
      {facets.folders.map((f) => (
        <FilterItem
          key={f.id ?? "uploads"}
          active={!!f.id && filter.folder === f.id}
          label={f.path ?? t("music.organize.uploads")}
          count={f.tracks}
          onClick={() => f.id && onPick({ ...keep, folder: f.id })}
        />
      ))}

      <h3 className="mb-1 mt-4 px-2 text-xs font-semibold uppercase tracking-wide text-muted">{t("music.organize.artists")}</h3>
      <div className="px-1 pb-1">
        <Input className="h-8" value={artistQuery} onChange={(e) => setArtistQuery(e.target.value)} placeholder={t("music.organize.findArtist")} aria-label={t("music.organize.findArtist")} />
      </div>
      {artists.map((a) => (
        <FilterItem key={a.name} active={filter.artist === a.name} label={a.name} count={a.tracks} onClick={() => onPick({ ...keep, artist: a.name })} />
      ))}

      <h3 className="mb-1 mt-4 px-2 text-xs font-semibold uppercase tracking-wide text-muted">{t("music.organize.albums")}</h3>
      {facets.albums.slice(0, 300).map((a) => (
        <FilterItem key={a.id} active={filter.album_id === a.id} label={a.title} sub={a.artist} count={a.tracks} onClick={() => onPick({ ...keep, album_id: a.id })} />
      ))}
    </aside>
  );
}

function FilterItem({ active, label, sub, count, onClick }: { active: boolean; label: string; sub?: string; count?: number; onClick: () => void }) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn("flex w-full items-center gap-2 rounded-[8px] px-2 py-1 text-left hover:bg-bg-alt", active && "bg-accent/10 text-accent-text")}
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate">{label}</span>
        {sub && <span className="block truncate text-xs text-muted">{sub}</span>}
      </span>
      {count !== undefined && <span className="text-xs tabular-nums text-muted">{count}</span>}
    </button>
  );
}

// ── Middle: the songs ───────────────────────────────────────────────────────

function SongTable({ rows, loading, drafts, selected, setSelected, setDraft, sort, desc, onSort }: {
  rows: OrganizeTrack[];
  loading: boolean;
  drafts: Drafts;
  selected: Set<string>;
  setSelected: (s: Set<string>) => void;
  setDraft: (id: string, field: keyof TrackFields, value: unknown) => void;
  sort?: string;
  desc: boolean;
  onSort: (s: string) => void;
}) {
  const { t } = useT();
  const tx = loose(t);
  const [editing, setEditing] = useState<{ id: string; key: keyof TrackFields } | null>(null);
  const [anchor, setAnchor] = useState<number | null>(null);
  if (loading) return <div className="p-3"><Skeleton className="h-64" /></div>;
  if (rows.length === 0) return <p className="p-6 text-center text-sm text-muted">{t("music.organize.empty")}</p>;

  function toggle(i: number, shift: boolean) {
    const next = new Set(selected);
    if (shift && anchor !== null) {
      const [a, b] = [Math.min(anchor, i), Math.max(anchor, i)];
      for (let k = a; k <= b; k++) next.add(rows[k].id);
    } else if (next.has(rows[i].id)) next.delete(rows[i].id);
    else next.add(rows[i].id);
    setAnchor(i);
    setSelected(next);
  }
  const allSelected = rows.every((r) => selected.has(r.id));

  return (
    <div className="scroll-subtle min-h-0 flex-1 overflow-auto">
      <table className="w-full border-collapse text-sm">
        <thead className="sticky top-0 z-10 bg-card">
          <tr className="border-b border-border text-left text-xs text-muted">
            <th className="w-8 px-2 py-1.5">
              <input
                type="checkbox"
                aria-label={t("music.organize.selectAll")}
                checked={allSelected}
                onChange={() => setSelected(allSelected ? new Set() : new Set(rows.map((r) => r.id)))}
              />
            </th>
            {COLUMNS.map((c) => (
              <th key={c.key} className={cn("px-2 py-1.5 font-medium", c.width)}>
                {c.sort ? (
                  <button type="button" className="hover:text-fg" onClick={() => onSort(c.sort!)}>
                    {tx(`music.organize.col.${camel(c.key)}`)}{sort === c.sort ? (desc ? " ↓" : " ↑") : ""}
                  </button>
                ) : tx(`music.organize.col.${camel(c.key)}`)}
              </th>
            ))}
            <th className="w-14 px-2 py-1.5 font-medium">{t("music.organize.col.length")}</th>
            <th className="px-2 py-1.5 font-medium">{t("music.organize.col.state")}</th>
          </tr>
        </thead>
        <tbody>
          {rows.map((r, i) => {
            const draft = drafts[r.id] ?? {};
            return (
              <tr key={r.id} className={cn("border-b border-border/60 hover:bg-bg-alt/60", selected.has(r.id) && "bg-accent/5")}>
                <td className="px-2 py-1">
                  <input type="checkbox" aria-label={r.title} checked={selected.has(r.id)} onChange={(e) => toggle(i, (e.nativeEvent as MouseEvent).shiftKey)} />
                </td>
                {COLUMNS.map((c) => {
                  const key = c.key as keyof TrackFields;
                  const changed = c.editable && key in draft;
                  const value = changed ? (draft as Record<string, unknown>)[key] : r[c.key];
                  const isEditing = editing?.id === r.id && editing.key === key;
                  return (
                    <td
                      key={c.key}
                      className={cn("max-w-[18rem] px-2 py-1", changed && "bg-warning/12", c.editable && "cursor-text")}
                      onDoubleClick={() => c.editable && setEditing({ id: r.id, key })}
                    >
                      {isEditing ? (
                        <CellEditor
                          value={value}
                          numeric={!!c.numeric}
                          onDone={(v) => {
                            setEditing(null);
                            if (v !== undefined) setDraft(r.id, key, v);
                          }}
                        />
                      ) : (
                        <span
                          className={cn("block truncate", c.key === "path" && "text-xs text-muted", c.editable && "rounded px-0.5 focus:outline focus:outline-accent")}
                          tabIndex={c.editable ? 0 : undefined}
                          onKeyDown={(e) => { if (c.editable && (e.key === "Enter" || e.key === "F2")) setEditing({ id: r.id, key }); }}
                          title={value == null ? "" : String(value)}
                        >
                          {value == null || value === "" ? <span className="text-muted/60">—</span> : String(value)}
                        </span>
                      )}
                    </td>
                  );
                })}
                <td className="px-2 py-1 text-xs tabular-nums text-muted">{clock(r.duration_secs)}</td>
                <td className="px-2 py-1">
                  <div className="flex gap-1 whitespace-nowrap">
                    {r.identity ? <Pill tone="success">MB</Pill> : null}
                    {r.edited ? <Pill tone="accent">{t("music.organize.state.edited")}</Pill> : null}
                    {!r.has_cover ? <Pill>{t("music.organize.state.noCover")}</Pill> : null}
                  </div>
                </td>
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}

function CellEditor({ value, numeric, onDone }: { value: unknown; numeric: boolean; onDone: (v: unknown) => void }) {
  const [text, setText] = useState(value == null ? "" : String(value));
  function commit() {
    const s = text.trim();
    if (numeric) {
      if (s === "") return onDone(null);
      const n = Number.parseInt(s, 10);
      return onDone(Number.isFinite(n) && n > 0 ? n : undefined);
    }
    onDone(s === "" ? null : s);
  }
  return (
    <input
      autoFocus
      className="h-7 w-full rounded border border-accent bg-card px-1 text-sm outline-none"
      value={text}
      inputMode={numeric ? "numeric" : undefined}
      onChange={(e) => setText(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => {
        if (e.key === "Enter") commit();
        if (e.key === "Escape") onDone(undefined);
      }}
    />
  );
}

function BulkEditDialog({ open, onOpenChange, count, onApply }: {
  open: boolean;
  onOpenChange: (o: boolean) => void;
  count: number;
  onApply: (fields: TrackFields) => void;
}) {
  const { t } = useT();
  const tx = loose(t);
  const fields = EDITABLE.filter((f) => f !== "title" && f !== "track_number");
  const [on, setOn] = useState<Record<string, boolean>>({});
  const [values, setValues] = useState<Record<string, string>>({});
  function apply() {
    const out: Record<string, unknown> = {};
    for (const f of fields) {
      if (!on[f]) continue;
      const v = (values[f] ?? "").trim();
      out[f] = f === "disc_number" ? (v ? Number.parseInt(v, 10) || null : null) : v || null;
    }
    onApply(out as TrackFields);
    setOn({});
    setValues({});
  }
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        title={t("music.organize.bulkTitle", { count })}
        description={t("music.organize.bulkHint")}
        footer={
          <>
            <Button variant="ghost" onClick={() => onOpenChange(false)}>{t("common.action.cancel")}</Button>
            <Button onClick={apply} disabled={!Object.values(on).some(Boolean)}>{t("music.organize.bulkApply")}</Button>
          </>
        }
      >
        <div className="space-y-2">
          {fields.map((f) => (
            <label key={f} className="flex items-center gap-3 text-sm">
              <input type="checkbox" checked={!!on[f]} onChange={(e) => setOn((o) => ({ ...o, [f]: e.target.checked }))} />
              <span className="w-28 shrink-0">{tx(`music.organize.col.${camel(f)}`)}</span>
              <div className="flex-1">
                <Input
                  className="h-9"
                  value={values[f] ?? ""}
                  disabled={!on[f]}
                  inputMode={f === "disc_number" ? "numeric" : undefined}
                  placeholder={on[f] ? t("music.organize.bulkEmptyClears") : ""}
                  onChange={(e) => setValues((v) => ({ ...v, [f]: e.target.value }))}
                  aria-label={tx(`music.organize.col.${camel(f)}`)}
                />
              </div>
            </label>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}

// ── Right: MusicBrainz ──────────────────────────────────────────────────────

function MusicBrainzPane({ chosen, onApplied }: { chosen: OrganizeTrack[]; onApplied: () => void }) {
  const { t } = useT();
  const { features } = useServerFeatures();
  if (!features.music_identify) {
    return <aside className="p-4 text-sm text-muted">{t("music.organize.mb.unavailable")}</aside>;
  }
  return (
    <aside className="scroll-subtle min-h-0 overflow-y-auto p-3 text-sm">
      <h2 className="mb-2 font-semibold">MusicBrainz</h2>
      {chosen.length === 0 ? (
        <p className="text-muted">{t("music.organize.mb.pick")}</p>
      ) : chosen.length === 1 ? (
        <OneSong key={chosen[0].id} song={chosen[0]} onApplied={onApplied} />
      ) : (
        <ManySongs key={chosen.map((c) => c.id).join()} songs={chosen} onApplied={onApplied} />
      )}
    </aside>
  );
}

function OneSong({ song, onApplied }: { song: OrganizeTrack; onApplied: () => void }) {
  const { t } = useT();
  const [title, setTitle] = useState(song.title);
  const [artist, setArtist] = useState(song.artist ?? "");
  const [album, setAlbum] = useState(song.album ?? "");
  const [link, setLink] = useState("");
  const [open, setOpen] = useState<RecordingCandidate | null>(null);
  const search = useMutation({
    mutationFn: () => searchRecordings(song.id, { title: title.trim() || undefined, artist: artist.trim() || undefined, album: album.trim() || undefined }),
    onError: (e) => toast.error(errorText(e, t("music.organize.mb.failed"))),
  });
  const apply = useMutation({
    mutationFn: (c: { recording: string; release?: string | null }) => identifyTrack(song.id, c.recording, c.release),
    onSuccess: () => { toast.success(t("music.organize.mb.identified")); setOpen(null); onApplied(); },
    onError: (e) => toast.error(errorText(e, t("music.organize.mb.failed"))),
  });
  return (
    <div className="space-y-3">
      <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); search.mutate(); }}>
        <Input className="h-9" value={title} onChange={(e) => setTitle(e.target.value)} placeholder={t("music.organize.col.title")} aria-label={t("music.organize.col.title")} />
        <Input className="h-9" value={artist} onChange={(e) => setArtist(e.target.value)} placeholder={t("music.organize.col.artist")} aria-label={t("music.organize.col.artist")} />
        <Input className="h-9" value={album} onChange={(e) => setAlbum(e.target.value)} placeholder={t("music.organize.col.album")} aria-label={t("music.organize.col.album")} />
        <Button type="submit" size="sm" variant="secondary" icon={<Search className="h-4 w-4" />} loading={search.isPending} disabled={!title.trim() && !artist.trim() && !album.trim()}>
          {t("music.organize.mb.search")}
        </Button>
        <p className="text-xs text-muted">{t("music.organize.mb.ownWords")}</p>
      </form>
      {search.data && search.data.length === 0 && <p className="text-xs text-muted">{t("music.organize.mb.nothing")}</p>}
      <ul className="space-y-1">
        {(search.data ?? []).map((c) => (
          <li key={`${c.mb_recording_id}/${c.mb_release_id ?? ""}`} className="rounded-card border border-border">
            <button type="button" className="w-full px-2 py-1.5 text-left hover:bg-bg-alt" onClick={() => setOpen(open === c ? null : c)}>
              <span className="block truncate">{c.title}</span>
              <span className="block truncate text-xs text-muted">{[c.artist, c.album, c.year].filter(Boolean).join(" · ")} {c.duration_ms ? `· ${clock(c.duration_ms / 1000)}` : ""}</span>
            </button>
            {open === c && (
              <div className="border-t border-border p-2">
                <Compare
                  rows={[
                    ["title", song.title, c.title],
                    ["artist", song.artist, c.artist],
                    ["album", song.album, c.album],
                    ["track_number", song.track_number, c.track_number],
                    ["length", clock(song.duration_secs), c.duration_ms ? clock(c.duration_ms / 1000) : null],
                  ]}
                />
                <Button size="sm" className="mt-2" icon={<Check className="h-4 w-4" />} loading={apply.isPending}
                  onClick={() => apply.mutate({ recording: c.mb_recording_id, release: c.mb_release_id })}>
                  {t("music.organize.mb.use")}
                </Button>
              </div>
            )}
          </li>
        ))}
      </ul>
      <form className="flex gap-2" onSubmit={(e) => { e.preventDefault(); if (link.trim()) apply.mutate({ recording: link.trim() }); }}>
        <div className="flex-1"><Input className="h-9" value={link} onChange={(e) => setLink(e.target.value)} placeholder={t("music.organize.mb.link")} aria-label={t("music.organize.mb.link")} /></div>
        <Button type="submit" size="sm" variant="secondary" disabled={!link.trim()}>{t("music.organize.mb.use")}</Button>
      </form>
    </div>
  );
}

/** Field by field, Picard's way: the same in grey, what would change marked. */
function Compare({ rows }: { rows: [string, unknown, unknown][] }) {
  const { t } = useT();
  const tx = loose(t);
  const show = (v: unknown) => (v == null || v === "" ? "—" : String(v));
  return (
    <table className="w-full text-xs">
      <thead>
        <tr className="text-left text-muted">
          <th className="pb-1 font-medium" />
          <th className="pb-1 font-medium">{t("music.organize.mb.now")}</th>
          <th className="pb-1 font-medium">MusicBrainz</th>
        </tr>
      </thead>
      <tbody>
        {rows.map(([key, now, mb]) => {
          const same = show(now).toLowerCase() === show(mb).toLowerCase();
          return (
            <tr key={key}>
              <td className="pr-2 text-muted">{tx(`music.organize.col.${camel(key)}`)}</td>
              <td className={cn("pr-2", !same && "text-muted line-through")}>{show(now)}</td>
              <td className={cn(same ? "text-muted" : "font-medium text-success-text")}>{show(mb)}</td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}

function ManySongs({ songs, onApplied }: { songs: OrganizeTrack[]; onApplied: () => void }) {
  const { t } = useT();
  const common = (k: "album" | "album_artist" | "artist") => {
    const counts = new Map<string, number>();
    for (const s of songs) if (s[k]) counts.set(s[k]!, (counts.get(s[k]!) ?? 0) + 1);
    return [...counts.entries()].sort((a, b) => b[1] - a[1])[0]?.[0] ?? "";
  };
  const [album, setAlbum] = useState(common("album"));
  const [artist, setArtist] = useState(common("album_artist") || common("artist"));
  const [chosen, setChosen] = useState<ReleaseCandidate | null>(null);
  const search = useMutation({
    mutationFn: () => searchReleases(songs.map((s) => s.id), { album: album.trim() || undefined, artist: artist.trim() || undefined }),
    onSuccess: (r) => setChosen(r[0] ?? null),
    onError: (e) => toast.error(errorText(e, t("music.organize.mb.failed"))),
  });
  if (songs.length > 100) return <p className="text-muted">{t("music.organize.mb.tooMany")}</p>;
  return (
    <div className="space-y-3">
      <p className="text-xs text-muted">{t("music.organize.mb.manyHint", { count: songs.length })}</p>
      <form className="space-y-2" onSubmit={(e) => { e.preventDefault(); search.mutate(); }}>
        <Input className="h-9" value={album} onChange={(e) => setAlbum(e.target.value)} placeholder={t("music.organize.col.album")} aria-label={t("music.organize.col.album")} />
        <Input className="h-9" value={artist} onChange={(e) => setArtist(e.target.value)} placeholder={t("music.organize.col.album_artist")} aria-label={t("music.organize.col.album_artist")} />
        <Button type="submit" size="sm" variant="secondary" icon={<Search className="h-4 w-4" />} loading={search.isPending}>
          {t("music.organize.mb.findRelease")}
        </Button>
      </form>
      {search.data && search.data.length === 0 && <p className="text-xs text-muted">{t("music.organize.mb.nothing")}</p>}
      {(search.data ?? []).length > 0 && (
        <ul className="space-y-1">
          {(search.data ?? []).map((r) => (
            <li key={r.mb_release_id}>
              <button
                type="button"
                onClick={() => setChosen(r)}
                className={cn("w-full rounded-card border border-border px-2 py-1.5 text-left hover:bg-bg-alt", chosen === r && "border-accent bg-accent/5")}
              >
                <span className="block truncate">{r.title}{r.year ? ` (${r.year})` : ""}</span>
                <span className="block truncate text-xs text-muted">
                  {[r.artist, t("music.organize.mb.tracks", { tracks: r.track_count, matched: r.matched }), ...r.secondary_types].filter(Boolean).join(" · ")}
                </span>
              </button>
            </li>
          ))}
        </ul>
      )}
      {chosen && <ReleasePairs key={chosen.mb_release_id} release={chosen} songs={songs} onApplied={onApplied} />}
    </div>
  );
}

/** Which song is which track of the release; a pair can be changed or skipped
 *  before applying. */
function ReleasePairs({ release, songs, onApplied }: { release: ReleaseCandidate; songs: OrganizeTrack[]; onApplied: () => void }) {
  const { t } = useT();
  const [pairs, setPairs] = useState<Record<string, string>>(() =>
    Object.fromEntries(release.tracks.map((m) => [m.track_id, `${m.disc}:${m.position}:${m.mb_recording_id}`])),
  );
  const [cover, setCover] = useState(true);
  const [genre, setGenre] = useState(false);
  const options = release.tracks.slice().sort((a, b) => a.disc - b.disc || a.position - b.position);
  const apply = useMutation({
    mutationFn: () => applyRelease({
      release: release.mb_release_id,
      fetch_cover: cover,
      use_genre: genre,
      tracks: Object.entries(pairs)
        .filter(([, v]) => v)
        .map(([id, v]) => {
          const [disc, , recording] = v.split(":");
          return { id, recording, disc: Number(disc) || null };
        }),
    }),
    onSuccess: (s) => {
      toast.withAction(t("music.organize.mb.applied", { count: s.updated }), {
        label: t("music.organize.undo"),
        onClick: () => void undoBatch(s.batch_id).then(onApplied),
      });
      onApplied();
    },
    onError: (e) => toast.error(errorText(e, t("music.organize.mb.failed"))),
  });
  const count = Object.values(pairs).filter(Boolean).length;
  return (
    <div className="space-y-2 rounded-card border border-border p-2">
      <ul className="space-y-1">
        {songs.map((s) => {
          const v = pairs[s.id] ?? "";
          const m = options.find((o) => `${o.disc}:${o.position}:${o.mb_recording_id}` === v);
          return (
            <li key={s.id} className="text-xs">
              <span className="block truncate text-muted">{s.path}</span>
              <select
                className="mt-0.5 w-full rounded border border-border bg-card px-1 py-1"
                value={v}
                aria-label={s.title}
                onChange={(e) => setPairs((p) => ({ ...p, [s.id]: e.target.value }))}
              >
                <option value="">{t("music.organize.mb.skip")}</option>
                {options.map((o) => (
                  <option key={`${o.disc}:${o.position}`} value={`${o.disc}:${o.position}:${o.mb_recording_id}`}>
                    {`${o.disc}-${String(o.position).padStart(2, "0")} ${o.title}`}
                  </option>
                ))}
              </select>
              {m && m.title.toLowerCase() !== s.title.toLowerCase() && (
                <span className="block truncate"><span className="text-muted line-through">{s.title}</span> → <span className="text-success-text">{m.title}</span></span>
              )}
            </li>
          );
        })}
      </ul>
      <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={cover} onChange={(e) => setCover(e.target.checked)} />{t("music.organize.mb.fetchCover")}</label>
      <label className="flex items-center gap-2 text-xs"><input type="checkbox" checked={genre} onChange={(e) => setGenre(e.target.checked)} />{t("music.organize.mb.useGenre")}</label>
      <Button size="sm" icon={<Check className="h-4 w-4" />} loading={apply.isPending} disabled={count === 0} onClick={() => apply.mutate()}>
        {t("music.organize.mb.applyRelease", { count })}
      </Button>
    </div>
  );
}

// ── History ─────────────────────────────────────────────────────────────────

function HistoryView({ onChanged }: { onChanged: () => void }) {
  const { t } = useT();
  const tx = loose(t);
  const qc = useQueryClient();
  const [open, setOpen] = useState<string | null>(null);
  const batches = useInfiniteQuery({
    queryKey: ["organize-batches"],
    queryFn: ({ pageParam }) => listBatches(pageParam),
    initialPageParam: null as string | null,
    getNextPageParam: (last) => last.next ?? null,
  });
  const undo = useMutation({
    mutationFn: (id: string) => undoBatch(id),
    onSuccess: (r) => {
      toast.success(t("music.organize.undone", { count: r.items }));
      qc.invalidateQueries({ queryKey: ["organize-batches"] });
      qc.invalidateQueries({ queryKey: ["organize-edits"] });
      onChanged();
    },
    onError: (e) => toast.error(errorText(e, t("music.organize.undoFailed"))),
  });
  const items = (batches.data?.pages ?? []).flatMap((p) => p.items);
  if (batches.isLoading) return <div className="p-4"><Skeleton className="h-64" /></div>;
  if (items.length === 0) return <p className="p-8 text-center text-sm text-muted">{t("music.organize.historyEmpty")}</p>;
  return (
    <div className="scroll-subtle min-h-0 flex-1 overflow-y-auto">
      <ul className="mx-auto max-w-3xl divide-y divide-border p-4">
        {items.map((b) => (
          <li key={b.batch_id} className="py-2">
            <div className="flex items-center gap-3">
              <History className="h-4 w-4 shrink-0 text-muted" />
              <button type="button" className="min-w-0 flex-1 text-left" onClick={() => setOpen(open === b.batch_id ? null : b.batch_id)}>
                <span className={cn("block truncate text-sm", b.undone && "text-muted line-through")}>{batchTitle(tx, b)}</span>
                <span className="block text-xs text-muted">{new Date(b.at).toLocaleString()} · {tx(`music.organize.source.${camel(b.source)}`)}</span>
              </button>
              {!b.undone && (
                <Button size="sm" variant="ghost" icon={<Undo2 className="h-4 w-4" />} loading={undo.isPending && undo.variables === b.batch_id} onClick={() => undo.mutate(b.batch_id)}>
                  {t("music.organize.undo")}
                </Button>
              )}
            </div>
            {open === b.batch_id && <BatchEdits batchId={b.batch_id} />}
          </li>
        ))}
      </ul>
      {batches.hasNextPage && (
        <div className="pb-6 text-center">
          <Button size="sm" variant="secondary" loading={batches.isFetchingNextPage} onClick={() => batches.fetchNextPage()}>{t("music.organize.more")}</Button>
        </div>
      )}
    </div>
  );
}

function batchTitle(t: Loose, b: EditBatch): string {
  const name = b.first_label ?? "";
  return b.items > 1 ? t("music.organize.batchMany", { name, count: b.items - 1 }) : name;
}

function BatchEdits({ batchId }: { batchId: string }) {
  const { t } = useT();
  const tx = loose(t);
  const edits = useQuery({ queryKey: ["organize-edits", batchId], queryFn: () => listEdits({ batch_id: batchId }) });
  if (edits.isLoading) return <Loader2 className="ml-7 mt-2 h-4 w-4 animate-spin" />;
  return (
    <ul className="ml-7 mt-2 space-y-2">
      {(edits.data ?? []).map((e) => (
        <li key={e.id} className="text-xs">
          <span className="font-medium">{e.item_label ?? e.item_id}</span>
          <table className="mt-1">
            <tbody>
              {Object.keys(e.after).map((k) => (
                <Row key={k} label={tx(`music.organize.field.${camel(k)}`)}>
                  <span className="text-muted line-through">{show(e.before[k])}</span> → <span>{show(e.after[k])}</span>
                </Row>
              ))}
            </tbody>
          </table>
        </li>
      ))}
    </ul>
  );
}

function Row({ label, children }: { label: string; children: ReactNode }) {
  return (
    <tr>
      <td className="pr-3 align-top text-muted">{label}</td>
      <td>{children}</td>
    </tr>
  );
}

function show(v: unknown): string {
  return v == null || v === "" ? "—" : String(v);
}
