// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useMemo, useRef, useState } from "react";
import { Link, Navigate, useLocation, useParams, useSearchParams } from "react-router-dom";
import axios from "axios";
import { Copy, Pause, Play, RefreshCw, RotateCcw, Share, Shuffle, Smartphone } from "lucide-react";
import PlayerBar from "../../components/player/PlayerBar";
import OfflineBanner from "../../components/shell/OfflineBanner";
import { MediaRow } from "../../components/library/MediaCard";
import { Button, Cover, EmptyState, Skeleton, toast } from "../../components/ui";
import { TrackTrailing } from "../music/Downloads";
import { formatDuration, timeAgo } from "../../lib/format";
import { playTracks, resumeTracks, shuffleTracks } from "../../lib/play";
import { loadResume, saveResume } from "../../lib/offline/playlistResume";
import { usePlayerStore } from "../../store/playerStore";
import {
  applyPlaylistChanges,
  checkPlaylist,
  HomePlaylistLimitError,
  MAX_HOME_PLAYLISTS,
  offlineCoverUrl,
  savePlaylistOffline,
  useDownloads,
  type PlaylistChanges,
} from "../../lib/offline/downloads";
import type { OfflinePlaylist } from "../../lib/offline/db";
import { claimIconHandoff, createIconHandoff, iconFromParam, iconLink, iconParam } from "../../lib/offline/iconHandoff";
import { getPlaylist } from "../../api/music";
import { loginPathFor } from "../../lib/returnTo";
import { useAuthStore } from "../../store/authStore";
import { intlLocale, useT } from "../../i18n";

/*
 * One playlist, and nothing else — what a playlist's own Home Screen icon
 * opens. Everything renders from the copy on this device, so it works with no
 * connection; when there is one, the copy is refreshed and new songs download.
 *
 * It cannot start playing by itself: browsers only allow sound after a tap,
 * so the page is built around one big Play button instead.
 */

/** Where this page load started. An icon made for a playlist launches straight
 *  into `/play/<id>`; the main app only ever gets here by navigating. */
const launchPath = window.location.pathname;

interface BeforeInstallPromptEvent extends Event {
  prompt: () => Promise<void>;
}

const isStandalone = () =>
  window.matchMedia("(display-mode: standalone)").matches ||
  (navigator as Navigator & { standalone?: boolean }).standalone === true;

const isAndroid = () => /Android/i.test(navigator.userAgent);

const isIOS = () =>
  /iPhone|iPad|iPod/.test(navigator.userAgent) || (navigator.userAgent.includes("Macintosh") && navigator.maxTouchPoints > 1);

/** The cover, square-cropped to the 180 px iOS uses for a Home Screen icon.
 *  A data URL, because iOS reads the icon at the moment it is added and cannot
 *  send the sign-in a cover URL needs. JPEG, because it rides in a link. */
async function homeScreenIcon(coverUrl: string | null, size = 180, type = "image/jpeg"): Promise<string | null> {
  if (!coverUrl) return null;
  const src = await offlineCoverUrl(coverUrl);
  if (!src) return null;
  try {
    const img = new Image();
    img.src = src;
    await img.decode();
    const canvas = document.createElement("canvas");
    canvas.width = canvas.height = size;
    const ctx = canvas.getContext("2d");
    if (!ctx) return null;
    const side = Math.min(img.naturalWidth, img.naturalHeight);
    ctx.drawImage(img, (img.naturalWidth - side) / 2, (img.naturalHeight - side) / 2, side, side, 0, 0, size, size);
    return canvas.toDataURL(type, 0.8);
  } catch {
    return null;
  } finally {
    URL.revokeObjectURL(src);
  }
}

/** Point the page's name, manifest and icon at this playlist while it is open,
 *  so "Add to Home Screen" creates an icon for it rather than for the app. */
function useHomeScreenIdentity(playlist: OfflinePlaylist | undefined) {
  const id = playlist?.id;
  const name = playlist?.name;
  const coverUrl = playlist?.coverUrl ?? null;

  useEffect(() => {
    if (!id || !name) return;
    const restore: (() => void)[] = [];

    const setHead = (selector: string, attr: "href" | "content", value: string, create: () => HTMLElement) => {
      let el = document.head.querySelector<HTMLElement>(selector);
      const existed = !!el;
      if (!el) {
        el = create();
        document.head.appendChild(el);
      }
      const before = el.getAttribute(attr);
      el.setAttribute(attr, value);
      const node = el;
      restore.push(() => (existed && before != null ? node.setAttribute(attr, before) : node.remove()));
    };

    const title = document.title;
    document.title = name;
    restore.push(() => (document.title = title));

    setHead('link[rel="manifest"]', "href", `/play-manifest?id=${encodeURIComponent(id)}&name=${encodeURIComponent(name)}`, () =>
      Object.assign(document.createElement("link"), { rel: "manifest" })
    );
    setHead('meta[name="apple-mobile-web-app-title"]', "content", name, () =>
      Object.assign(document.createElement("meta"), { name: "apple-mobile-web-app-title" })
    );

    let cancelled = false;
    void homeScreenIcon(coverUrl).then((icon) => {
      if (!icon || cancelled) return;
      setHead('link[rel="apple-touch-icon"]', "href", icon, () =>
        Object.assign(document.createElement("link"), { rel: "apple-touch-icon" })
      );
    });
    // Android takes the icon from the manifest instead. Chrome installs only
    // with a PNG, WebP or SVG of at least 144 px; a 192 px WebP is also small
    // enough to ride in the manifest's URL.
    void homeScreenIcon(coverUrl, 192, "image/webp").then((icon) => {
      const i = iconParam(icon);
      if (!i || cancelled) return;
      document.head
        .querySelector('link[rel="manifest"]')
        ?.setAttribute("href", `/play-manifest?id=${encodeURIComponent(id)}&name=${encodeURIComponent(name)}&i=${i}`);
    });

    return () => {
      cancelled = true;
      restore.reverse().forEach((r) => r());
    };
  }, [id, name, coverUrl]);
}

/** How this page is being seen, which decides both the hint and whether songs
 *  should download here. */
function viewMode(playlistId: string): "icon" | "app" | "ios-safari" | "browser" {
  if (isStandalone()) return launchPath === `/play/${playlistId}` ? "icon" : "app";
  return isIOS() ? "ios-safari" : "browser";
}

const bold = (c: React.ReactNode) => <span className="font-medium text-fg">{c}</span>;

function InstallHint({ playlistId, name, coverUrl }: { playlistId: string; name: string; coverUrl: string | null }) {
  const { t, rich } = useT();
  const [prompt, setPrompt] = useState<BeforeInstallPromptEvent | null>(null);
  const [busy, setBusy] = useState(false);
  const mode = viewMode(playlistId);

  useEffect(() => {
    const onPrompt = (e: Event) => {
      e.preventDefault();
      setPrompt(e as BeforeInstallPromptEvent);
    };
    window.addEventListener("beforeinstallprompt", onPrompt);
    return () => window.removeEventListener("beforeinstallprompt", onPrompt);
  }, []);

  if (mode === "icon") return null;

  const makeLink = async () => {
    const [code, icon] = await Promise.all([createIconHandoff(name), homeScreenIcon(coverUrl)]);
    return iconLink(playlistId, name, code, icon);
  };

  const openInSafari = async () => {
    setBusy(true);
    try {
      // Opens Safari itself, whatever the default browser — the only place
      // iOS offers "Add to Home Screen".
      window.location.href = `x-safari-${await makeLink()}`;
    } catch {
      toast.error(t("play.install.prepareFailed"), t("play.install.checkConnection"));
    } finally {
      setBusy(false);
    }
  };

  const copyLink = () => {
    const link = makeLink();
    // Safari only lets a page write the clipboard during the tap itself; a
    // ClipboardItem holding a promise is how the write can wait for the link.
    const write =
      typeof ClipboardItem !== "undefined" && navigator.clipboard?.write
        ? navigator.clipboard.write([new ClipboardItem({ "text/plain": link.then((u) => new Blob([u], { type: "text/plain" })) })])
        : link.then((u) => navigator.clipboard.writeText(u));
    write.then(
      () => toast.success(t("play.install.linkCopied"), t("play.install.pasteInSafari")),
      () =>
        void link.then(
          (u) => toast.show(t("play.install.copyThisLink"), u),
          () => toast.error(t("play.install.prepareFailed"))
        )
    );
  };

  return (
    <div className="rounded-card border border-border p-3 text-sm">
      <p className="flex items-center gap-2 font-medium">
        <Smartphone className="h-4 w-4" /> {t("play.install.title")}
      </p>
      {mode === "app" && isAndroid() ? (
        // An installed app has no browser menu to install from. Chrome shares
        // this app's storage and sign-in on Android, so the page needs nothing
        // but its address.
        <>
          <p className="mt-1 text-muted">{rich("play.install.androidApp", { b: bold })}</p>
          <Button
            className="mt-2"
            size="sm"
            icon={<Share className="h-4 w-4" />}
            onClick={() => {
              window.location.href = `intent://${window.location.host}/play/${playlistId}#Intent;scheme=https;package=com.android.chrome;end`;
            }}
          >
            {t("play.install.openInChrome")}
          </Button>
        </>
      ) : mode === "app" ? (
        // Inside an app already on the Home Screen there is no Share button,
        // and iOS only adds icons from Safari.
        <>
          <p className="mt-1 text-muted">{t("play.install.iosApp")}</p>
          <div className="mt-2 flex flex-wrap gap-2">
            <Button size="sm" loading={busy} icon={<Share className="h-4 w-4" />} onClick={() => void openInSafari()}>
              {t("play.install.openInSafari")}
            </Button>
            <Button size="sm" variant="secondary" icon={<Copy className="h-4 w-4" />} onClick={copyLink}>
              {t("play.install.copyLink")}
            </Button>
          </div>
          <p className="mt-2 text-xs text-muted">{t("play.install.linkWarning")}</p>
        </>
      ) : prompt ? (
        <Button className="mt-2" size="sm" onClick={() => void prompt.prompt().finally(() => setPrompt(null))}>
          {t("play.install.addToHomeScreen")}
        </Button>
      ) : (
        <p className="mt-1 text-muted">{rich("play.install.browserMenu", { b: bold })}</p>
      )}
    </div>
  );
}

/**
 * Safari on an iPhone: only the step that makes the icon. Nothing is signed in
 * or downloaded here — Safari's storage is not the icon's — and the pairing
 * code in the URL is left for the icon to use.
 */
function SafariAddStep({ playlistId, code, name, icon }: { playlistId: string; code: string | null; name: string | null; icon: string | null }) {
  const { t, rich } = useT();
  const token = useAuthStore((s) => s.token);
  const location = useLocation();
  const [failed, setFailed] = useState(false);

  // Opened in Safari without a code (typed, or from a bookmark) but signed in
  // here: make one, and reload so the page arrives with it in its manifest.
  useEffect(() => {
    if (code || !token) return;
    let cancelled = false;
    (async () => {
      const n = name || (await getPlaylist(playlistId)).name;
      const k = await createIconHandoff(n);
      if (!cancelled) window.location.replace(iconLink(playlistId, n, k));
    })().catch(() => {
      if (!cancelled) setFailed(true);
    });
    return () => {
      cancelled = true;
    };
  }, [code, token, name, playlistId]);

  if (!code && !token) return <Navigate to={loginPathFor(location)} replace />;

  return (
    <div className="flex min-h-dvh flex-col items-center justify-center bg-bg px-6 text-center text-fg">
      {icon ? (
        <img src={icon} alt="" className="h-20 w-20 rounded-[18px] shadow-card" />
      ) : (
        <Smartphone className="h-10 w-10 text-accent" />
      )}
      {!code ? (
        <p className="mt-4 text-sm text-muted">{t(failed ? "play.safari.prepareFailed" : "play.safari.preparing")}</p>
      ) : (
        <>
          <h1 className="mt-4 text-2xl font-semibold tracking-tight">{name || t("play.safari.yourPlaylist")}</h1>
          <ol className="mt-6 max-w-xs space-y-3 text-left text-sm">
            <li>
              {rich("play.safari.step1", {
                share: () => <Share className="inline h-4 w-4 align-[-3px]" />,
                b: (c) => <span className="font-medium">{c}</span>,
              })}
            </li>
            <li>{rich("play.safari.step2", { b: (c) => <span className="font-medium">{c}</span> })}</li>
            <li>{t("play.safari.step3")}</li>
          </ol>
        </>
      )}
    </div>
  );
}

/** The icon's first launch: trade the pairing code for its own session. */
function ClaimIcon({ code }: { code: string }) {
  const { t } = useT();
  const location = useLocation();
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    void claimIconHandoff(code).then((ok) => {
      if (!ok) setFailed(true);
    });
  }, [code]);

  // Too late or already used: signing in by hand still lands back here.
  if (failed) return <Navigate to={loginPathFor(location)} replace />;
  return (
    <div className="flex min-h-dvh items-center justify-center bg-bg text-sm text-muted">{t("play.signingIn")}</div>
  );
}

export default function PlayPage() {
  const { playlistId = "" } = useParams();
  const [params] = useSearchParams();
  const location = useLocation();
  const token = useAuthStore((s) => s.token);
  const code = params.get("k");

  if (viewMode(playlistId) === "ios-safari") {
    return <SafariAddStep playlistId={playlistId} code={code} name={params.get("n")} icon={iconFromParam(params.get("i"))} />;
  }
  if (!token) return code ? <ClaimIcon code={code} /> : <Navigate to={loginPathFor(location)} replace />;
  return <PlaylistPlayer playlistId={playlistId} />;
}

function formatBytes(n: number): string {
  const [value, unit, digits] =
    n >= 1024 ** 3 ? [n / 1024 ** 3, "gigabyte", 1] : [Math.max(1, Math.round(n / 1024 ** 2)), "megabyte", 0];
  return new Intl.NumberFormat(intlLocale(), { style: "unit", unit, maximumFractionDigits: digits, minimumFractionDigits: digits }).format(value);
}

/** Android's browsers say whether they're on mobile data; iOS doesn't. */
function onMobileData(): boolean {
  return (navigator as Navigator & { connection?: { type?: string } }).connection?.type === "cellular";
}

/**
 * The one way this player talks to the server. Checking costs a few KB — the
 * playlist and its song list — and shows what an update would do before any
 * of it happens, because iOS gives a web app no way to know whether it's on
 * Wi-Fi: the size is what lets someone decide to wait for it.
 */
function PlaylistUpdate({ playlistId, savedAt, onGone }: { playlistId: string; savedAt: number; onGone: () => void }) {
  const { t } = useT();
  const [step, setStep] = useState<"idle" | "checking" | "applying">("idle");
  const [found, setFound] = useState<PlaylistChanges | null>(null);

  const check = async () => {
    if (!navigator.onLine) {
      toast.error(t("play.update.noConnection"), t("play.update.noConnectionHint"));
      return;
    }
    setStep("checking");
    try {
      const changes = await checkPlaylist(playlistId);
      if (changes.toDownload.length === 0 && changes.toRemove.length === 0) {
        // Name, order or cover may still have changed; none of that costs data.
        await applyPlaylistChanges(changes);
        toast.success(t("play.update.upToDate"), t("play.update.nothingNew"));
      } else {
        setFound(changes);
      }
    } catch (e) {
      if (axios.isAxiosError(e) && (e.response?.status === 404 || e.response?.status === 403)) onGone();
      else toast.error(t("play.update.checkFailed"), t("play.update.noAnswer"));
    } finally {
      setStep("idle");
    }
  };

  const apply = async () => {
    if (!found) return;
    setStep("applying");
    try {
      await applyPlaylistChanges(found);
      const n = found.toDownload.length;
      toast.success(t("play.update.updating"), n > 0 ? t("play.update.downloading", { count: n }) : t("play.update.done"));
      setFound(null);
    } catch {
      toast.error(t("play.update.failed"), t("play.tryAgain"));
    } finally {
      setStep("idle");
    }
  };

  if (found) {
    const add = found.toDownload.length;
    const remove = found.toRemove.length;
    return (
      <div className="rounded-card border border-border p-3 text-sm">
        <p className="font-medium">{t("play.update.changes")}</p>
        <ul className="mt-1 space-y-0.5 text-muted">
          {add > 0 && (
            <li>
              {found.bytes > 0
                ? t("play.update.toDownloadSize", { count: add, size: formatBytes(found.bytes) })
                : t("play.update.toDownload", { count: add })}
            </li>
          )}
          {remove > 0 && <li>{t("play.update.toRemove", { count: remove })}</li>}
        </ul>
        {add > 0 && (
          <p className="mt-2 text-xs text-muted">
            {t(onMobileData() ? "play.update.mobileData" : "play.update.whileOpen")}
          </p>
        )}
        <div className="mt-3 flex gap-2">
          <Button size="sm" loading={step === "applying"} onClick={() => void apply()}>
            {t("play.update.apply")}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setFound(null)}>
            {t("play.update.notNow")}
          </Button>
        </div>
      </div>
    );
  }

  return (
    <div className="flex items-center justify-between gap-3 text-xs text-muted">
      <span>{t("play.update.updatedAgo", { when: timeAgo(new Date(savedAt).toISOString()) })}</span>
      <Button size="sm" variant="ghost" loading={step === "checking"} icon={<RefreshCw className="h-3.5 w-3.5" />} onClick={() => void check()}>
        {t("play.update.check")}
      </Button>
    </div>
  );
}

function PlaylistPlayer({ playlistId }: { playlistId: string }) {
  const { t } = useT();
  const ready = useDownloads((s) => s.ready);
  const snapshot = useDownloads((s) => s.playlists[playlistId]);
  const downloaded = useDownloads((s) => s.downloaded);
  const waiting = useDownloads((s) => s.waitingForNetwork);
  const current = usePlayerStore((s) => s.track);
  const playing = usePlayerStore((s) => s.playing);
  const queue = usePlayerStore((s) => s.queue);
  const position = usePlayerStore((s) => s.position);
  const setPlaying = usePlayerStore((s) => s.setPlaying);
  const [problem, setProblem] = useState<"limit" | "gone" | null>(null);
  // Which playlist the last sync finished for; until it matches, a first
  // open with no copy on the device shows loading rather than "not here".
  const [syncedFor, setSyncedFor] = useState<string | null>(null);
  const syncing = ready && !snapshot && navigator.onLine && syncedFor !== playlistId;

  useHomeScreenIdentity(snapshot);

  // Offline by default: the server is only asked when someone taps "Check for
  // updates". The one exception is a playlist with nothing on the device yet —
  // a freshly added icon, which on an iPhone starts with its own empty storage.
  const hasCopy = !!snapshot;
  useEffect(() => {
    if (!ready || hasCopy || !navigator.onLine) return;
    savePlaylistOffline(playlistId)
      .then(() => setProblem(null))
      .catch((e) => {
        if (e instanceof HomePlaylistLimitError) setProblem("limit");
        else if (axios.isAxiosError(e) && (e.response?.status === 404 || e.response?.status === 403)) setProblem("gone");
        // Anything else is a connection that isn't really there: the copy we have is the answer.
      })
      .finally(() => setSyncedFor(playlistId));
  }, [ready, hasCopy, playlistId]);

  const tracks = useMemo(() => snapshot?.tracks ?? [], [snapshot]);
  const have = tracks.filter((t) => downloaded[t.id]).length;
  const secs = tracks.reduce((sum, t) => sum + (t.duration_secs ?? 0), 0);

  // Remember where this playlist was left, on this device. The player saves its
  // position every 20 s, on pause and when the app goes to the background; the
  // store still holds the previous song's position for a moment after a song
  // changes, so a new song counts from zero until its own position arrives.
  const baseline = useRef<{ trackId: string; position: number } | null>(null);
  useEffect(() => {
    if (current?.kind !== "music" || tracks.length === 0) return;
    const inPlaylist = new Set(tracks.map((t) => t.id));
    const queueIds = queue.map((q) => (q.kind === "music" ? q.trackId : ""));
    if (queueIds.length === 0 || !queueIds.every((id) => inPlaylist.has(id))) return;
    if (baseline.current?.trackId !== current.trackId) baseline.current = { trackId: current.trackId, position };
    const at = position === baseline.current.position ? 0 : position;
    saveResume(playlistId, { queue: queueIds, trackId: current.trackId, position: at, updatedAt: Date.now() });
  }, [current, queue, position, tracks, playlistId]);

  const loadedHere = current?.kind === "music" && tracks.some((t) => t.id === current.trackId);
  const saved = loadResume(playlistId);
  const savedTrack = saved ? tracks.find((t) => t.id === saved.trackId) : undefined;
  const finished =
    !!saved && !!savedTrack && saved.trackId === saved.queue[saved.queue.length - 1] && (savedTrack.duration_secs ?? Infinity) - saved.position < 5;
  const canResume = !!saved && !!savedTrack && !finished && (saved.position > 0 || saved.queue.indexOf(saved.trackId) > 0);

  const resume = () => {
    if (!saved) return;
    const byId = new Map(tracks.map((t) => [t.id, t]));
    const available = new Set(playable().map((t) => t.id));
    const list = saved.queue.map((id) => byId.get(id)).filter((t): t is NonNullable<typeof t> => !!t && available.has(t.id));
    const index = list.findIndex((t) => t.id === saved.trackId);
    if (index < 0) {
      playFrom(undefined, true);
      return;
    }
    void resumeTracks(list, index, saved.position);
  };

  /** Only what is on the device plays — this player doesn't stream. A song
   *  still downloading joins once it's there. */
  const playable = () => tracks.filter((t) => downloaded[t.id]);

  /** `restart` for the buttons: always from zero, where tapping the current
   *  song's row pauses it instead. */
  const playFrom = (trackId?: string, restart = false) => {
    const list = playable();
    if (list.length === 0) {
      toast.error(t("play.nothingToPlay"), t("play.noneOnDevice"));
      return;
    }
    const index = trackId ? list.findIndex((t) => t.id === trackId) : 0;
    if (index < 0) {
      toast.error(t("play.notOnDevice"), t("play.playsWhenDownloaded"));
      return;
    }
    void (restart ? resumeTracks(list, index, 0) : playTracks(list, index));
  };

  let body;
  if (!ready || (!snapshot && syncing)) {
    body = (
      <div className="space-y-3">
        <Skeleton className="mx-auto aspect-square w-56" />
        <Skeleton className="h-7 w-48" />
        {[...Array(5)].map((_, i) => <Skeleton key={i} className="h-11" />)}
      </div>
    );
  } else if (!snapshot) {
    body =
      problem === "limit" ? (
        <EmptyState
          title={t("play.limit.title", { max: MAX_HOME_PLAYLISTS })}
          description={t("play.limit.description")}
          action={
            <Link className="text-sm font-medium text-accent hover:underline" to="/music/downloads">
              {t("play.limit.openDownloads")}
            </Link>
          }
        />
      ) : problem === "gone" ? (
        <EmptyState title={t("play.gone.title")} description={t("play.gone.description")} />
      ) : (
        <EmptyState title={t("play.notOnDevice")} description={t("play.notYet.description")} />
      );
  } else {
    body = (
      <>
        <div className="flex flex-col items-center text-center">
          <Cover kind="music" src={snapshot.coverUrl} alt={snapshot.name} aspect="square" className="w-56 shadow-card" />
          <h1 className="mt-5 text-2xl font-semibold leading-tight tracking-tight">{snapshot.name}</h1>
          <p className="mt-1 text-xs text-muted">
            {t("play.songCount", { count: tracks.length })}
            {secs > 0 ? ` · ${formatDuration(secs)}` : ""}
            {" · "}
            {have === tracks.length
              ? t("play.allOnDevice")
              : t(waiting ? "play.someOnDeviceWaiting" : "play.someOnDevice", { have, total: tracks.length })}
          </p>
          {problem === "gone" && (
            <p className="mt-2 text-xs text-muted">{t("play.gone.stillPlays")}</p>
          )}
          {loadedHere ? (
            <Button
              size="lg"
              className="mt-5 w-full"
              icon={playing ? <Pause className="h-5 w-5 fill-current" /> : <Play className="h-5 w-5 fill-current" />}
              onClick={() => setPlaying(!playing)}
            >
              {playing ? t("common.action.pause") : t("play.continue")}
            </Button>
          ) : canResume && savedTrack ? (
            <Button size="lg" className="mt-5 h-auto w-full flex-col gap-0 py-2.5" onClick={resume}>
              <span className="flex items-center gap-2">
                <Play className="h-5 w-5 fill-current" /> {t("play.continue")}
              </span>
              <span className="max-w-full truncate text-xs font-normal opacity-80">
                {savedTrack.title}
                {saved && saved.position > 0 ? ` · ${t("play.from", { time: formatDuration(Math.floor(saved.position)) })}` : ""}
              </span>
            </Button>
          ) : null}
          <div className={`flex w-full gap-2 ${loadedHere || canResume ? "mt-2" : "mt-5"}`}>
            <Button
              size="lg"
              variant={loadedHere || canResume ? "secondary" : "primary"}
              className="flex-1"
              icon={loadedHere || canResume ? <RotateCcw className="h-5 w-5" /> : <Play className="h-5 w-5 fill-current" />}
              onClick={() => playFrom(undefined, true)}
            >
              {loadedHere || canResume ? t("play.fromStart") : t("common.action.play")}
            </Button>
            <Button size="lg" variant="secondary" className="flex-1" icon={<Shuffle className="h-5 w-5" />} onClick={() => {
              const list = playable();
              if (list.length > 0) void shuffleTracks(list);
            }}>
              {t("play.shuffle")}
            </Button>
          </div>
        </div>

        <PlaylistUpdate playlistId={playlistId} savedAt={snapshot.savedAt} onGone={() => setProblem("gone")} />

        <InstallHint playlistId={playlistId} name={snapshot.name} coverUrl={snapshot.coverUrl} />

        <div>
          {tracks.map((t, i) => {
            const active = current?.kind === "music" && current.trackId === t.id;
            return (
              <MediaRow
                key={t.id}
                kind="music"
                index={i + 1}
                title={t.title}
                subtitle={t.artist}
                cover={t.cover_url}
                active={active}
                playing={active && playing}
                trailing={<TrackTrailing id={t.id} durationSecs={t.duration_secs} />}
                onClick={() => playFrom(t.id)}
                onPlay={() => playFrom(t.id)}
              />
            );
          })}
        </div>
      </>
    );
  }

  return (
    <div className="flex h-dvh flex-col bg-bg text-fg">
      <OfflineBanner />
      <main className="min-h-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-md space-y-5 px-4 pb-8 pt-[max(1.5rem,env(safe-area-inset-top))]">
          {body}
          <p className="text-center text-xs">
            <Link to="/music" className="text-muted hover:underline">
              {t("play.openApp")}
            </Link>
          </p>
        </div>
      </main>
      <PlayerBar canStop={false} />
    </div>
  );
}
