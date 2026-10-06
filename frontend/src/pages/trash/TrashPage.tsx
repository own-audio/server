// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { FileText, ListMusic, RotateCcw, Trash2 } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../../components/ui/CloudIcon";
import {
  emptyTrash,
  listTrash,
  purgeTrashItem,
  restoreTrashBatch,
  restoreTrashItem,
  type RestoreResult,
  type TrashItem,
  type TrashKind,
  type TrashScope,
} from "../../api/trash";
import { formatBytes, formatMicro, getBilling } from "../../api/billing";
import { Page } from "../../components/shell/SplitView";
import { Button, Dialog, DialogContent, EmptyState, IconButton, SegmentedControl, Skeleton, toast } from "../../components/ui";
import { useMyPermissions } from "../../lib/permissions";
import { daysLeft, groupByDeletion, type TrashGroup } from "../../lib/trash";
import { timeAgo } from "../../lib/format";
import { useAuthStore } from "../../store/authStore";
import { t, useT, type PlainKey } from "../../i18n";

const KIND: Record<TrashKind, { label: PlainKey; icon: React.ReactNode }> = {
  audiobook: { label: "trash.kind.audiobook", icon: <BookIcon className="h-4 w-4 text-book" /> },
  music_track: { label: "trash.kind.song", icon: <MusicIcon className="h-4 w-4 text-music" /> },
  playlist: { label: "trash.kind.playlist", icon: <ListMusic className="h-4 w-4 text-music" /> },
  podcast_episode: { label: "trash.kind.episode", icon: <PodcastIcon className="h-4 w-4 text-podcast" /> },
  companion_file: { label: "trash.kind.file", icon: <FileText className="h-4 w-4 text-muted" /> },
};

const ONE_CENT = 10_000;

interface PendingRestore {
  /** The item's title, or null for a batch of `count` items. */
  title: string | null;
  count: number;
  charge: number;
  days: number;
  run: () => Promise<RestoreResult>;
}

/**
 * The trash (docs/file-sync-plan.md §5.1 and §8).
 *
 * Everything deleted waits here for 30 days. Restoring is free the same day
 * and otherwise charges the storage for the days an item spent here — so the
 * cost is shown, and confirmed, before a restore that costs anything.
 */
export default function TrashPage() {
  const { t } = useT();
  const qc = useQueryClient();
  const me = useAuthStore((s) => s.user?.id);
  const { isFamilyAdmin } = useMyPermissions();
  const [scope, setScope] = useState<TrashScope>("mine");
  const effective: TrashScope = isFamilyAdmin ? scope : "mine";

  const { data: items = [], isLoading } = useQuery({
    queryKey: ["trash", effective],
    queryFn: () => listTrash(effective),
  });
  const { data: billing } = useQuery({ queryKey: ["billing"], queryFn: getBilling, retry: false });
  const currency = billing?.pricing.currency ?? "USD";
  const money = (micro: number) => formatMicro(micro, currency);
  // A few KB for a few days is a fraction of a cent; "$0.00" would read as free.
  const cost = (micro: number) => (micro < ONE_CENT ? t("trash.lessThan", { amount: money(ONE_CENT) }) : money(micro));

  const [pendingRestore, setPendingRestore] = useState<PendingRestore | null>(null);
  const [purging, setPurging] = useState<TrashItem | null>(null);
  const [emptying, setEmptying] = useState(false);

  // A restore can bring the item back into any list — refresh them all.
  const refreshAll = () => qc.invalidateQueries();

  const restore = useMutation({
    mutationFn: (p: PendingRestore) => p.run(),
    onSuccess: (r) => {
      refreshAll();
      setPendingRestore(null);
      toast.success(
        t("trash.restored", { count: r.restored }),
        r.charged_micro > 0 ? t("trash.charged", { cost: cost(r.charged_micro) }) : undefined
      );
    },
    onError: () => toast.error(t("trash.restoreError")),
  });

  const purge = useMutation({
    mutationFn: (item: TrashItem) => purgeTrashItem(item.kind, item.id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["trash"] });
      setPurging(null);
      toast.success(t("trash.deletedForever"));
    },
    onError: () => toast.error(t("trash.deleteError")),
  });

  const empty = useMutation({
    mutationFn: () => emptyTrash(effective),
    onSuccess: (r) => {
      qc.invalidateQueries({ queryKey: ["trash"] });
      setEmptying(false);
      toast.success(t("trash.purged", { count: r.purged }));
    },
    onError: () => toast.error(t("trash.emptyError")),
  });

  /** A restore that costs a cent or more asks first; anything less runs at once. */
  function startRestore(p: PendingRestore) {
    if (p.charge >= ONE_CENT) setPendingRestore(p);
    else restore.mutate(p);
  }

  function restoreOne(item: TrashItem) {
    startRestore({
      title: item.title,
      count: 1,
      charge: item.restore_charge_micro,
      days: daysIn(item),
      run: () => restoreTrashItem(item.kind, item.id),
    });
  }

  function restoreGroup(g: TrashGroup) {
    if (!g.batch) return restoreOne(g.items[0]);
    const batch = g.batch;
    startRestore({
      title: null,
      count: g.items.length,
      charge: g.items.reduce((sum, i) => sum + i.restore_charge_micro, 0),
      days: Math.max(...g.items.map(daysIn)),
      run: () => restoreTrashBatch(batch),
    });
  }

  const groups = groupByDeletion(items);
  const totalBytes = items.reduce((sum, i) => sum + i.size_bytes, 0);

  return (
    <Page
      title={t("trash.page.title")}
      actions={
        items.length > 0 && (
          <Button variant="ghost" size="sm" className="text-error" icon={<Trash2 className="h-4 w-4" />} onClick={() => setEmptying(true)}>
            {t("trash.empty")}
          </Button>
        )
      }
    >
      <p className="-mt-3 mb-5 max-w-2xl text-sm text-muted">{t("trash.intro")}</p>

      {isFamilyAdmin && (
        <SegmentedControl
          className="mb-5 w-fit"
          value={scope}
          onChange={setScope}
          segments={[
            { value: "mine", label: t("trash.scope.mine") },
            { value: "family", label: t("trash.scope.family") },
          ]}
        />
      )}

      {isLoading ? (
        <div className="space-y-2">
          {[0, 1, 2].map((i) => (
            <Skeleton key={i} className="h-14" />
          ))}
        </div>
      ) : items.length === 0 ? (
        <EmptyState
          icon={<Trash2 />}
          title={t("trash.emptyState.title")}
          description={t("trash.emptyState.description")}
        />
      ) : (
        <>
          <p className="mb-3 text-xs text-muted">
            {t("trash.summary", { items: t("trash.itemCount", { count: items.length }), size: formatBytes(totalBytes) })}
          </p>
          <div className="space-y-4">
            {groups.map((g) => (
              <section key={g.key} className="overflow-hidden rounded-card border border-border">
                <div className="flex items-center gap-3 bg-bg-alt px-4 py-2">
                  <span className="min-w-0 flex-1 truncate text-xs text-muted">
                    {deletedLine(g, me)}
                    {g.items.length > 1 && ` · ${t("trash.itemCount", { count: g.items.length })}`}
                  </span>
                  {g.items.length > 1 && g.batch && (
                    <Button size="sm" variant="ghost" icon={<RotateCcw className="h-4 w-4" />} onClick={() => restoreGroup(g)}>
                      {t("trash.restoreAll")}
                    </Button>
                  )}
                </div>
                {g.items.map((item) => (
                  <div key={`${item.kind}:${item.id}`} className="flex items-center gap-3 border-t border-border px-4 py-2.5">
                    <span aria-hidden="true">{KIND[item.kind].icon}</span>
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm">{item.title}</span>
                      <span className="block truncate text-xs text-muted">
                        {[
                          t(KIND[item.kind].label),
                          item.size_bytes > 0 ? formatBytes(item.size_bytes) : null,
                          item.owner.id !== me ? t("trash.item.owner", { name: item.owner.display_name ?? t("trash.item.someone") }) : null,
                          goneIn(item),
                          item.restore_charge_micro > 0 ? t("trash.item.restoreCost", { cost: cost(item.restore_charge_micro) }) : null,
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                      </span>
                    </span>
                    <Button size="sm" variant="secondary" icon={<RotateCcw className="h-4 w-4" />} onClick={() => restoreOne(item)}>
                      {t("trash.restore")}
                    </Button>
                    <IconButton size="sm" label={t("trash.deleteForever")} onClick={() => setPurging(item)}>
                      <Trash2 className="h-4 w-4" />
                    </IconButton>
                  </div>
                ))}
              </section>
            ))}
          </div>
        </>
      )}

      {pendingRestore && (
        <Dialog open onOpenChange={(v) => !v && setPendingRestore(null)}>
          <DialogContent
            title={
              pendingRestore.title != null
                ? t("trash.restoreDialog.titleOne", { title: pendingRestore.title })
                : t("trash.restoreDialog.titleMany", { count: pendingRestore.count })
            }
            description={t("trash.restoreDialog.description", { amount: money(pendingRestore.charge), days: pendingRestore.days })}
            footer={
              <>
                <Button variant="ghost" onClick={() => setPendingRestore(null)}>
                  {t("common.action.cancel")}
                </Button>
                <Button loading={restore.isPending} onClick={() => restore.mutate(pendingRestore)}>
                  {t("trash.restoreDialog.confirm", { amount: money(pendingRestore.charge) })}
                </Button>
              </>
            }
          >
            {null}
          </DialogContent>
        </Dialog>
      )}

      {purging && (
        <Dialog open onOpenChange={(v) => !v && setPurging(null)}>
          <DialogContent
            title={t("trash.purgeDialog.title", { title: purging.title })}
            description={t("trash.purgeDialog.description")}
            footer={
              <>
                <Button variant="ghost" onClick={() => setPurging(null)}>
                  {t("common.action.cancel")}
                </Button>
                <Button variant="danger" loading={purge.isPending} onClick={() => purge.mutate(purging)}>
                  {t("trash.deleteForever")}
                </Button>
              </>
            }
          >
            {null}
          </DialogContent>
        </Dialog>
      )}

      {emptying && (
        <Dialog open onOpenChange={(v) => !v && setEmptying(false)}>
          <DialogContent
            title={t("trash.emptyDialog.title")}
            description={t("trash.emptyDialog.description", { count: items.length })}
            footer={
              <>
                <Button variant="ghost" onClick={() => setEmptying(false)}>
                  {t("common.action.cancel")}
                </Button>
                <Button variant="danger" loading={empty.isPending} onClick={() => empty.mutate()}>
                  {t("trash.empty")}
                </Button>
              </>
            }
          >
            {null}
          </DialogContent>
        </Dialog>
      )}
    </Page>
  );
}

function daysIn(item: TrashItem): number {
  return Math.max(0, Math.floor((Date.now() - new Date(item.trashed_at).getTime()) / 86_400_000));
}

function goneIn(item: TrashItem): string {
  const d = daysLeft(item.purge_at);
  return d <= 1 ? t("trash.item.goneTomorrow") : t("trash.item.goneIn", { days: d });
}

function deletedLine(g: TrashGroup, me: string | undefined): string {
  const when = timeAgo(g.trashedAt);
  const by = g.items[0].trashed_by;
  if (!by?.display_name) return t("trash.group.deleted", { when });
  return by.id === me ? t("trash.group.deletedByYou", { when }) : t("trash.group.deletedBy", { when, name: by.display_name });
}
