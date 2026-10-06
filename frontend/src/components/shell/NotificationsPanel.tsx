// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useNavigate } from "react-router-dom";
import * as Popover from "@radix-ui/react-popover";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Bell } from "lucide-react";
import { ackNotifications, listNotifications } from "../../api/notifications";
import { grantContentAccess, type MediaKind } from "../../api/family";
import { restoreTrashItem, type TrashKind } from "../../api/trash";
import type { Notification } from "../../api/types";
import { toast } from "../ui";
import { timeAgo } from "../../lib/format";
import { Button } from "../ui/Button";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/**
 * The notification inbox.
 *
 * There is no push delivery server-side — every client polls this, which is
 * the design rather than a stopgap. Polling happens when the window regains
 * focus (TanStack Query's `refetchOnWindowFocus`) plus a slow interval, not on
 * a tight timer: nothing here is time-critical.
 *
 * Acknowledging is what removes a notification from the inbox, so it happens
 * when the panel is opened and the user has actually seen them — not on
 * arrival, which would empty the inbox for someone who never looked.
 */
const POLL_MS = 120_000;

/** The three fields `request_content_access` puts in `data`, loosely typed coming off the wire. */
function accessRequestPayload(n: Notification): { kind: MediaKind; itemId: string; requesterId: string } | null {
  if (n.kind !== "access_request" || !n.data) return null;
  const { media_kind, item_id, requester_id } = n.data as Record<string, unknown>;
  if (typeof media_kind !== "string" || typeof item_id !== "string" || typeof requester_id !== "string") {
    return null;
  }
  return { kind: media_kind as MediaKind, itemId: item_id, requesterId: requester_id };
}

/** `item_trashed`: a family admin moved one of your items to the trash. */
function trashedPayload(n: Notification): { kind: TrashKind; id: string } | null {
  if (n.kind !== "item_trashed" || !n.data) return null;
  const { kind, id } = n.data as Record<string, unknown>;
  if (typeof kind !== "string" || typeof id !== "string") return null;
  return { kind: kind as TrashKind, id };
}

/** `playlist_shared`: someone shared a playlist live, or sent you a copy of one. */
function playlistSharedPayload(n: Notification): { playlistId: string; from: string; mode: "live" | "copy" } | null {
  if (n.kind !== "playlist_shared" || !n.data) return null;
  const { playlist_id, from, mode } = n.data as Record<string, unknown>;
  if (typeof playlist_id !== "string" || typeof from !== "string") return null;
  return { playlistId: playlist_id, from, mode: mode === "copy" ? "copy" : "live" };
}

export default function NotificationsPanel({ collapsed = false }: { collapsed?: boolean }) {
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);
  const { t } = useT();
  const navigate = useNavigate();

  const { data: notifications = [] } = useQuery({
    queryKey: ["notifications"],
    queryFn: () => listNotifications(50),
    refetchInterval: POLL_MS,
    refetchOnWindowFocus: true,
    retry: false,
  });

  const ack = useMutation({
    mutationFn: (ids: string[]) => ackNotifications(ids),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["notifications"] }),
  });

  // One tap from the notification itself — the alternative was sending the admin to the item and
  // trusting them to find the same picker, for a decision that is almost always just "yes".
  const grant = useMutation({
    mutationFn: (payload: { kind: MediaKind; itemId: string; requesterId: string; notificationId: string }) =>
      grantContentAccess(payload.kind, payload.itemId, payload.requesterId),
    onSuccess: (audience, payload) => {
      qc.setQueryData(["audience", payload.kind, payload.itemId], audience);
      ack.mutate([payload.notificationId]);
    },
    onError: () => toast.error(t("shell.notifications.grantFailed")),
  });

  // Restoring from the notification itself — the owner's most likely answer
  // to "someone deleted your book" is "put it back".
  const restore = useMutation({
    mutationFn: (p: { kind: TrashKind; id: string; notificationId: string }) => restoreTrashItem(p.kind, p.id),
    onSuccess: (_r, p) => {
      void qc.invalidateQueries();
      ack.mutate([p.notificationId]);
      toast.success(t("common.trash.restored"));
    },
    onError: (_e, p) => {
      // Already restored or deleted for good — either way there is nothing left to do.
      ack.mutate([p.notificationId]);
      toast.show(t("shell.notifications.notInTrash"));
    },
  });

  const count = notifications.length;

  return (
    <Popover.Root open={open} onOpenChange={setOpen}>
      <Popover.Trigger asChild>
        <button
          aria-label={count > 0 ? t("shell.notifications.unread", { count }) : t("shell.notifications.title")}
          className={cn(
            "relative flex items-center rounded-[10px] text-sm font-medium transition-colors",
            collapsed ? "mx-auto h-10 w-10 justify-center" : "h-9 w-full gap-3 px-3",
            count > 0 ? "text-fg hover:bg-bg-alt" : "text-fg/80 hover:bg-bg-alt"
          )}
        >
          <Bell className="h-[18px] w-[18px] shrink-0 stroke-[1.75]" />
          {collapsed ? (
            // On the rail the count rides the bell rather than taking a column.
            count > 0 && (
              // A dot, not a number: three digits in a 40px square is noise,
              // and the exact count is one click away.
              <span className="absolute right-1.5 top-1.5 h-2 w-2 rounded-pill bg-accent ring-2 ring-bg-alt" />
            )
          ) : (
            <>
              {t("shell.notifications.title")}
              {count > 0 && (
                <span className="ml-auto inline-flex h-5 min-w-5 items-center justify-center rounded-pill bg-accent px-1.5 text-[11px] font-semibold text-on-accent">
                  {count}
                </span>
              )}
            </>
          )}
        </button>
      </Popover.Trigger>

      <Popover.Portal>
        <Popover.Content
          side="right"
          align="end"
          sideOffset={10}
          className="z-[60] max-h-96 w-80 overflow-y-auto rounded-card border border-border bg-card p-2 shadow-pop animate-pop-in"
        >
          <div className="mb-1 flex items-center justify-between px-1.5">
            <span className="text-xs font-semibold uppercase tracking-wide text-muted">{t("shell.notifications.title")}</span>
            {count > 0 && (
              <Button
                size="sm"
                variant="ghost"
                loading={ack.isPending}
                onClick={() => ack.mutate(notifications.map((n) => n.id))}
              >
                {t("shell.notifications.markAllRead")}
              </Button>
            )}
          </div>

          {count === 0 && <p className="px-2 py-6 text-center text-sm text-muted">{t("shell.notifications.empty")}</p>}

          {notifications.map((n) => {
            const request = accessRequestPayload(n);
            const trashed = trashedPayload(n);
            const shared = playlistSharedPayload(n);
            const granting = grant.isPending && grant.variables?.notificationId === n.id;
            return (
              <div key={n.id} className="group rounded-lg px-2 py-2 hover:bg-bg-alt">
                <div className="flex items-start gap-2">
                  <span className="min-w-0 flex-1">
                    {/* The server writes titles in English for every client; this one the web can say in the user's language. */}
                    <span className="block text-sm font-medium">
                      {shared ? t(shared.mode === "copy" ? "shell.notifications.playlistShared.copy" : "shell.notifications.playlistShared.live", { from: shared.from }) : n.title}
                    </span>
                    {n.body && <span className="mt-0.5 block text-xs leading-relaxed text-muted">{n.body}</span>}
                    <span className="mt-1 block text-[11px] text-muted">{timeAgo(n.created_at)}</span>
                  </span>
                  {!request && !trashed && (
                    <button
                      onClick={() => ack.mutate([n.id])}
                      className="shrink-0 text-xs text-muted opacity-0 transition-opacity hover:text-fg group-hover:opacity-100"
                    >
                      {t("common.action.dismiss")}
                    </button>
                  )}
                </div>
                {request && (
                  <div className="mt-1.5 flex items-center gap-2">
                    <Button
                      size="sm"
                      loading={granting}
                      onClick={() =>
                        grant.mutate({ kind: request.kind, itemId: request.itemId, requesterId: request.requesterId, notificationId: n.id })
                      }
                    >
                      {t("shell.notifications.grant")}
                    </Button>
                    <button onClick={() => ack.mutate([n.id])} className="text-xs text-muted hover:text-fg">
                      {t("common.action.dismiss")}
                    </button>
                  </div>
                )}
                {shared && (
                  <div className="mt-1.5 flex items-center gap-2">
                    <Button
                      size="sm"
                      onClick={() => {
                        ack.mutate([n.id]);
                        setOpen(false);
                        navigate(`/music/playlists/${shared.playlistId}`);
                      }}
                    >
                      {t("shell.notifications.open")}
                    </Button>
                  </div>
                )}
                {trashed && (
                  <div className="mt-1.5 flex items-center gap-2">
                    <Button
                      size="sm"
                      loading={restore.isPending && restore.variables?.notificationId === n.id}
                      onClick={() => restore.mutate({ ...trashed, notificationId: n.id })}
                    >
                      {t("common.action.restore")}
                    </Button>
                    <button onClick={() => ack.mutate([n.id])} className="text-xs text-muted hover:text-fg">
                      {t("common.action.dismiss")}
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
