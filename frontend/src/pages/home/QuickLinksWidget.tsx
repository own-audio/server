// SPDX-License-Identifier: AGPL-3.0-or-later
import { Link } from "react-router-dom";
import { BookIcon, MusicIcon, PodcastIcon } from "../../components/ui/CloudIcon";
import { useT, type PlainKey } from "../../i18n";

/* The three libraries one tap away — on a phone the sidebar is a drawer, so
   without these every section is two taps and a reach to the top corner. */

const LINKS: { to: string; label: PlainKey; Icon: typeof BookIcon; tint: string; ink: string }[] = [
  { to: "/audiobooks", label: "common.kind.audiobooks", Icon: BookIcon, tint: "cloud-tint-book", ink: "text-book" },
  { to: "/podcasts", label: "common.kind.podcasts", Icon: PodcastIcon, tint: "cloud-tint-podcast", ink: "text-podcast" },
  { to: "/music", label: "common.kind.music", Icon: MusicIcon, tint: "cloud-tint-music", ink: "text-music" },
];

export function QuickLinksWidget() {
  const { t } = useT();
  return (
    <nav aria-label={t("home.quickLinks.label")} className="grid grid-cols-3 gap-2">
      {LINKS.map(({ to, label, Icon, tint, ink }) => (
        <Link
          key={to}
          to={to}
          className={`${tint} flex min-h-24 flex-col items-center justify-center gap-2 rounded-card border border-border px-2 py-4 transition-transform duration-150 hover:brightness-95 active:scale-[0.97] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent`}
        >
          <Icon className={`h-8 w-8 ${ink}`} />
          <span className="text-sm font-semibold">{t(label)}</span>
        </Link>
      ))}
    </nav>
  );
}
