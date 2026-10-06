// SPDX-License-Identifier: AGPL-3.0-or-later
import { Upload, X } from "lucide-react";
import * as Popover from "@radix-ui/react-popover";
import { useUploadQueue } from "../../lib/uploadQueue";
import { IconButton } from "../ui/Button";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

/** Shows what is uploading while the user is somewhere else in the app. */
export default function UploadIndicator({ collapsed = false }: { collapsed?: boolean }) {
  const { items, active, remove, clearFinished } = useUploadQueue();
  const { t } = useT();
  if (items.length === 0) return null;

  const inFlight = items.filter((i) => i.status === "uploading" || i.status === "queued").length;
  const failed = items.filter((i) => i.status === "failed").length;

  return (
    <Popover.Root>
      <Popover.Trigger asChild>
        <button
          aria-label={
            inFlight > 0
              ? t("shell.uploads.uploading", { count: inFlight })
              : failed > 0
                ? t("shell.uploads.failedLong", { count: failed })
                : t("shell.uploads.title")
          }
          className={cn(
            "flex items-center rounded-[10px] text-sm font-medium transition-colors",
            collapsed ? "mx-auto h-10 w-10 justify-center" : "h-9 w-full gap-3 px-3",
            failed > 0 ? "text-error hover:bg-error/10" : active ? "text-accent hover:bg-accent/10" : "text-fg/80 hover:bg-bg-alt"
          )}
        >
          <Upload className={cn("h-[18px] w-[18px] shrink-0 stroke-[1.75]", active && "animate-pulse")} />
          {!collapsed &&
            (inFlight > 0
              ? t("shell.uploads.uploading", { count: inFlight })
              : failed > 0
                ? t("shell.uploads.failedShort", { count: failed })
                : t("shell.uploads.title"))}
        </button>
      </Popover.Trigger>
      <Popover.Portal>
        <Popover.Content
          side="top"
          align="start"
          sideOffset={8}
          className="z-[60] max-h-80 w-72 overflow-y-auto rounded-card border border-border bg-card p-2 shadow-pop animate-pop-in"
        >
          <div className="mb-1 flex items-center justify-between px-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-muted">{t("shell.uploads.title")}</span>
            <button onClick={clearFinished} className="text-xs text-muted hover:text-fg">
              {t("shell.uploads.clearFinished")}
            </button>
          </div>
          {items.map((i) => (
            <div key={i.id} className="flex items-center gap-2 rounded-lg px-1.5 py-1.5 hover:bg-bg-alt">
              <span className="min-w-0 flex-1">
                <span className="block truncate text-sm">{i.label}</span>
                <span className={cn("block text-xs", i.status === "failed" ? "text-error" : "text-muted")}>
                  {i.status === "uploading"
                    ? t("shell.uploads.progress", { progress: Math.round(i.progress * 100) / 100 })
                    : i.status === "done"
                      ? t("common.action.done")
                      : i.status === "failed"
                        ? t("shell.uploads.failedRetry", { error: i.error ?? t("shell.uploads.failed") })
                        : t("shell.uploads.waiting")}
                </span>
              </span>
              {i.status !== "uploading" && (
                <IconButton size="sm" label={t("shell.uploads.dismissItem", { name: i.label })} onClick={() => remove(i.id)}>
                  <X className="h-3.5 w-3.5" />
                </IconButton>
              )}
            </div>
          ))}
        </Popover.Content>
      </Popover.Portal>
    </Popover.Root>
  );
}
