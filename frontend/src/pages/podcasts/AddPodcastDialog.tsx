// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { Search } from "lucide-react";
import { PodcastIcon } from "../../components/ui/CloudIcon";
import { browseCategory, listCategories, listFeeds, searchPodcasts, subscribeFeed } from "../../api/podcasts";
import type { Visibility } from "../../api/types";
import { useDebounce } from "../../lib/useDebounce";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Dialog, DialogContent, Input, SearchField, Skeleton, toast } from "../../components/ui";
import type { PodcastSearchResult } from "../../api/types";
import { cn } from "../../lib/cn";
import { useServerFeatures } from "../../lib/features";
import { stripHtml } from "../../lib/format";
import { useT } from "../../i18n";

/* Three ways in: browse a category, search, or paste a URL for a show the
   catalogue misses (or a YouTube channel).

   Browsing lives in the empty state of the search box, which used to just
   say "type at least two characters". That is the cold-start surface — it
   works for someone who has followed nothing and cannot yet be asked what
   they like. Nothing here reads listening history, so none of it is
   behind the recommendations switch.

   Search is debounced but no longer because a third party is being
   called: since 2026-08-29 it runs against the self-hosted catalogue and
   the query never leaves the deployment. */
export default function AddPodcastDialog() {
  const navigate = useNavigate();
  const { t, locale } = useT();
  const qc = useQueryClient();
  const [query, setQuery] = useState("");
  const [category, setCategory] = useState<string | null>(null);
  const [url, setUrl] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<string | null>(null);
  // Private by default: sharing with the family is a deliberate act, never the
  // fallback for someone who didn't read the form.
  const [visibility, setVisibility] = useState<Visibility>("private");
  const debounced = useDebounce(query.trim(), 300);

  const { data: feeds = [] } = useQuery({ queryKey: ["feeds"], queryFn: listFeeds });
  const followed = new Set(feeds.map((f) => f.feed_url));

  const searching = debounced.length >= 2;
  // Categories and browse need the server's catalogue; search alone may be
  // Apple's directory. A server older than revision 5 has no
  // `podcast_search`, and its discovery includes search.
  const { features } = useServerFeatures();
  const canSearch = features.podcast_search || features.podcast_discovery;

  const { data: results = [], isFetching, isError } = useQuery({
    queryKey: ["podcast-search", debounced],
    queryFn: () => searchPodcasts(debounced),
    enabled: searching && canSearch,
    retry: false,
  });

  // Only fetched while the search box is empty, so opening the dialog to
  // paste a URL costs nothing.
  const { data: categories = [] } = useQuery({
    queryKey: ["podcast-categories"],
    queryFn: listCategories,
    enabled: !searching && features.podcast_discovery,
    staleTime: 60 * 60 * 1000, // the catalogue is reloaded weekly
    retry: false,
  });

  const { data: browsed = [], isFetching: browsing } = useQuery({
    queryKey: ["podcast-browse", category],
    queryFn: () => browseCategory(category!),
    enabled: !searching && !!category && features.podcast_discovery,
    retry: false,
  });

  // One list, whichever way the user got here.
  const shown: PodcastSearchResult[] = searching ? results : category ? browsed : [];

  const subscribe = useMutation({
    mutationFn: (feedUrl: string) => subscribeFeed(feedUrl, visibility),
    onSuccess: (feed) => {
      qc.invalidateQueries({ queryKey: ["feeds"] });
      toast.success(t("podcasts.action.following"), feed.title);
      navigate(`/podcasts/${feed.id}`, { replace: true });
    },
    onError: (err) => { setPending(null); setError(apiErrorMessage(err, t("podcasts.add.error.follow"))); },
  });

  function follow(feedUrl: string) {
    setError(null);
    setPending(feedUrl);
    subscribe.mutate(feedUrl);
  }

  return (
    <Dialog open onOpenChange={(v) => { if (!v) navigate("/podcasts"); }}>
      <DialogContent title={t("podcasts.add.title")} description={t("podcasts.add.description")} className="sm:max-w-2xl">
        <SearchField autoFocus value={query} onChange={(e) => setQuery(e.target.value)} placeholder={t("podcasts.add.searchPlaceholder")} className="w-full" aria-label={t("podcasts.add.searchLabel")} />

        <div className="mt-3 min-h-32">
          {!searching && categories.length > 0 && (
            <div className="mb-3 flex flex-wrap gap-1.5">
              {/* active_count, not feed_count: three quarters of the
                  catalogue last published over a year ago, and a category
                  boasting 926,824 shows when 165,822 are alive is a lie
                  told in the user's favour. */}
              {categories.slice(0, 18).map((c) => (
                <button
                  key={c.category}
                  type="button"
                  onClick={() => setCategory(category === c.category ? null : c.category)}
                  className={cn(
                    "rounded-full border px-2.5 py-1 text-xs capitalize transition-colors",
                    category === c.category
                      ? "border-transparent bg-[var(--accent)] text-white"
                      : "border-border text-muted hover:text-fg"
                  )}
                >
                  {c.category}
                  <span className="ml-1.5 opacity-60">{c.active_count.toLocaleString(locale)}</span>
                </button>
              ))}
            </div>
          )}

          {!searching && !category && (
            <p className="px-1 py-6 text-center text-sm text-muted">
              {t("podcasts.add.browseHint")}
            </p>
          )}
          {(isFetching || browsing) && <div className="space-y-2">{[0, 1, 2].map((i) => <Skeleton key={i} className="h-20" />)}</div>}
          {isError && <p className="px-1 py-6 text-center text-sm text-muted">{t("podcasts.add.catalogueDown")}</p>}
          {!isFetching && !isError && searching && results.length === 0 && <p className="px-1 py-6 text-center text-sm text-muted">{t("podcasts.add.nothingFound", { query: debounced })}</p>}
          {!browsing && !searching && category && browsed.length === 0 && (
            <p className="px-1 py-6 text-center text-sm text-muted">{t("podcasts.add.categoryEmpty")}</p>
          )}

          <ul className="space-y-2">
            {shown.map((r: PodcastSearchResult) => {
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
                      <Button size="sm" variant={already ? "secondary" : "primary"} disabled={already} loading={pending === r.feed_url && subscribe.isPending} onClick={() => follow(r.feed_url)}>
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
        </div>

        <form
          className="mt-5 border-t border-border pt-4"
          onSubmit={(e) => { e.preventDefault(); if (url.trim()) follow(url.trim()); }}
        >
          <Input
            label={t("podcasts.add.urlLabel")}
            type="url"
            value={url}
            onChange={(e) => setUrl(e.target.value)}
            placeholder={t("podcasts.add.urlPlaceholder")}
            hint={t("podcasts.add.urlHint")}
          />
          <label className="mt-4 flex items-start gap-2.5">
            <input
              type="checkbox"
              checked={visibility === "family"}
              onChange={(e) => setVisibility(e.target.checked ? "family" : "private")}
              className="mt-0.5 h-4 w-4 accent-[var(--accent)]"
            />
            <span className="text-sm">
              {t("podcasts.shareWithFamily")}
              <span className="block text-xs text-muted">{t("podcasts.add.shareHint")}</span>
            </span>
          </label>

          <div className="mt-3 flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={() => navigate("/podcasts")}>{t("common.action.cancel")}</Button>
            <Button type="submit" icon={<Search className="h-4 w-4" />} disabled={!url.trim()} loading={pending === url.trim() && subscribe.isPending}>{t("podcasts.action.follow")}</Button>
          </div>
        </form>

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
