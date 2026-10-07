// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from "react";
import { NavLink, useMatch, useNavigate, useLocation } from "react-router-dom";
import { BarChart3, Home, Keyboard, LogOut, ChevronsLeft, ChevronsRight, PlayCircle, Settings, Sparkles, User, UserRound, Users, Trash2, Wallet, Languages } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../ui/CloudIcon";
import Logo, { Wordmark } from "../Logo";
import ThemeToggle from "./ThemeToggle";
import { Menu, MenuTrigger, MenuContent, MenuItem, MenuSeparator } from "../ui/Menu";
import { Tooltip } from "../ui/Tooltip";
import { IconButton } from "../ui/Button";
import { useAuthStore } from "../../store/authStore";
import { logout } from "../../api/auth";
import { useMyPermissions } from "../../lib/permissions";
import { useServerFeatures } from "../../lib/features";
import { useShortcuts } from "../../lib/shortcuts";
import { useSidebar } from "../../lib/sidebar";
import { usePlayerStore } from "../../store/playerStore";
import NowPlayingGlyph from "./NowPlayingGlyph";
import UploadIndicator from "./UploadIndicator";
import NotificationsPanel from "./NotificationsPanel";
import { cn } from "../../lib/cn";
import { clearDownloads } from "../../lib/offline/downloads";
import { t, type PlainKey } from "../../i18n";

type Cloud = "book" | "podcast" | "music";

interface Item {
  to: string;
  label: PlainKey;
  icon: React.ReactNode;
  cloud?: Cloud;
  end?: boolean;
  /** A small pill after the label ("New"), hidden on the collapsed rail. */
  badge?: string;
}

const library: Item[] = [
  { to: "/audiobooks", label: "common.kind.audiobooks", icon: <BookIcon />, cloud: "book" },
  { to: "/podcasts", label: "common.kind.podcasts", icon: <PodcastIcon />, cloud: "podcast" },
  { to: "/music", label: "common.kind.music", icon: <MusicIcon />, cloud: "music" },
];

/* Active colour follows the cloud the section belongs to; sections that
   belong to no cloud use the accent (brand brief §4.2). */
const cloudText: Record<Cloud, string> = {
  book: "text-book",
  podcast: "text-podcast",
  music: "text-music",
};

const activeByCloud: Record<Cloud | "none", string> = {
  book: "text-book bg-book/10",
  podcast: "text-podcast bg-podcast/10",
  music: "text-music bg-music/10",
  none: "text-accent-text bg-accent/10",
};

function NavItem({ item, collapsed, nowPlaying }: { item: Item; collapsed: boolean; nowPlaying?: "playing" | "paused" }) {
  /* `isActive` is resolved here rather than through NavLink's function-form
     className. Radix's `asChild` — which the tooltip wrapper uses — merges
     props by concatenating `className`, and a function stringifies into the
     literal source of itself. Every tooltip-wrapped link silently lost all of
     its classes: no width, no centring, no active colour. A plain string
     className survives the merge. */
  const active = useMatch({ path: item.to, end: item.end ?? false }) !== null;

  const link = (
    <NavLink
      to={item.to}
      end={item.end}
      // The label stays in the DOM so the link keeps its accessible name;
      // collapsing hides it visually, it doesn't remove it.
      className={cn(
        "flex items-center rounded-[10px] text-sm font-medium transition-colors",
        "[&>svg]:h-[18px] [&>svg]:w-[18px] [&>svg]:shrink-0 [&>svg]:stroke-[1.75]",
        collapsed ? "mx-auto h-10 w-10 justify-center" : "h-9 gap-3 px-3",
        active ? activeByCloud[item.cloud ?? "none"] : "text-fg/80 hover:bg-bg-alt hover:text-fg"
      )}
    >
      {item.icon}
      <span className={cn("whitespace-nowrap", collapsed && "sr-only")}>{t(item.label)}</span>
      {item.badge && !collapsed && !nowPlaying && (
        <span className={cn("ml-auto shrink-0 rounded-pill px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-wide", item.cloud ? `${cloudText[item.cloud]} bg-current/12` : "bg-accent/15 text-accent-text")}>
          {item.badge}
        </span>
      )}
      {nowPlaying && (
        <>
          <NowPlayingGlyph
            playing={nowPlaying === "playing"}
            className={cn(
              item.cloud ? cloudText[item.cloud] : "text-accent-text",
              // On the rail there is no room beside the label, so it tucks
              // into the corner of the icon square.
              collapsed ? "absolute bottom-1 right-1 h-2.5 w-2.5" : "ml-auto"
            )}
          />
          <span className="sr-only">{nowPlaying === "playing" ? t("shell.nav.playing") : t("shell.nav.paused")}</span>
        </>
      )}
    </NavLink>
  );

  return collapsed ? (
    <Tooltip label={t(item.label)} side="right">
      {link}
    </Tooltip>
  ) : (
    link
  );
}

function SectionLabel({ children, collapsed }: { children: string; collapsed: boolean }) {
  // Collapsed, a heading with no items beneath it reads as a stray word; a
  // divider says "new group" just as well.
  if (collapsed) return <div className="mx-auto my-2 h-px w-6 bg-border" />;
  return <p className="mb-1 mt-5 px-3 text-[11px] font-semibold uppercase tracking-wider text-muted">{children}</p>;
}

/** `forceExpanded` is for the mobile drawer, where a rail makes no sense. */
export default function Sidebar({ forceExpanded = false }: { forceExpanded?: boolean }) {
  const navigate = useNavigate();
  const { user, clearAuth } = useAuthStore();
  const openShortcuts = useShortcuts((state) => state.setOpen);
  const { canGenerate } = useMyPermissions();
  // What this server offers (GET /server): an open-source server has no
  // billing, narration or translation, and the items simply aren't there.
  const { features } = useServerFeatures();
  // "New" until the page has been opened once; per browser, which is enough for a nudge.
  const { pathname } = useLocation();
  const [translateSeen, setTranslateSeen] = useState(() => {
    try {
      return localStorage.getItem("own-audio-seen-translate") === "1";
    } catch {
      return false;
    }
  });
  useEffect(() => {
    if (pathname !== "/translate" || translateSeen) return;
    try {
      localStorage.setItem("own-audio-seen-translate", "1");
    } catch {
      /* private mode: the badge just stays */
    }
    setTranslateSeen(true);
  }, [pathname, translateSeen]);
  const { collapsed: stored, toggle } = useSidebar();
  const collapsed = forceExpanded ? false : stored;

  /* Which section the sound is coming from. The player is global, so without
     this there is nothing tying what you hear to where it lives. */
  const track = usePlayerStore((s) => s.track);
  const isPlaying = usePlayerStore((s) => s.playing);
  const playingRoute =
    track?.kind === "audiobook" ? "/audiobooks" : track?.kind === "podcast" ? "/podcasts" : track?.kind === "music" ? "/music" : null;
  const nowPlayingFor = (to: string): "playing" | "paused" | undefined =>
    to === playingRoute ? (isPlaying ? "playing" : "paused") : undefined;

  async function signOut() {
    try {
      await logout();
    } catch {
      // the server may already have dropped the session
    } finally {
      // Someone else may sign in on this device next.
      void clearDownloads();
      clearAuth();
      navigate("/auth/login", { replace: true });
    }
  }

  return (
    <aside
      className={cn(
        "flex h-full shrink-0 flex-col border-r border-border bg-bg-alt py-4 lg:bg-bg-alt/60",
        collapsed ? "w-16 px-2" : "w-64 px-3 lg:w-60"
      )}
    >
      <div className={cn("mb-2 flex items-center", collapsed ? "flex-col gap-1" : "gap-2.5 px-2")}>
        <NavLink
          to="/"
          className={cn("flex items-center py-1.5", collapsed ? "justify-center" : "gap-2.5")}
          aria-label={t("shell.nav.homeLink")}
        >
          <Logo size={collapsed ? 28 : 30} />
          {!collapsed && <Wordmark className="text-[17px]" />}
        </NavLink>
        {!forceExpanded && (
          <Tooltip label={collapsed ? t("shell.sidebar.expandShortcut") : t("shell.sidebar.collapseShortcut")} side="right">
            <IconButton
              size="sm"
              label={collapsed ? t("shell.sidebar.expand") : t("shell.sidebar.collapse")}
              onClick={toggle}
              className={cn("text-muted", !collapsed && "ml-auto")}
            >
              {collapsed ? <ChevronsRight className="h-4 w-4" /> : <ChevronsLeft className="h-4 w-4" />}
            </IconButton>
          </Tooltip>
        )}
      </div>

      <nav className="scroll-subtle flex-1 overflow-y-auto overflow-x-hidden">
        <NavItem item={{ to: "/", label: "shell.nav.home", icon: <Home />, end: true }} collapsed={collapsed} />
        {/* Your own overview beside Home, not an item in the library. */}
        <NavItem item={{ to: "/stats", label: "shell.nav.stats", icon: <BarChart3 /> }} collapsed={collapsed} />


        <SectionLabel collapsed={collapsed}>{t("shell.nav.library")}</SectionLabel>
        {library.map((i) => (
          <NavItem key={i.to} item={i} collapsed={collapsed} nowPlaying={nowPlayingFor(i.to)} />
        ))}

        {/* Translating needs no narration permission (the server does not ask for one), so the
            section is there for everyone; narrating a book only for those who may. Both only on
            a server that offers them at all. */}
        {(features.translation || (features.narration && canGenerate)) && (
          <SectionLabel collapsed={collapsed}>{t("shell.nav.create")}</SectionLabel>
        )}
        {features.narration && canGenerate && (
          <NavItem item={{ to: "/generate", label: "shell.nav.narrate", icon: <Sparkles />, cloud: "book" }} collapsed={collapsed} />
        )}
        {features.translation && (
          <NavItem
            item={{ to: "/translate", label: "shell.nav.translate", icon: <Languages />, cloud: "podcast", badge: translateSeen ? undefined : t("shell.nav.new") }}
            collapsed={collapsed}
          />
        )}

        <SectionLabel collapsed={collapsed}>{t("shell.nav.household")}</SectionLabel>
        <NavItem item={{ to: "/family", label: "shell.nav.family", icon: <Users /> }} collapsed={collapsed} />
        {/* Beside Family: it is about what the family does not see. "Just me", not "Private" —
            nothing here is locked, it is only unshared. */}
        <NavItem item={{ to: "/private", label: "shell.nav.justMe", icon: <UserRound /> }} collapsed={collapsed} />
        {features.billing && <NavItem item={{ to: "/billing", label: "shell.nav.billing", icon: <Wallet /> }} collapsed={collapsed} />}
        <NavItem item={{ to: "/trash", label: "shell.nav.trash", icon: <Trash2 /> }} collapsed={collapsed} />

      </nav>

      <div className={cn("mt-3 flex flex-col border-t border-border pt-3", collapsed ? "gap-1" : "gap-2")}>
        <NotificationsPanel collapsed={collapsed} />
        <UploadIndicator collapsed={collapsed} />
        <ThemeToggle compact={collapsed} />

        <Menu>
          <MenuTrigger asChild>
            <button
              aria-label={user?.display_name ?? t("shell.nav.account")}
              className={cn(
                "flex h-10 items-center rounded-[10px] text-left hover:bg-bg-alt",
                collapsed ? "mx-auto w-10 justify-center" : "w-full gap-2.5 px-2"
              )}
            >
              <span className="flex h-7 w-7 shrink-0 items-center justify-center rounded-pill bg-accent/15 text-accent">
                <User className="h-4 w-4" />
              </span>
              {!collapsed && (
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm font-medium text-fg">{user?.display_name ?? t("shell.nav.account")}</span>
                  <span className="block truncate text-[11px] text-muted">{user?.email}</span>
                </span>
              )}
            </button>
          </MenuTrigger>
          <MenuContent align="start" side="top" className="w-56">
            <MenuItem icon={<Settings />} onSelect={() => navigate("/settings")}>
              {t("shell.account.settings")}
            </MenuItem>
            <MenuItem icon={<PlayCircle />} onSelect={() => navigate("/settings/playback")}>
              {t("shell.account.playback")}
            </MenuItem>
            <MenuItem icon={<Keyboard />} onSelect={() => openShortcuts(true)}>
              {t("shell.account.shortcuts")}
            </MenuItem>
            <MenuSeparator />
            <MenuItem icon={<LogOut />} onSelect={signOut}>
              {t("common.action.signOut")}
            </MenuItem>
          </MenuContent>
        </Menu>
      </div>
    </aside>
  );
}
