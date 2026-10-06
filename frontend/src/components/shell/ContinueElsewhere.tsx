// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { Laptop, Monitor, Smartphone, X } from "lucide-react";
import { followQueue, type ForeignQueue } from "../../lib/queueSync";
import { Button, IconButton, toast } from "../ui";
import { formatClock } from "../../lib/time";
import { useT, type PlainKey } from "../../i18n";

const PLAYING_ON: Record<string, PlainKey> = {
  ios: "shell.continue.playingOnIos",
  android: "shell.continue.playingOnAndroid",
  macos: "shell.continue.playingOnMac",
  windows: "shell.continue.playingOnWindows",
  web: "shell.continue.playingOnWeb",
};

function deviceIcon(kind: string | null) {
  if (kind === "ios" || kind === "android") return <Smartphone className="h-4 w-4" />;
  if (kind === "macos" || kind === "windows") return <Laptop className="h-4 w-4" />;
  return <Monitor className="h-4 w-4" />;
}

/**
 * "You were listening on your phone — pick it up here."
 *
 * The queue is shared last-write-wins, so following it is always an explicit
 * choice: taking over automatically would stop whatever is actually playing on
 * the other device.
 *
 * `device_label` is shown when the other client set one. The web app never
 * sends its own — the contract says to omit it unless the user actually named
 * the device, and inventing "Chrome on macOS" is exactly the raw-model-id
 * behaviour that field warns against.
 */
export default function ContinueElsewhere({ queue, onDismiss }: { queue: ForeignQueue; onDismiss: () => void }) {
  const [busy, setBusy] = useState(false);
  const { t } = useT();

  const playingOn = queue.deviceLabel
    ? t("shell.continue.playingOnLabel", { device: queue.deviceLabel })
    : t(PLAYING_ON[queue.device ?? ""] ?? "shell.continue.playingOnOther");

  async function follow() {
    setBusy(true);
    try {
      await followQueue(queue);
      onDismiss();
    } catch {
      toast.error(t("shell.continue.failed"), t("shell.continue.failedHint"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div
      role="status"
      className="pointer-events-auto flex items-start gap-3 rounded-card border border-border bg-card p-3 shadow-pop animate-pop-in"
    >
      <span className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-pill bg-accent/12 text-accent">
        {deviceIcon(queue.device)}
      </span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{playingOn}</p>
        <p className="mt-0.5 text-xs text-muted">
          {queue.positionSecs > 0
            ? t("shell.continue.itemsPaused", { count: queue.itemCount, time: formatClock(queue.positionSecs) })
            : t("shell.continue.items", { count: queue.itemCount })}
        </p>
        <Button size="sm" className="mt-2" loading={busy} onClick={follow}>
          {t("shell.continue.here")}
        </Button>
      </div>
      <IconButton size="sm" label={t("common.action.dismiss")} onClick={onDismiss}>
        <X className="h-4 w-4" />
      </IconButton>
    </div>
  );
}
