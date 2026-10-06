// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";

import { PodcastIcon } from "../../components/ui/CloudIcon";
import { listFeeds, similarFeeds, subscribeFeed } from "../../api/podcasts";
import type { PodcastSearchResult, Visibility } from "../../api/types";
import { stripHtml } from "../../lib/format";
import { Button, Dialog, DialogContent, Skeleton, toast } from "../../components/ui";
import { useT } from "../../i18n";

interface Props {
  feedId: string;
  feedTitle: string;
  onClose: () => void;
}

/**
 * "Shows like this one" with room to browse — opened from a show's ⋯ menu,
 * for someone who liked one show and wants more of the same.
 *
 * The feed page already ends in the same list, but only as eight cards
 * below a full episode list, which nobody scrolls to on purpose. This is
 * the same query given a surface of its own: everything the catalogue
 * returns, followable without leaving the show it started from.
 *
 * Reads no listening history — only the categories of the feed whose menu
 * was opened — so it sits outside the recommendations switch, exactly like
 * the inline section. See docs/podcast-recommendations-plan.md §0.
 */
export default function SimilarShowsDialog({ feedId, feedTitle, onClose }: Props) {
  const { t } = useT();
  const qc = useQueryClient();
  const [pending, setPending] = useState<string | null>(null);
  const [visibility, setVisibility] = useState<Visibility>("private");

  // Same key as the feed page's inline section, so opening this from a show
  // already on screen costs no second request.
  const { data: similar = [], isLoading, isError } = useQuery({
    queryKey: ["podcast-similar", feedId],
    queryFn: () => similarFeeds(feedId),
    staleTime: 60 * 60 * 1000,
    retry: false,
  });

  const { data: feeds = [] } = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });
  const followed = new Set(feeds.map((f) => f.feed_url));

  const subscribe = useMutation({
    mutationFn: (feedUrl: string) => subscribeFeed(feedUrl, visibility),
    onSuccess: (feed) => {
      // Deliberately not invalidating "podcast-similar" here: the server
      // filters out what the household follows, so a refetch would pull the
      // row out from under the cursor mid-browse. It happens on close.
      qc.invalidateQueries({ queryKey: ["feeds"] });
      toast.success(t("podcasts.action.following"), feed.title);
      setPending(null);
    },
    onError: () => { setPending(null); toast.error(t("podcasts.error.followShow")); },
  });

  function close() {
    qc.invalidateQueries({ queryKey: ["podcast-similar"] });
    onClose();
  }

  return (
    <Dialog open onOpenChange={(v) => { if (!v) close(); }}>
      <DialogContent
        title={t("podcasts.similar.title")}
        description={t("podcasts.similar.description", { title: feedTitle })}
        className="sm:max-w-2xl"
      >
        {isLoading && <div className="space-y-2">{[0, 1, 2].map((i) => <Skeleton key={i} className="h-20" />)}</div>}

        {isError && (
          <p className="px-1 py-6 text-center text-sm text-muted">{t("podcasts.similar.catalogueDown")}</p>
        )}

        {!isLoading && !isError && similar.length === 0 && (
          <p className="px-1 py-6 text-center text-sm text-muted">
            {t("podcasts.similar.empty")}
          </p>
        )}

        {similar.length > 0 && (
          <ul className="space-y-2">
            {similar.map((r: PodcastSearchResult) => {
              const already = followed.has(r.feed_url);
              return (
                <li key={r.feed_url} className="flex gap-3 rounded-card border border-border p-3">
                  {r.image_url ? (
                    <img src={r.image_url} alt="" loading="lazy" className="h-16 w-16 shrink-0 rounded-lg object-cover" />
                  ) : (
                    <span className="flex h-16 w-16 shrink-0 items-center justify-center rounded-lg cloud-tint-podcast text-podcast"><PodcastIcon className="h-6 w-6" /></span>
                  )}
                  <div className="min-w-0 flex-1">
                    <div className="flex items-start justify-between gap-3">
                      <div className="min-w-0">
                        <p className="truncate text-sm font-medium">{r.title}</p>
                        {r.author && <p className="truncate text-xs text-muted">{r.author}</p>}
                      </div>
                      <Button
                        size="sm"
                        variant={already ? "secondary" : "primary"}
                        disabled={already}
                        loading={pending === r.feed_url && subscribe.isPending}
                        onClick={() => { setPending(r.feed_url); subscribe.mutate(r.feed_url); }}
                      >
                        {already ? t("podcasts.action.following") : t("podcasts.action.follow")}
                      </Button>
                    </div>
                    {r.description && <p className="mt-1 line-clamp-2 text-xs text-muted">{stripHtml(r.description)}</p>}
                    <div className="mt-1.5 flex gap-3 text-[11px] text-muted">
                      {r.language && <span>{r.language.toUpperCase()}</span>}
                      {r.episode_count != null && <span>{t("podcasts.episodeCount", { count: r.episode_count })}</span>}
                    </div>
                  </div>
                </li>
              );
            })}
          </ul>
        )}

        {similar.length > 0 && (
          <label className="mt-4 flex items-start gap-2.5 border-t border-border pt-4">
            <input
              type="checkbox"
              checked={visibility === "family"}
              onChange={(e) => setVisibility(e.target.checked ? "family" : "private")}
              className="mt-0.5 h-4 w-4 accent-[var(--accent)]"
            />
            <span className="text-sm">
              {t("podcasts.shareWithFamily")}
              <span className="block text-xs text-muted">{t("podcasts.similar.shareHint")}</span>
            </span>
          </label>
        )}
      </DialogContent>
    </Dialog>
  );
}
