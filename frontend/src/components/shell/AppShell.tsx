// SPDX-License-Identifier: AGPL-3.0-or-later
import { Suspense, useEffect, useState } from "react";
import { Outlet, useLocation } from "react-router-dom";
import { Menu as MenuIcon, Search, X } from "lucide-react";
import Sidebar from "./Sidebar";
import PlayerBar from "../player/PlayerBar";
import Logo, { Wordmark } from "../Logo";
import { IconButton } from "../ui/Button";
import { Tooltip, TooltipProvider } from "../ui/Tooltip";
import CommandPalette from "./CommandPalette";
import ShortcutsDialog from "./ShortcutsDialog";
import { startQueueSync, type ForeignQueue } from "../../lib/queueSync";
import ContinueElsewhere from "./ContinueElsewhere";
import OfflineBanner from "./OfflineBanner";
import { useSidebar } from "../../lib/sidebar";
import { useCommandPalette } from "../../lib/commandPalette";
import { useT } from "../../i18n";

/* Three-column shape on wide screens; below `lg` the sidebar becomes a
   drawer so the content column gets the whole width. The top bar over the
   content column is the one home of search at every width. */
export default function AppShell() {
  const [drawerOpen, setDrawerOpen] = useState(false);
  const toggleSidebar = useSidebar((s) => s.toggle);
  const openPalette = useCommandPalette((s) => s.setOpen);
  const { t } = useT();
  const [elsewhere, setElsewhere] = useState<ForeignQueue | null>(null);
  const location = useLocation();
  const [drawerOpenedAt, setDrawerOpenedAt] = useState(location.pathname);

  // Close the drawer on navigation, not on every click inside it — an
  // onClick on the whole panel used to fire before an interior control (the
  // notifications bell, upload indicator) got to open its own popover, so
  // the drawer closing raced it shut and the popover never appeared. Adjusted
  // during render (React's sanctioned way to reset state on a prop change)
  // rather than in an effect, so it can't itself cascade an extra render.
  if (location.pathname !== drawerOpenedAt) {
    setDrawerOpenedAt(location.pathname);
    if (drawerOpen) setDrawerOpen(false);
  }

  // ⌘B / Ctrl+B collapses the sidebar to a rail, the convention every editor uses.
  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "b") {
        e.preventDefault();
        toggleSidebar();
      }
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [toggleSidebar]);

  // Another device writing the shared queue is reported, never applied on its
  // own — silently replacing what someone started on their phone is worse than
  // doing nothing.
  useEffect(() => startQueueSync(setElsewhere), []);

  return (
    <TooltipProvider>
      <div className="flex h-dvh flex-col bg-bg text-fg">
        <a
          href="#main"
          className="sr-only focus:not-sr-only focus:absolute focus:left-3 focus:top-3 focus:z-[70] focus:rounded-pill focus:bg-accent focus:px-4 focus:py-2 focus:text-sm focus:text-on-accent"
        >
          {t("shell.skipToContent")}
        </a>
        <OfflineBanner />

        <div className="relative flex min-h-0 flex-1">
          <div className="hidden lg:block"><Sidebar /></div>

          {drawerOpen && (
            <div className="fixed inset-0 z-40 lg:hidden">
              <button aria-label={t("shell.topBar.closeMenu")} className="absolute inset-0 bg-overlay animate-fade-in" onClick={() => setDrawerOpen(false)} />
              <div className="absolute inset-y-0 left-0 shadow-pop animate-drawer-in">
                <Sidebar forceExpanded />
                <IconButton label={t("shell.topBar.closeMenu")} className="absolute right-2 top-3" onClick={() => setDrawerOpen(false)}><X className="h-5 w-5" /></IconButton>
              </div>
            </div>
          )}

          <div className="flex min-h-0 min-w-0 flex-1 flex-col">
            <header className="flex h-12 shrink-0 items-center gap-2 border-b border-border px-2 lg:px-4">
              <div className="flex shrink-0 items-center gap-2 lg:hidden">
                <IconButton label={t("shell.topBar.menu")} onClick={() => setDrawerOpen(true)}><MenuIcon className="h-5 w-5" /></IconButton>
                <Logo size={26} />
                <Wordmark className="text-[15px]" />
              </div>
              <Tooltip label={t("shell.topBar.searchShortcut")} side="bottom">
                <IconButton label={t("common.action.search")} className="ml-auto" onClick={() => openPalette(true)}>
                  <Search className="h-5 w-5" />
                </IconButton>
              </Tooltip>
            </header>
            <main id="main" className="min-h-0 min-w-0 flex-1">
              <Suspense fallback={null}>
                <Outlet />
              </Suspense>
            </main>
          </div>
        </div>

        <PlayerBar />
        {elsewhere && (
          <div className="pointer-events-none fixed bottom-24 right-4 z-50 w-80 max-w-[calc(100vw-2rem)]">
            <ContinueElsewhere queue={elsewhere} onDismiss={() => setElsewhere(null)} />
          </div>
        )}
        <CommandPalette />
        <ShortcutsDialog />
      </div>
    </TooltipProvider>
  );
}
