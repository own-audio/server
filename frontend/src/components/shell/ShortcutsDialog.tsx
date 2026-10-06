// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect } from "react";
import { Dialog, DialogContent } from "../ui";
import { useShortcuts } from "../../lib/shortcuts";
import { useT, type PlainKey } from "../../i18n";

/* Keys are shown as written; `keyLabel` is used instead for the few that are
   words rather than key names. */
interface Shortcut {
  keys: string;
  keyLabel?: PlainKey;
  what: PlainKey;
}

const GROUPS: { title: PlainKey; items: Shortcut[] }[] = [
  {
    title: "shell.shortcuts.group.anywhere",
    items: [
      { keys: "⌘K / Ctrl K", what: "shell.search.title" },
      { keys: "⌘B / Ctrl B", what: "shell.shortcuts.toggleSidebar" },
      { keys: "?", what: "shell.shortcuts.thisList" },
    ],
  },
  {
    title: "shell.shortcuts.group.resizing",
    items: [
      { keys: "drag", keyLabel: "shell.shortcuts.key.drag", what: "shell.shortcuts.setWidth" },
      { keys: "← / →", what: "shell.shortcuts.nudge" },
      { keys: "dblclick", keyLabel: "shell.shortcuts.key.doubleClick", what: "shell.shortcuts.defaultWidth" },
    ],
  },
  {
    title: "shell.shortcuts.group.playing",
    items: [
      { keys: "space", keyLabel: "shell.shortcuts.key.space", what: "shell.shortcuts.playPause" },
      { keys: "→ / ←", what: "shell.shortcuts.skip" },
      { keys: "Shift → / ←", what: "shell.shortcuts.nextPrevious" },
      { keys: "↑ / ↓", what: "player.volume" },
      { keys: "M", what: "player.mute" },
      { keys: "S", what: "player.shuffle" },
      { keys: "R", what: "player.repeat" },
    ],
  },
];

/** Opened with `?`, which is why the handler ignores typing in a field. */
export default function ShortcutsDialog() {
  const { open, setOpen } = useShortcuts();
  const { t } = useT();

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return;
      if ((el as HTMLElement | null)?.isContentEditable) return;
      if (e.key === "?" && !e.metaKey && !e.ctrlKey) {
        e.preventDefault();
        setOpen(!useShortcuts.getState().open);
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [setOpen]);

  return (
    <Dialog open={open} onOpenChange={setOpen}>
      <DialogContent title={t("shell.shortcuts.title")} description={t("shell.shortcuts.description")}>
        <div className="space-y-5">
          {GROUPS.map((g) => (
            <section key={g.title}>
              <p className="mb-2 text-xs font-semibold uppercase tracking-wide text-muted">{t(g.title)}</p>
              <dl className="space-y-1.5">
                {g.items.map(({ keys, keyLabel, what }) => (
                  <div key={keys} className="flex items-center gap-3">
                    <dt className="w-32 shrink-0">
                      <kbd className="rounded-md border border-border bg-bg-alt px-1.5 py-0.5 text-[11px] font-medium">{keyLabel ? t(keyLabel) : keys}</kbd>
                    </dt>
                    <dd className="text-sm text-muted">{t(what)}</dd>
                  </div>
                ))}
              </dl>
            </section>
          ))}
        </div>
      </DialogContent>
    </Dialog>
  );
}
