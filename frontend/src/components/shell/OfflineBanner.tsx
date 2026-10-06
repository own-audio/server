// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from "react";
import { Link, useLocation } from "react-router-dom";
import { WifiOff } from "lucide-react";
import { useT } from "../../i18n";

/* Offline, every server-backed view is empty or stale, and nothing says why.
   One line saying so — and pointing at the one view that still works — beats
   a dozen empty states that each look like the library vanished. */
export default function OfflineBanner() {
  const [online, setOnline] = useState(() => navigator.onLine);
  const { pathname } = useLocation();
  const { t } = useT();

  useEffect(() => {
    const up = () => setOnline(true);
    const down = () => setOnline(false);
    window.addEventListener("online", up);
    window.addEventListener("offline", down);
    return () => {
      window.removeEventListener("online", up);
      window.removeEventListener("offline", down);
    };
  }, []);

  if (online) return null;
  return (
    <div role="status" className="flex shrink-0 items-center justify-center gap-2 border-b border-border bg-bg-alt px-3 py-1.5 text-xs text-muted">
      <WifiOff className="h-3.5 w-3.5" />
      <span>{t("shell.offline.message")}</span>
      {pathname !== "/music/downloads" && (
        <Link to="/music/downloads" className="font-medium text-accent hover:underline">
          {t("shell.offline.downloads")}
        </Link>
      )}
    </div>
  );
}
