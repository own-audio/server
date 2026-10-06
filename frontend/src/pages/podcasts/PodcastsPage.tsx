// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useLocation, useNavigate, useParams } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { CheckSquare, Compass, Plus, RefreshCw, Trash2 } from "lucide-react";
import { PodcastIcon } from "../../components/ui/CloudIcon";
import { deleteFeed, listFeeds, refreshFeed, setFeedVisibility } from "../../api/podcasts";
import { timeAgo } from "../../lib/format";
import { useMyPermissions } from "../../lib/permissions";
import { useT, type PlainKey } from "../../i18n";
import { SplitView, ColumnHeader } from "../../components/shell/SplitView";
import SectionTheme from "../../components/shell/SectionTheme";
import { MediaCard, MediaGrid, MediaRow } from "../../components/library/MediaCard";
import { ViewToggle, SortMenu, FilterField, FilterChips, MoreMenuTrigger } from "../../components/library/BrowseControls";
import { BatchBar } from "../../components/library/BatchBar";
import { useSelection } from "../../lib/useSelection";
import { usePersistedState } from "../../lib/persistedState";
import {
  Button,
  Dialog,
  DialogContent,
  EmptyState,
  IconButton,
  MenuItem,
  MenuSeparator,
  Skeleton,
  toast,
} from "../../components/ui";
import FeedDetail from "./FeedDetail";
import AddPodcastDialog from "./AddPodcastDialog";
import SimilarShowsDialog from "./SimilarShowsDialog";

type Sort = "recent" | "title" | "author";
const SORTS: { value: Sort; label: PlainKey }[] = [
  { value: "recent", label: "podcasts.list.sort.recent" },
  { value: "title", label: "podcasts.list.sort.title" },
  { value: "author", label: "podcasts.list.sort.author" },
];

export default function PodcastsPage() {
  const { feedId } = useParams<{ feedId: string }>();
  const adding = useLocation().pathname === "/podcasts/add";
  const navigate = useNavigate();
  const { t } = useT();
  const qc = useQueryClient();
  const [view, setView] = usePersistedState<"grid" | "list">("own-audio-feeds-view", "grid");
  const [sort, setSort] = usePersistedState<Sort>("own-audio-feeds-sort", "recent");
  const [owner, setOwner] = usePersistedState<"all" | "mine">("own-audio-feeds-owner", "all");
  const [filter, setFilter] = useState("");
  // The show whose "Similar shows" dialog is open, title included so the
  // dialog can name it without waiting for a fetch.
  const [similarFor, setSimilarFor] = useState<{ id: string; title: string } | null>(null);
  // Same shape, for the unfollow confirmation. A themed Dialog rather than
  // window.confirm(): the native dialog blocks the page's JS thread, which
  // pauses any audio element mid-playback — this one doesn't.
  const [confirmUnfollow, setConfirmUnfollow] = useState<{ id: string; title: string } | null>(null);
  const { canUpload } = useMyPermissions();
  const selection = useSelection();

  const { data: feeds = [], isLoading } = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });

  const refresh = useMutation({
    mutationFn: refreshFeed,
    onSuccess: () => { qc.invalidateQueries({ queryKey: ["feeds"] }); toast.success(t("podcasts.toast.refreshed")); },
    onError: () => toast.error(t("podcasts.error.refresh")),
  });
  const unsubscribe = useMutation({
    mutationFn: deleteFeed,
    onSuccess: (_d, id) => {
      qc.invalidateQueries({ queryKey: ["feeds"] });
      if (id === feedId) navigate("/podcasts");
      setConfirmUnfollow(null);
    },
    onError: () => toast.error(t("podcasts.list.error.unfollow")),
  });

  const visible = useMemo(() => {
    const q = filter.trim().toLowerCase();
    let list = feeds.filter((f) => !q || f.title.toLowerCase().includes(q) || (f.author ?? "").toLowerCase().includes(q));
    if (owner === "mine") list = list.filter((f) => f.visibility === "private");
    return [...list].sort((a, b) => {
      if (sort === "title") return a.title.localeCompare(b.title);
      if (sort === "author") return (a.author ?? "").localeCompare(b.author ?? "");
      return (b.last_refreshed_at ?? "").localeCompare(a.last_refreshed_at ?? "");
    });
  }, [feeds, filter, owner, sort]);

  const content = (
    <>
      <ColumnHeader
        title={t("common.kind.podcasts")}
        actions={
          <>
            <IconButton
              size="sm"
              label={selection.count > 0 ? t("podcasts.list.clearSelection") : t("podcasts.list.selectAll")}
              active={selection.count > 0}
              onClick={() => (selection.count > 0 ? selection.clear() : selection.selectAll(visible.map((f) => f.id)))}
            >
              <CheckSquare className="h-4 w-4" />
            </IconButton>
            {canUpload && <Button size="sm" icon={<Plus className="h-4 w-4" />} onClick={() => navigate("/podcasts/add")}>{t("podcasts.action.follow")}</Button>}
          </>
        }
      >
        <div className="flex items-center gap-1.5">
          <FilterField value={filter} onChange={setFilter} placeholder={t("podcasts.list.filter")} className="min-w-0 flex-1" />
          <SortMenu value={sort} onChange={setSort} options={SORTS.map((o) => ({ value: o.value, label: t(o.label) }))} />
          <ViewToggle value={view} onChange={setView} />
        </div>
        <FilterChips<"all" | "mine">
          value={owner}
          onChange={setOwner}
          chips={[
            { value: "all", label: t("podcasts.list.owner.all") },
            { value: "mine", label: t("podcasts.list.owner.mine") },
          ]}
        />
      </ColumnHeader>

      {isLoading && <MediaGrid>{[...Array(6)].map((_, i) => <Skeleton key={i} className="aspect-square" />)}</MediaGrid>}

      {!isLoading && visible.length === 0 && (
        <EmptyState
          icon={<PodcastIcon />}
          title={filter ? t("podcasts.list.empty.noMatchTitle") : owner === "mine" ? t("podcasts.list.empty.noPrivateTitle") : t("podcasts.list.empty.noneTitle")}
          description={
            filter
              ? t("podcasts.list.empty.noMatchBody")
              : owner === "mine"
                ? t("podcasts.list.empty.noPrivateBody")
                : t("podcasts.list.empty.noneBody")
          }
          action={!filter && owner === "all" && canUpload && <Button onClick={() => navigate("/podcasts/add")}>{t("podcasts.list.findShow")}</Button>}
        />
      )}

      {!isLoading && visible.length > 0 && (() => {
        const props = (f: (typeof visible)[number]) => ({
          kind: "podcast" as const,
          title: f.title,
          subtitle: f.author,
          cover: f.image_url,
          coverAuth: false,
          family: f.visibility === "family",
          translatable: f.has_transcripts,
          selected: selection.count > 0 ? selection.has(f.id) : f.id === feedId,
          onClick: () => (selection.count > 0 ? selection.toggle(f.id) : navigate(`/podcasts/${f.id}`)),
          menu: (
            <MoreMenuTrigger variant={view === "grid" ? "overlay" : "plain"}>
              <MenuItem icon={<CheckSquare />} onSelect={() => selection.toggle(f.id)}>{t("podcasts.action.select")}</MenuItem>
              <MenuItem icon={<RefreshCw />} onSelect={() => refresh.mutate(f.id)}>{t("podcasts.action.refresh")}</MenuItem>
              <MenuItem icon={<Compass />} onSelect={() => setSimilarFor({ id: f.id, title: f.title })}>{t("podcasts.action.similarShows")}</MenuItem>
              <MenuSeparator />
              <MenuItem destructive icon={<Trash2 />} disabled={!f.is_owner} onSelect={() => setConfirmUnfollow({ id: f.id, title: f.title })}>{t("podcasts.action.unfollow")}</MenuItem>
            </MoreMenuTrigger>
          ),
        });
        return view === "grid" ? (
          <MediaGrid>{visible.map((f) => <MediaCard key={f.id} {...props(f)} meta={f.last_refreshed_at ? t("podcasts.list.updated", { when: timeAgo(f.last_refreshed_at) }) : undefined} />)}</MediaGrid>
        ) : (
          <div className="p-3">{visible.map((f) => <MediaRow key={f.id} {...props(f)} trailing={f.last_refreshed_at ? timeAgo(f.last_refreshed_at) : undefined} />)}</div>
        );
      })()}

      <BatchBar
        selected={visible.filter((f) => selection.has(f.id)).map((f) => ({ id: f.id, label: f.title, isOwner: f.is_owner }))}
        onClear={selection.clear}
        onSetVisibility={async (id, v) => {
          await setFeedVisibility(id, v);
          qc.invalidateQueries({ queryKey: ["feeds"] });
        }}
        onDelete={async (id) => {
          await deleteFeed(id);
          qc.invalidateQueries({ queryKey: ["feeds"] });
        }}
      />
    </>
  );

  return (
    <SectionTheme cloud="podcast">
      {adding && <AddPodcastDialog />}
      {similarFor && (
        <SimilarShowsDialog
          key={similarFor.id}
          feedId={similarFor.id}
          feedTitle={similarFor.title}
          onClose={() => setSimilarFor(null)}
        />
      )}
      <Dialog open={!!confirmUnfollow} onOpenChange={(v) => !v && setConfirmUnfollow(null)}>
        <DialogContent
          title={t("podcasts.list.unfollowConfirm.title", { title: confirmUnfollow?.title ?? "" })}
          footer={
            <>
              <Button variant="ghost" onClick={() => setConfirmUnfollow(null)}>
                {t("common.action.cancel")}
              </Button>
              <Button
                variant="danger"
                loading={unsubscribe.isPending}
                onClick={() => confirmUnfollow && unsubscribe.mutate(confirmUnfollow.id)}
              >
                {t("podcasts.action.unfollow")}
              </Button>
            </>
          }
        >
          <p className="text-sm text-muted">{t("podcasts.list.unfollowConfirm.body")}</p>
        </DialogContent>
      </Dialog>
    <SplitView
      contentWidth="narrow"
      widthKey="podcasts"
      hasDetail={!!feedId}
      onBack={() => navigate("/podcasts")}
      content={content}
      detail={feedId ? <FeedDetail feedId={feedId} /> : <EmptyState icon={<PodcastIcon />} title={t("podcasts.list.pickShow.title")} description={t("podcasts.list.pickShow.body")} />}
    />
    </SectionTheme>
  );
}
