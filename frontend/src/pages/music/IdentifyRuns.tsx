// SPDX-License-Identifier: AGPL-3.0-or-later
// Identify a library in the background, then go through what it proposes —
// the server's identify runs (`features.identify_library`). Nothing here ever
// writes to a file: accepting changes the library, and a run can be undone.
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { useInfiniteQuery, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { ArrowRight, Check, Disc3, Loader2, ScanSearch, Trash2, Undo2, X } from "lucide-react";
import {
  acceptProposal,
  acceptSure,
  chooseEdition,
  createRun,
  deleteRun,
  formatCharge,
  getGroup,
  getRun,
  identifyError,
  isActive,
  listLibraryFolders,
  listProposals,
  listRuns,
  quoteRun,
  rejectProposal,
  revertRun,
  setKeepAlbum,
  type Decision,
  type IdentifyRun,
  type Proposal,
  type ProposalClass,
  type RunScope,
} from "../../api/identify";
import { Button, EmptyState, Pill, SegmentedControl, Select, Skeleton, toast } from "../../components/ui";
import { Page } from "../../components/shell/SplitView";
import { useServerFeatures } from "../../lib/features";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";
import { MUSIC_KEYS } from "./musicKeys";

type Tab = "review" | "sure" | "none" | "decided";

/** The reasons this console knows how to say, by the server's code; a newer
 *  server may send others, shown as they come. */
const REASONS = {
  incomplete: "incomplete",
  partial: "partial",
  layout: "layout",
  duplicate: "duplicate",
  ambiguous: "ambiguous",
  album_differs: "albumDiffers",
  artist_differs: "artistDiffers",
  weak: "weak",
  not_found: "notFound",
  not_on_release: "notOnRelease",
} as const;
type ReasonCode = keyof typeof REASONS;
const isReason = (r: string): r is ReasonCode => r in REASONS;

const TAB_FILTER: Record<Tab, { class?: ProposalClass; decision?: Decision }> = {
  review: { class: "review", decision: "pending" },
  sure: { class: "sure", decision: "pending" },
  none: { class: "none", decision: "pending" },
  decided: {},
};

/** "1-04" for a track on a known disc, "4" without one. */
function position(disc?: number | null, track?: number | null): string {
  if (!track) return "";
  return disc && disc > 0 ? `${disc}-${String(track).padStart(2, "0")}` : String(track);
}

function clock(ms?: number | null): string {
  if (!ms) return "";
  const s = Math.round(ms / 1000);
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
}

/** What a run covers, as named when it started, else by its kind. */
function useScopeName(): (run: IdentifyRun) => string {
  const { t } = useT();
  return (run) => {
    const kind = run.scope.kind;
    // A library run's stored label is in whatever language started it.
    if (run.scope.label && kind !== "library") return run.scope.label;
    return kind === "folder" ? t("music.identifyRuns.scope.folder", { path: "" }) : t(`music.identifyRuns.scope.${kind}`);
  };
}

function StatusPill({ run }: { run: IdentifyRun }) {
  const { t } = useT();
  const tone = run.status === "ready" ? "success" : run.status === "failed" ? "error" : run.status === "waiting" ? "warning" : "accent";
  return <Pill tone={tone}>{t(`music.identifyRuns.status.${run.status}`)}</Pill>;
}

function Progress({ run }: { run: IdentifyRun }) {
  const { t } = useT();
  const pct = run.groups_total > 0 ? Math.round((run.groups_done / run.groups_total) * 100) : 0;
  return (
    <div>
      <div className="h-1.5 overflow-hidden rounded-pill bg-bg-alt">
        <div className="h-full bg-accent transition-all" style={{ width: `${pct}%` }} />
      </div>
      <p className="mt-1 text-xs text-muted">
        {t("music.identifyRuns.progress", { done: run.groups_done, total: run.groups_total, tracks: run.tracks })}
      </p>
    </div>
  );
}

// ── The list of runs, and starting one ────────────────────────────────────

export default function IdentifyRunsPage() {
  const { runId } = useParams();
  return runId ? <RunPage runId={runId} /> : <RunsPage />;
}

function RunsPage() {
  const { t } = useT();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const { features } = useServerFeatures();
  const scopeName = useScopeName();
  const [scope, setScope] = useState("library");

  const runs = useQuery({
    queryKey: ["identify-runs"],
    queryFn: listRuns,
    refetchInterval: (q) => ((q.state.data ?? []).some(isActive) ? 3000 : false),
  });
  const folders = useQuery({ queryKey: ["library-folders-music"], queryFn: listLibraryFolders, enabled: features.library_folders });

  const start = useMutation({
    mutationFn: async (): Promise<IdentifyRun | null> => {
      const folder = folders.data?.find((f) => f.id === scope);
      const runScope: RunScope = scope === "library"
        ? { kind: "library", label: t("music.identifyRuns.scope.library") }
        : { kind: "folder", id: scope, label: folder?.path };
      if (features.billing) {
        const quote = await quoteRun(runScope);
        if (quote.charge_micro > 0
          && !window.confirm(t("music.identifyRuns.confirmCharge", { tracks: quote.tracks, price: formatCharge(quote.charge_micro) }))) {
          return null;
        }
      }
      return createRun(runScope);
    },
    onSuccess: (run) => {
      if (!run) return;
      qc.invalidateQueries({ queryKey: ["identify-runs"] });
      navigate(`/music/identify/${run.id}`);
    },
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.startFailed"))),
  });

  return (
    <Page title={t("music.identifyRuns.title")} back="/music" width="max-w-3xl">
      <p className="mb-4 text-sm text-muted">{t("music.identifyRuns.intro")}</p>

      <div className="mb-6 flex flex-wrap items-end gap-3 rounded-card border border-border p-4">
        <div className="min-w-[14rem] flex-1">
          <Select label={t("music.identifyRuns.scope.label")} value={scope} onChange={(e) => setScope(e.target.value)}>
            <option value="library">{t("music.identifyRuns.scope.library")}</option>
            {(folders.data ?? []).map((f) => (
              <option key={f.id} value={f.id}>{t("music.identifyRuns.scope.folder", { path: f.path })}</option>
            ))}
          </Select>
        </div>
        <Button icon={<ScanSearch className="h-4 w-4" />} loading={start.isPending} onClick={() => start.mutate()}>
          {t("music.identifyRuns.start")}
        </Button>
      </div>

      {runs.isLoading ? (
        <Skeleton className="h-32" />
      ) : (runs.data ?? []).length === 0 ? (
        <EmptyState icon={<ScanSearch />} title={t("music.identifyRuns.emptyTitle")} description={t("music.identifyRuns.emptyDescription")} />
      ) : (
        <ul className="space-y-2">
          {(runs.data ?? []).map((run) => (
            <li key={run.id}>
              <Link to={`/music/identify/${run.id}`} className="block rounded-card border border-border p-3 hover:bg-bg-alt">
                <div className="flex items-center justify-between gap-3">
                  <span className="truncate text-sm font-medium">{scopeName(run)}</span>
                  <StatusPill run={run} />
                </div>
                <p className="mt-1 text-xs text-muted">
                  {t("music.identifyRuns.summary", {
                    sure: run.counts.sure, review: run.counts.review, none: run.counts.none, accepted: run.counts.accepted,
                  })}
                </p>
                {isActive(run) && <div className="mt-2"><Progress run={run} /></div>}
              </Link>
            </li>
          ))}
        </ul>
      )}
    </Page>
  );
}

// ── One run: progress, and its proposals ──────────────────────────────────

function RunPage({ runId }: { runId: string }) {
  const { t } = useT();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const scopeName = useScopeName();
  const [tab, setTab] = useState<Tab>("review");

  const run = useQuery({
    queryKey: ["identify-run", runId],
    queryFn: () => getRun(runId),
    refetchInterval: (q) => (isActive(q.state.data) ? 3000 : false),
  });
  const r = run.data;

  const proposals = useInfiniteQuery({
    // Re-read as the run finds more: the key moves with its progress.
    queryKey: ["identify-proposals", runId, tab, r?.groups_done, r?.status],
    queryFn: ({ pageParam }) => listProposals(runId, TAB_FILTER[tab], pageParam),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next,
    enabled: !!r,
  });
  const items = useMemo(() => {
    const all = (proposals.data?.pages ?? []).flatMap((p) => p.items);
    return tab === "decided" ? all.filter((p) => p.decision !== "pending") : all;
  }, [proposals.data, tab]);

  function refresh() {
    qc.invalidateQueries({ queryKey: ["identify-run", runId] });
    qc.invalidateQueries({ queryKey: ["identify-proposals", runId] });
    qc.invalidateQueries({ queryKey: ["identify-runs"] });
    MUSIC_KEYS.forEach((k) => qc.invalidateQueries({ queryKey: [k] }));
  }

  const acceptAll = useMutation({
    mutationFn: () => acceptSure(runId),
    onSuccess: (n) => { toast.success(t("music.identifyRuns.acceptedSure", { count: n })); refresh(); },
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });
  const revert = useMutation({
    mutationFn: () => revertRun(runId),
    onSuccess: (n) => { toast.success(t("music.identifyRuns.reverted", { count: n })); refresh(); },
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });
  const remove = useMutation({
    mutationFn: () => deleteRun(runId),
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["identify-runs"] }); navigate("/music/identify"); },
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });

  if (run.isLoading || !r) {
    return <Page title={t("music.identifyRuns.title")} back="/music/identify" width="max-w-4xl"><Skeleton className="h-40" /></Page>;
  }

  return (
    <RunView
      run={r}
      title={scopeName(r)}
      tab={tab}
      setTab={setTab}
      items={items}
      proposals={proposals}
      refresh={refresh}
      acceptAll={acceptAll}
      revert={revert}
      remove={remove}
    />
  );
}

/** Keys on the run page; the player keeps space, arrows, M, S and R. */
function isTyping(el: EventTarget | null): boolean {
  return el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement
    || !!(el as HTMLElement | null)?.isContentEditable;
}

/** Songs a group's "accept album" takes: pending, with a match, not another artist's. */
function acceptableInGroup(ps: Proposal[]): Proposal[] {
  // A song matched to another artist is probably a wrong match: decide it on its own.
  return ps.filter((p) => p.decision === "pending" && p.proposal && !p.reasons.includes("artist_differs"));
}

function RunView({ run: r, title, tab, setTab, items, proposals, refresh, acceptAll, revert, remove }: {
  run: IdentifyRun;
  title: string;
  tab: Tab;
  setTab: (t: Tab) => void;
  items: Proposal[];
  proposals: { isLoading: boolean; hasNextPage: boolean; isFetchingNextPage: boolean; fetchNextPage: () => unknown };
  refresh: () => void;
  acceptAll: { isPending: boolean; mutate: () => void };
  revert: { isPending: boolean; mutate: () => void };
  remove: { isPending: boolean; mutate: () => void };
}) {
  const { t } = useT();
  const groups = useMemo(() => groupBy(items), [items]);
  const pendingOrder = useMemo(() => groups.flatMap(([, ps]) => ps).filter((p) => p.decision === "pending"), [groups]);
  const [focused, setFocused] = useState<string | null>(null);
  const focusedIndex = pendingOrder.findIndex((p) => p.id === focused);

  // The row that was focused may be gone (decided): keep the place in the list.
  const lastIndex = useRef(0);
  useEffect(() => {
    if (focusedIndex >= 0) lastIndex.current = focusedIndex;
    else if (focused && pendingOrder.length > 0) setFocused(pendingOrder[Math.min(lastIndex.current, pendingOrder.length - 1)].id);
  }, [focused, focusedIndex, pendingOrder]);

  const act = useMutation({
    mutationFn: async ({ kind, p }: { kind: "accept" | "reject" | "album"; p: Proposal }) => {
      if (kind === "accept") await acceptProposal(p.id);
      else if (kind === "reject") await rejectProposal(p.id);
      else for (const q of acceptableInGroup(groups.find(([id]) => id === p.group.id)?.[1] ?? [])) await acceptProposal(q.id);
    },
    onSuccess: refresh,
    onError: (e) => { toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))); refresh(); },
  });

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if (isTyping(e.target) || e.metaKey || e.ctrlKey || e.altKey || act.isPending) return;
      const key = e.key;
      const at = focusedIndex;
      const current = at >= 0 ? pendingOrder[at] : undefined;
      if (key === "j" || key === "k") {
        if (pendingOrder.length === 0) return;
        const next = at < 0 ? 0 : Math.max(0, Math.min(pendingOrder.length - 1, at + (key === "j" ? 1 : -1)));
        setFocused(pendingOrder[next].id);
      } else if (key === "a" && current?.proposal) {
        act.mutate({ kind: "accept", p: current });
      } else if (key === "x" && current) {
        act.mutate({ kind: "reject", p: current });
      } else if (key === "A" && current) {
        act.mutate({ kind: "album", p: current });
      } else {
        return;
      }
      e.preventDefault();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [act, focusedIndex, pendingOrder]);

  return (
    <Page
      title={title}
      back="/music/identify"
      width="max-w-4xl"
      actions={
        <div className="flex gap-2">
          {r.counts.accepted > 0 && (
            <Button size="sm" variant="secondary" icon={<Undo2 className="h-4 w-4" />} loading={revert.isPending}
              onClick={() => { if (window.confirm(t("music.identifyRuns.revertConfirm"))) revert.mutate(); }}>
              {t("music.identifyRuns.revert")}
            </Button>
          )}
          <Button size="sm" variant="ghost" icon={<Trash2 className="h-4 w-4" />} loading={remove.isPending}
            onClick={() => { if (window.confirm(t("music.identifyRuns.deleteConfirm"))) remove.mutate(); }}>
            {t("music.identifyRuns.delete")}
          </Button>
        </div>
      }
    >
      <div className="mb-4 space-y-3 rounded-card border border-border p-4">
        <div className="flex items-center justify-between gap-3">
          <StatusPill run={r} />
          <span className="text-xs text-muted">
            {t("music.identifyRuns.summary", { sure: r.counts.sure, review: r.counts.review, none: r.counts.none, accepted: r.counts.accepted })}
          </span>
        </div>
        {isActive(r) && <Progress run={r} />}
        {r.status === "waiting" && <p className="text-xs text-muted">{t("music.identifyRuns.waiting")}</p>}
        {r.status === "failed" && r.error && <p className="text-xs text-error">{r.error}</p>}
        {r.status === "ready" && r.tracks === 0 && <p className="text-sm text-muted">{t("music.identifyRuns.nothingToDo")}</p>}
      </div>

      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <SegmentedControl<Tab>
          size="sm"
          value={tab}
          onChange={setTab}
          segments={[
            { value: "review", label: t("music.identifyRuns.tab.review") },
            { value: "sure", label: t("music.identifyRuns.tab.sure") },
            { value: "none", label: t("music.identifyRuns.tab.none") },
            { value: "decided", label: t("music.identifyRuns.tab.decided") },
          ]}
        />
        {pendingOrder.length > 0 && (
          <p className="hidden text-xs text-muted md:block">{t("music.identifyRuns.keys")}</p>
        )}
        {tab === "sure" && items.length > 0 && (
          <Button size="sm" icon={<Check className="h-4 w-4" />} loading={acceptAll.isPending} onClick={() => acceptAll.mutate()}>
            {t("music.identifyRuns.acceptAllSure", { count: items.length })}
          </Button>
        )}
      </div>

      {proposals.isLoading ? (
        <Skeleton className="h-40" />
      ) : items.length === 0 ? (
        <EmptyState icon={<ScanSearch />} title={t(`music.identifyRuns.empty.${tab}`)} description={isActive(r) ? t("music.identifyRuns.stillLooking") : ""} />
      ) : (
        <div className="space-y-3">
          {groups.map(([groupId, ps]) => (
            <GroupCard key={groupId} proposals={ps} focused={focused} onFocus={setFocused} onChanged={refresh} />
          ))}
          {proposals.hasNextPage && (
            <Button variant="secondary" size="sm" loading={proposals.isFetchingNextPage} onClick={() => proposals.fetchNextPage()}>
              {t("music.identifyRuns.more")}
            </Button>
          )}
        </div>
      )}
    </Page>
  );
}

/** Proposals by group, in the server's order; an album's tracks by disc and number. */
function groupBy(items: Proposal[]): [string, Proposal[]][] {
  const out = new Map<string, Proposal[]>();
  for (const p of items) {
    const list = out.get(p.group.id) ?? [];
    list.push(p);
    out.set(p.group.id, list);
  }
  for (const list of out.values()) {
    list.sort((a, b) =>
      (a.track.disc_number ?? 1) - (b.track.disc_number ?? 1) || (a.track.track_number ?? 999) - (b.track.track_number ?? 999),
    );
  }
  return [...out.entries()];
}

function GroupCard({ proposals, focused, onFocus, onChanged }: {
  proposals: Proposal[];
  focused: string | null;
  onFocus: (id: string) => void;
  onChanged: () => void;
}) {
  const { t } = useT();
  const group = proposals[0].group;
  const [editionsOpen, setEditionsOpen] = useState(false);
  const pending = acceptableInGroup(proposals);

  const acceptGroup = useMutation({
    mutationFn: async () => {
      for (const p of pending) await acceptProposal(p.id);
    },
    onSuccess: onChanged,
    onError: (e) => { toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))); onChanged(); },
  });

  const first = proposals.find((p) => p.proposal)?.proposal;
  return (
    <div className="rounded-card border border-border">
      <div className="flex flex-wrap items-center gap-2 border-b border-border px-4 py-2">
        <span className="min-w-0 flex-1 truncate text-sm font-medium">{group.label}</span>
        {group.kind === "album" && first?.album && (
          <span className="truncate text-xs text-muted">
            <ArrowRight className="mr-1 inline h-3 w-3" />
            {first.album}{first.year ? ` (${first.year})` : ""}
          </span>
        )}
        {group.kind === "album" && (
          <Button size="sm" variant="ghost" icon={<Disc3 className="h-4 w-4" />} onClick={() => setEditionsOpen((v) => !v)}>
            {t("music.identifyRuns.otherEdition")}
          </Button>
        )}
        {pending.length > 1 && (
          <Button size="sm" variant="secondary" icon={<Check className="h-4 w-4" />} loading={acceptGroup.isPending} onClick={() => acceptGroup.mutate()}>
            {t("music.identifyRuns.acceptGroup", { count: pending.length })}
          </Button>
        )}
      </div>
      {editionsOpen && <Editions groupId={group.id} onChosen={() => { setEditionsOpen(false); onChanged(); }} />}
      {proposals.map((p, i) => (
        <ProposalRow key={p.id} p={p} first={i === 0} focused={p.id === focused} onFocus={() => onFocus(p.id)} onChanged={onChanged} />
      ))}
    </div>
  );
}

function Editions({ groupId, onChosen }: { groupId: string; onChosen: () => void }) {
  const { t } = useT();
  const { data, isLoading } = useQuery({ queryKey: ["identify-group", groupId], queryFn: () => getGroup(groupId) });
  const choose = useMutation({
    mutationFn: (index: number) => chooseEdition(groupId, index),
    onSuccess: onChosen,
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });
  if (isLoading || !data) return <div className="px-4 py-2"><Loader2 className="h-4 w-4 animate-spin" /></div>;
  if (data.editions.length === 0) return <p className="px-4 py-2 text-xs text-muted">{t("music.identifyRuns.noEditions")}</p>;
  return (
    <ul className="border-b border-border bg-bg-alt/50 px-4 py-2">
      {data.chosen == null && <li className="py-1 text-xs text-muted">{t("music.identifyRuns.noChosenEdition")}</li>}
      {data.editions.map((e) => (
        <li key={e.index} className="flex items-center gap-3 py-1 text-sm">
          <span className="min-w-0 flex-1 truncate">
            {e.title}{e.year ? ` (${e.year})` : ""}
            <span className="ml-2 text-xs text-muted">
              {t("music.identifyRuns.editionDetail", { tracks: e.track_count, matched: e.matched })}
              {e.secondary_types.length > 0 ? ` · ${e.secondary_types.join(", ")}` : ""}
              {e.status && e.status !== "Official" ? ` · ${e.status}` : ""}
            </span>
          </span>
          {data.chosen === e.index ? (
            <Pill tone="accent">{t("music.identifyRuns.chosen")}</Pill>
          ) : (
            <Button size="sm" variant="secondary" loading={choose.isPending && choose.variables === e.index} onClick={() => choose.mutate(e.index)}>
              {t("music.identifyRuns.useEdition")}
            </Button>
          )}
        </li>
      ))}
    </ul>
  );
}

function ProposalRow({ p, first, focused, onFocus, onChanged }: {
  p: Proposal;
  first: boolean;
  focused: boolean;
  onFocus: () => void;
  onChanged: () => void;
}) {
  const { t } = useT();
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (focused) ref.current?.scrollIntoView({ block: "nearest" });
  }, [focused]);
  const accept = useMutation({
    mutationFn: () => acceptProposal(p.id),
    onSuccess: onChanged,
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });
  const reject = useMutation({
    mutationFn: () => rejectProposal(p.id),
    onSuccess: onChanged,
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });
  const keep = useMutation({
    mutationFn: (value: boolean) => setKeepAlbum(p.id, value),
    onSuccess: onChanged,
    onError: (e) => toast.error(identifyError(e, t("music.identifyRuns.actionFailed"))),
  });
  const x = p.proposal;
  const albumQuestion = !!x && (p.reasons.includes("album_differs") || p.keep_album);

  return (
    <div
      ref={ref}
      onClick={p.decision === "pending" ? onFocus : undefined}
      className={cn(
        "grid gap-2 px-4 py-2.5 sm:grid-cols-[1fr_auto_1fr_auto] sm:items-center",
        !first && "border-t border-border",
        focused && "bg-accent/5 ring-2 ring-inset ring-accent/40",
      )}
    >
      <div className="min-w-0 text-sm">
        <p className="truncate">
          <span className="mr-2 text-xs tabular-nums text-muted">{position(p.track.disc_number, p.track.track_number)}</span>
          {p.track.title}
        </p>
        <p className="truncate text-xs text-muted">{[p.track.artist, p.track.album].filter(Boolean).join(" · ")}</p>
      </div>
      <ArrowRight className="hidden h-4 w-4 text-muted sm:block" />
      <div className="min-w-0 text-sm">
        {x ? (
          <>
            <p className="truncate">
              <span className="mr-2 text-xs tabular-nums text-muted">{position(x.disc_number, x.track_number)}</span>
              {x.title}
              <span className="ml-2 text-xs text-muted">{clock(x.length_ms)}</span>
            </p>
            <p className="truncate text-xs text-muted">
              {[x.artist ?? p.track.artist, p.keep_album ? p.track.album : x.album, x.year].filter(Boolean).join(" · ")}
            </p>
          </>
        ) : (
          <p className="text-xs text-muted">{t("music.identifyRuns.nothingFound")}</p>
        )}
        {p.reasons.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {p.reasons.map((r) => (
              <Pill key={r} tone="warning">{isReason(r) ? t(`music.identifyRuns.reason.${REASONS[r]}`) : r}</Pill>
            ))}
          </div>
        )}
        {albumQuestion && p.decision === "pending" && (
          <label className="mt-1 flex items-center gap-2 text-xs">
            <input type="checkbox" checked={p.keep_album} disabled={keep.isPending} onChange={(e) => keep.mutate(e.target.checked)} />
            {t("music.identifyRuns.keepAlbum", { album: p.track.album ?? "" })}
          </label>
        )}
      </div>
      <div className="flex justify-end gap-1">
        {p.decision === "pending" ? (
          <>
            {x && (
              <Button size="sm" icon={<Check className="h-4 w-4" />} loading={accept.isPending} onClick={() => accept.mutate()}>
                {t("music.identifyRuns.accept")}
              </Button>
            )}
            <Button size="sm" variant="ghost" icon={<X className="h-4 w-4" />} loading={reject.isPending} onClick={() => reject.mutate()}>
              {t("music.identifyRuns.reject")}
            </Button>
          </>
        ) : (
          <Pill tone={p.decision === "accepted" ? "success" : "neutral"}>{t(`music.identifyRuns.decision.${p.decision}`)}</Pill>
        )}
      </div>
    </div>
  );
}
