// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback, useEffect, useRef, useState } from "react";
import { useNavigate } from "react-router-dom";
import MusicPlayerActions from "./MusicPlayerActions";
import { albumKey } from "../../pages/music/albumKey";
import SectionTheme from "../shell/SectionTheme";
import { cloudForKind } from "../../lib/sectionTheme";
import {
  ChevronDown,
  ChevronUp,
  ListMusic,
  Pause,
  Play,
  Repeat,
  Repeat1,
  RotateCcw,
  RotateCw,
  Shuffle,
  SkipBack,
  SkipForward,
  X,
  FileText,
} from "lucide-react";
import { usePlayerStore, type PlayerTrack } from "../../store/playerStore";
import { saveBookProgress, saveEpisodeProgress, type EndedReason } from "../../api/playback";
import { saveTrackProgress } from "../../api/music";
import { getBookFileStreamUrl } from "../../api/audiobooks";
import { getStreamUrl } from "../../api/podcasts";
import { getTrackStreamUrl } from "../../api/music";
import { offlineStreamUrl } from "../../lib/offline/downloads";
import { Cover } from "../ui/Cover";
import { toast } from "../../lib/toast";
import { IconButton } from "../ui/Button";
import { Dialog, DialogContent } from "../ui/Dialog";
import { stripHtml } from "../../lib/format";
import { formatClock } from "../../lib/time";
import { closeSpan, flush, flushOnUnload, openSpan, startReporter, type OpenSpan } from "../../lib/sessionReporter";
import {
  clearMediaSession,
  setMediaSessionHandlers,
  setMediaSessionMetadata,
  setMediaSessionPlaybackState,
  setMediaSessionPosition,
} from "../../lib/mediaSession";
import { useAuthStore } from "../../store/authStore";
import { apiBaseUrl } from "../../api/client";
import { pushNow } from "../../lib/queueSync";
import { saveTranslationPosition } from "../../lib/translationProgress";
import { BookmarksPopover, ChaptersPopover, SleepTimerPopover, SpeedPopover, VolumeControl } from "./popovers";
import QueuePanel from "./QueuePanel";
import SeekBar from "./SeekBar";
import { cn } from "../../lib/cn";
import { t as translate, useT } from "../../i18n";

/** Saves are frequent enough to survive a crash, rare enough not to be chatty. */
const PROGRESS_SAVE_MS = 20_000;
/** How close to a song's end the player moves on while it isn't on screen —
 *  see `handleTimeUpdate`. Background time updates can be a second apart. */
const EARLY_ADVANCE_SECS = 1.5;
/** How long after an interruption coming back to the app still resumes the
 *  music. Past this, silence is more likely what someone wants. */
const RESUME_AFTER_INTERRUPTION_MS = 15 * 60_000;

function trackKey(t: PlayerTrack): string {
  if (t.kind === "podcast") return t.epId;
  if (t.kind === "audiobook") return `${t.bookId}:${t.fileId}`;
  return t.trackId;
}

/** The browser only gives a numeric code; turn it into something actionable. */
function mediaErrorText(err: MediaError | null | undefined): string {
  switch (err?.code) {
    case MediaError.MEDIA_ERR_ABORTED:
      return translate("player.error.aborted");
    case MediaError.MEDIA_ERR_NETWORK:
      return translate("player.error.network");
    case MediaError.MEDIA_ERR_DECODE:
      return translate("player.error.decode");
    case MediaError.MEDIA_ERR_SRC_NOT_SUPPORTED:
      return translate("player.error.source");
    default:
      return translate("player.error.generic");
  }
}

function isCompleted(pos: number, dur: number) {
  return dur > 0 && pos / dur > 0.95;
}

/** Re-resolve a stream URL. They expire in ~4 h, so a tab left open overnight
 *  gets a 403 from storage on the next play rather than at page load. */
async function resolveStreamUrl(t: PlayerTrack): Promise<string> {
  if (t.kind === "audiobook") return getBookFileStreamUrl(t.bookId, t.fileId);
  if (t.kind === "podcast") return getStreamUrl(t.feedId, t.epId);
  return (await offlineStreamUrl(t.trackId)) ?? getTrackStreamUrl(t.trackId);
}

/** `canStop` is off where there is nothing to go back to — a playlist's own
 *  Home Screen player — so the bar can't be closed out from under it. */
export default function PlayerBar({ canStop = true }: { canStop?: boolean } = {}) {
  const {
    track,
    queue,
    queueIndex,
    playing,
    shuffle,
    repeat,
    volume,
    muted,
    speed,
    sleepTimer,
    showQueue,
    skipForwardSecs,
    skipBackwardSecs,
    setPlaying,
    stop,
    playNext,
    playPrev,
    toggleShuffle,
    cycleRepeat,
    setShowQueue,
    setSleepTimer,
  } = usePlayerStore();

  const { t } = useT();
  const audioRef = useRef<HTMLAudioElement>(null);
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [expanded, setExpanded] = useState(false);
  const [notesOpen, setNotesOpen] = useState(false);
  const source = usePlayerStore((s) => s.source);
  const navigate = useNavigate();
  const [stalled, setStalled] = useState<string | null>(null);
  const lastKeyRef = useRef<string | null>(null);
  const spanRef = useRef<OpenSpan | null>(null);
  const retriedRef = useRef(false);
  const advancedEarlyRef = useRef(false);
  /* A pause we asked for, as opposed to one the system imposed — an ad or a
     video in another app, a call. The media element fires the same `pause`
     event for both, and only this tells them apart. */
  const selfPausingRef = useRef(false);
  const interruptedAtRef = useRef<number | null>(null);
  /* Set by an interruption, cleared by the first resume after it — which must
     rebuild the audio rather than just play it (see `revive`). */
  const needsReviveRef = useRef(false);

  /* After an interruption iOS can resume a media element that is audibly
     dead: the time runs, nothing comes out. Loading the same source again at
     the same second makes it attach to the audio output afresh. */
  const revive = useCallback((audio: HTMLAudioElement) => {
    needsReviveRef.current = false;
    const url = audio.currentSrc || audio.src;
    if (!url) return;
    const at = audio.currentTime;
    retriedRef.current = false;
    selfPausingRef.current = !audio.paused;
    audio.src = url;
    audio.load();
    audio.addEventListener(
      "loadedmetadata",
      () => {
        selfPausingRef.current = false;
        audio.currentTime = at;
        void audio.play().catch(() => {});
      },
      { once: true }
    );
  }, []);

  const isBook = track?.kind === "audiobook";
  const positionInBook = isBook && track ? track.bookPositionOffsetSecs + currentTime : currentTime;

  // ── Progress persistence ────────────────────────────────────────────────

  const persist = useCallback(
    (position: number, completed: boolean) => {
      if (!track) return;
      usePlayerStore.getState().setPosition(position);
      if (track.kind === "podcast" && track.translationId) {
        // Never the server: see PodcastPlayerTrack.translationId.
        saveTranslationPosition(track.translationId, position, completed);
      } else if (track.kind === "podcast") {
        saveEpisodeProgress(track.epId, position, completed).catch(() => {});
      } else if (track.kind === "music") {
        saveTrackProgress(track.trackId, position, completed).catch(() => {});
      } else {
        saveBookProgress(track.bookId, track.fileId, track.bookPositionOffsetSecs + position, completed).catch(() => {});
      }
    },
    [track]
  );

  // ── Listening spans ─────────────────────────────────────────────────────
  // Explicit reporting switches the server's progress-derived sessions off for
  // this user, so a span dropped here is listening time that exists nowhere.

  // Why the current span ended, when the user's action already told us.
  // Consumed by the track-change cleanup below, which cannot otherwise tell a
  // skip from the user simply picking something else.
  const skipIntentRef = useRef<EndedReason | null>(null);

  const endSpan = useCallback(
    (position: number, reason: EndedReason) => {
      if (spanRef.current) {
        closeSpan(spanRef.current, position, speed, reason);
        spanRef.current = null;
      }
    },
    [speed]
  );

  // Next/previous, marked as a skip. Every transport surface routes through
  // these — the buttons, the keyboard, and the media-session handlers the OS
  // calls — because a skip that reports as `stopped` because it came from a
  // media key is a silently wrong signal nothing would ever flag.
  const skipToNext = useCallback(() => {
    skipIntentRef.current = "skipped";
    playNext();
  }, [playNext]);

  const skipToPrev = useCallback(() => {
    skipIntentRef.current = "skipped";
    playPrev();
  }, [playPrev]);

  const beginSpan = useCallback(
    (position: number) => {
      if (!track || spanRef.current) return;
      // A translated episode is not an episode: the statistics board could not place its id,
      // and attributing the time to the original would be counting listening that never
      // happened to it (guide §11a).
      if (track.kind === "podcast" && track.translationId) return;
      spanRef.current = openSpan(
        track.kind === "audiobook"
          ? { mediaKind: "audiobook", itemId: track.bookId, partId: track.fileId, startPosition: position }
          : track.kind === "podcast"
            ? { mediaKind: "podcast", itemId: track.epId, startPosition: position }
            : { mediaKind: "music", itemId: track.trackId, startPosition: position }
      );
    },
    [track]
  );

  useEffect(() => startReporter(), []);

  // ── Load and play the current track ─────────────────────────────────────

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio) return;
    if (!track) {
      // Only a playing element fires `pause`; marking one that is already
      // paused would leave the mark on the next, real interruption.
      if (!audio.paused) selfPausingRef.current = true;
      audio.pause();
      audio.removeAttribute("src");
      lastKeyRef.current = null;
      clearMediaSession();
      return;
    }

    const key = trackKey(track);
    if (key === lastKeyRef.current && repeat === "one") {
      audio.currentTime = 0;
      void audio.play().catch(() => {});
      return;
    }

    lastKeyRef.current = key;
    retriedRef.current = false;
    advancedEarlyRef.current = false;
    setStalled(null);
    audio.src = track.streamUrl;
    audio.load();
    // Start now rather than at `canplay`: in the background every moment of
    // silence is one iOS may use to suspend the app (see handleTimeUpdate).
    if (track.resumePosition <= 0) {
      audio.volume = muted ? 0 : volume;
      audio.playbackRate = speed;
      void audio.play().catch(() => {});
    }

    const onCanPlay = () => {
      if (track.resumePosition > 0) audio.currentTime = track.resumePosition;
      audio.volume = muted ? 0 : volume;
      audio.playbackRate = speed;
      void audio.play().catch(() => {});
      beginSpan(audio.currentTime);
    };
    audio.addEventListener("canplay", onCanPlay, { once: true });
    void setMediaSessionMetadata(track);
    return () => audio.removeEventListener("canplay", onCanPlay);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [track?.kind, track?.streamUrl, track && trackKey(track)]);

  // A stream URL that expired while the tab sat open fails at load; re-resolve
  // once and retry before telling the listener anything is wrong.
  useEffect(() => {
    const audio = audioRef.current;
    if (!audio || !track) return;
    async function onError() {
      // A failure here used to be invisible: the message existed only in the
      // expanded view, so from the bar the player just quietly did nothing.
      const fail = (reason: string) => {
        setStalled(reason);
        toast.error(translate("player.error.cantPlay"), reason);
      };
      if (retriedRef.current || !track || !audio) {
        fail(mediaErrorText(audio?.error));
        return;
      }
      retriedRef.current = true;
      try {
        const fresh = await resolveStreamUrl(track);
        const at = audio.currentTime;
        audio.src = fresh;
        audio.load();
        audio.addEventListener(
          "canplay",
          () => {
            audio.currentTime = at;
            void audio.play().catch(() => {});
          },
          { once: true }
        );
      } catch {
        fail(translate("player.error.noFreshLink"));
      }
    }
    audio.addEventListener("error", onError);
    return () => audio.removeEventListener("error", onError);
  }, [track]);

  useEffect(() => {
    const audio = audioRef.current;
    if (audio) audio.volume = muted ? 0 : volume;
  }, [volume, muted]);

  useEffect(() => {
    const audio = audioRef.current;
    if (audio) audio.playbackRate = speed;
  }, [speed]);

  useEffect(() => {
    const audio = audioRef.current;
    if (!audio || !track) return;
    if (playing) {
      void audio.play().catch(() => {});
      beginSpan(audio.currentTime);
    } else {
      // Interrupted: the system already paused it. Pausing again here made it
      // look like the listener's own pause, which is why the music never came
      // back once the interruption ended — iOS resumes only what the page
      // didn't pause itself.
      if (interruptedAtRef.current == null) {
        if (!audio.paused) selfPausingRef.current = true;
        audio.pause();
      }
      endSpan(audio.currentTime, "stopped");
      // Report before saving progress: until a reported session exists, the
      // server derives one from the progress save, and the two would count the
      // same playback twice.
      void flush().finally(() => {
        persist(audio.currentTime, isCompleted(audio.currentTime, duration));
        // Pausing is when someone picks the phone up instead — publish the
        // position so the queue hands off to where they actually are.
        void pushNow(true);
      });
    }
    if (playing || interruptedAtRef.current == null) setMediaSessionPlaybackState(playing);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [playing]);

  // If iOS doesn't give the music back after an interruption, coming back to
  // the app does — which is also when someone would otherwise tap Play.
  useEffect(() => {
    function resumeIfInterrupted() {
      const at = interruptedAtRef.current;
      const audio = audioRef.current;
      if (document.visibilityState !== "visible" || at == null || !audio || !track) return;
      interruptedAtRef.current = null;
      if (Date.now() - at < RESUME_AFTER_INTERRUPTION_MS) revive(audio);
      else needsReviveRef.current = false;
    }
    document.addEventListener("visibilitychange", resumeIfInterrupted);
    window.addEventListener("focus", resumeIfInterrupted);
    return () => {
      document.removeEventListener("visibilitychange", resumeIfInterrupted);
      window.removeEventListener("focus", resumeIfInterrupted);
    };
  }, [track, revive]);

  function handlePauseEvent() {
    const audio = audioRef.current;
    // Not ours, and not the end of the song: something else took the audio.
    if (!selfPausingRef.current && audio && !audio.ended && audio.src) {
      interruptedAtRef.current = Date.now();
      needsReviveRef.current = true;
    }
    selfPausingRef.current = false;
    setPlaying(false);
  }

  function handlePlayEvent() {
    interruptedAtRef.current = null;
    setPlaying(true);
    // iOS gave the music back by itself, or someone tapped Play: either way
    // the first play after an interruption rebuilds the audio.
    const audio = audioRef.current;
    if (needsReviveRef.current && audio) revive(audio);
  }

  // ── Periodic save ───────────────────────────────────────────────────────

  useEffect(() => {
    if (!track || !playing) return;
    const id = setInterval(() => {
      const audio = audioRef.current;
      if (audio && !audio.paused) persist(audio.currentTime, isCompleted(audio.currentTime, duration));
    }, PROGRESS_SAVE_MS);
    return () => clearInterval(id);
  }, [track, playing, duration, persist]);

  // ── Save and close the span when the track changes or the page goes away ─

  useEffect(() => {
    const audio = audioRef.current;
    return () => {
      if (audio && track) {
        // A span still open when the track changes means playback moved on
        // while it was running. `skipped` if the user asked for next/previous;
        // otherwise they navigated somewhere else entirely, which is
        // `replaced` — a different thing, and not a complaint about the track.
        endSpan(audio.currentTime, skipIntentRef.current ?? "replaced");
        skipIntentRef.current = null;
        persist(audio.currentTime, isCompleted(audio.currentTime, audio.duration));
      }
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [track?.kind, track?.streamUrl]);

  useEffect(() => {
    function onHide() {
      const audio = audioRef.current;
      if (audio && track) {
        endSpan(audio.currentTime, "stopped");
        persist(audio.currentTime, isCompleted(audio.currentTime, audio.duration));
      }
      // A normal request is cancelled with the page; keepalive is the only
      // thing that survives, so the last spans go out that way.
      flushOnUnload(useAuthStore.getState().token, apiBaseUrl);
    }
    window.addEventListener("pagehide", onHide);
    document.addEventListener("visibilitychange", () => {
      if (document.visibilityState === "hidden") void flush();
    });
    return () => window.removeEventListener("pagehide", onHide);
  }, [track, endSpan, persist]);

  // ── Transport ───────────────────────────────────────────────────────────

  const hasNext = repeat !== "none" || queueIndex + 1 < queue.length;
  const hasPrev = repeat === "all" || queueIndex > 0;

  const seekTo = useCallback((time: number) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.currentTime = Math.max(0, time);
    setCurrentTime(audio.currentTime);
  }, []);

  const seekBy = useCallback((delta: number) => {
    const audio = audioRef.current;
    if (!audio) return;
    audio.currentTime = Math.max(0, Math.min(audio.currentTime + delta, audio.duration || Infinity));
    setCurrentTime(audio.currentTime);
  }, []);

  /** Bookmarks store a whole-book position; the audio element knows only this file. */
  const seekInBook = useCallback(
    (bookPosition: number) => {
      if (!track || track.kind !== "audiobook") return;
      const local = bookPosition - track.bookPositionOffsetSecs;
      if (local >= 0 && local <= (duration || Infinity)) {
        seekTo(local);
        return;
      }
      const target = queue.findIndex(
        (item) =>
          item.kind === "audiobook" &&
          bookPosition >= item.bookPositionOffsetSecs &&
          bookPosition < item.bookPositionOffsetSecs + (item.durationSecs ?? 0)
      );
      if (target >= 0) {
        const item = queue[target];
        usePlayerStore.setState({
          queueIndex: target,
          track: { ...item, resumePosition: bookPosition - (item as { bookPositionOffsetSecs: number }).bookPositionOffsetSecs },
          playing: true,
        });
      }
    },
    [track, duration, queue, seekTo]
  );

  useEffect(() => {
    setMediaSessionHandlers({
      play: () => setPlaying(true),
      pause: () => setPlaying(false),
      previous: skipToPrev,
      next: skipToNext,
      seekTo,
      seekBy,
    }, { seekButtons: track?.kind !== "music" });
  }, [setPlaying, skipToPrev, skipToNext, seekTo, seekBy, track?.kind]);

  // ── Sleep timer ─────────────────────────────────────────────────────────

  useEffect(() => {
    if (sleepTimer.mode !== "at") return;
    const id = setInterval(() => {
      if (Date.now() >= sleepTimer.endsAt) {
        setPlaying(false);
        setSleepTimer({ mode: "off" });
      }
    }, 1000);
    return () => clearInterval(id);
  }, [sleepTimer, setPlaying, setSleepTimer]);

  // ── Keyboard ────────────────────────────────────────────────────────────

  useEffect(() => {
    function onKey(e: KeyboardEvent) {
      const el = e.target;
      if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement || el instanceof HTMLSelectElement) return;
      if ((el as HTMLElement | null)?.isContentEditable) return;
      if (!track || e.metaKey || e.ctrlKey) return;

      if (e.code === "Space") {
        e.preventDefault();
        setPlaying(!playing);
      } else if (e.code === "ArrowRight" && e.shiftKey) skipToNext();
      else if (e.code === "ArrowLeft" && e.shiftKey) skipToPrev();
      else if (e.code === "ArrowRight") seekBy(skipForwardSecs);
      else if (e.code === "ArrowLeft") seekBy(-skipBackwardSecs);
      else if (e.code === "ArrowUp") {
        e.preventDefault();
        usePlayerStore.getState().setVolume(Math.min(1, volume + 0.05));
      } else if (e.code === "ArrowDown") {
        e.preventDefault();
        usePlayerStore.getState().setVolume(Math.max(0, volume - 0.05));
      } else if (e.key === "m" || e.key === "M") usePlayerStore.getState().toggleMute();
      else if (e.key === "s" || e.key === "S") toggleShuffle();
      else if (e.key === "r" || e.key === "R") cycleRepeat();
    }
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [track, playing, volume, skipForwardSecs, skipBackwardSecs, setPlaying, skipToNext, skipToPrev, seekBy, toggleShuffle, cycleRepeat]);

  // ── Audio element events ────────────────────────────────────────────────

  function handleTimeUpdate() {
    const audio = audioRef.current;
    if (!audio) return;
    setCurrentTime(audio.currentTime);
    if (audio.duration && !isNaN(audio.duration)) {
      setDuration(audio.duration);
      setMediaSessionPosition(audio.currentTime, audio.duration, speed);
    }
    if (stalled) setStalled(null);

    // iOS suspends a Home Screen web app that is in the background — another
    // app in front, not the lock screen — as soon as its audio stops. That is
    // exactly the moment between a song ending and the next one starting, so
    // `ended` never got to start the next one until the app was reopened.
    // Moving on while the song is still playing keeps the app awake. It costs
    // the last second or so of the song, and only while the app isn't visible.
    if (
      document.visibilityState === "hidden" &&
      track?.kind === "music" &&
      hasNext &&
      repeat !== "one" &&
      sleepTimer.mode !== "endOfTrack" &&
      !advancedEarlyRef.current &&
      audio.duration > EARLY_ADVANCE_SECS * 4 &&
      audio.duration - audio.currentTime < EARLY_ADVANCE_SECS
    ) {
      advancedEarlyRef.current = true;
      handleEnded();
    }
  }

  function handleEnded() {
    if (!track) return;
    const audio = audioRef.current;
    // Closed before playNext() below, so the completion is recorded as one
    // rather than being mistaken for the skip that the track change looks like.
    endSpan(audio?.duration ?? duration, "completed");
    persist(duration || currentTime, !hasNext || repeat === "one");

    if (sleepTimer.mode === "endOfTrack") {
      setSleepTimer({ mode: "off" });
      setPlaying(false);
      return;
    }
    if (repeat === "one") {
      if (audio) {
        audio.currentTime = 0;
        void audio.play().catch(() => {});
      }
      return;
    }
    if (hasNext) {
      playNext();
      return;
    }
    setPlaying(false);
  }

  function handleClose() {
    const audio = audioRef.current;
    if (audio && track) {
      endSpan(audio.currentTime, "stopped");
      persist(audio.currentTime, isCompleted(audio.currentTime, duration));
    }
    void flush();
    stop();
    setExpanded(false);
    clearMediaSession();
  }

  if (!track) return null;

  const subtitle =
    track.kind === "podcast"
      ? track.feedTitle
      : track.kind === "music"
        ? track.artist
        : track.filePosition > 0
          ? t("player.bookPart", { book: track.bookTitle, n: track.filePosition })
          : track.bookTitle;

  const isMusic = track.kind === "music";

  const transport = (size: "md" | "lg") => (
    <div className={cn("flex items-center", size === "lg" ? "gap-3" : "gap-1")}>
      {isMusic ? (
        <IconButton size="sm" label={t("player.shuffle")} active={shuffle} onClick={toggleShuffle} className="hidden md:inline-flex">
          <Shuffle className="h-4 w-4" />
        </IconButton>
      ) : (
        <IconButton size="sm" label={t("player.skipBack", { seconds: skipBackwardSecs })} onClick={() => seekBy(-skipBackwardSecs)}>
          <RotateCcw className="h-4 w-4" />
        </IconButton>
      )}
      <IconButton label={t("common.action.previous")} onClick={skipToPrev} disabled={!hasPrev}>
        <SkipBack className="h-5 w-5 fill-current" />
      </IconButton>
      <button
        onClick={() => setPlaying(!playing)}
        aria-label={playing ? t("common.action.pause") : t("common.action.play")}
        className={cn(
          "flex items-center justify-center rounded-pill bg-fg text-bg transition-transform hover:scale-105 active:scale-95",
          size === "lg" ? "h-14 w-14" : "h-9 w-9"
        )}
      >
        {playing ? (
          <Pause className={cn("fill-current", size === "lg" ? "h-6 w-6" : "h-4 w-4")} />
        ) : (
          <Play className={cn("translate-x-px fill-current", size === "lg" ? "h-6 w-6" : "h-4 w-4")} />
        )}
      </button>
      <IconButton label={t("common.action.next")} onClick={skipToNext} disabled={!hasNext}>
        <SkipForward className="h-5 w-5 fill-current" />
      </IconButton>
      {isMusic ? (
        <IconButton size="sm" label={t("player.repeatMode", { mode: repeat })} active={repeat !== "none"} onClick={cycleRepeat} className="hidden md:inline-flex">
          {repeat === "one" ? <Repeat1 className="h-4 w-4" /> : <Repeat className="h-4 w-4" />}
        </IconButton>
      ) : (
        <IconButton size="sm" label={t("player.skipForward", { seconds: skipForwardSecs })} onClick={() => seekBy(skipForwardSecs)}>
          <RotateCw className="h-4 w-4" />
        </IconButton>
      )}
    </div>
  );

  const notes = track.kind === "podcast" && track.description ? stripHtml(track.description) : "";

  const kindTools = (
    <>
      {notes && (
        <IconButton size="sm" label={t("player.episodeNotes")} onClick={() => setNotesOpen(true)}>
          <FileText className="h-4 w-4" />
        </IconButton>
      )}
      {!isMusic && <SpeedPopover />}
      <SleepTimerPopover />
      {isBook && <ChaptersPopover />}
      {isBook && track.kind === "audiobook" && (
        <BookmarksPopover bookId={track.bookId} fileId={track.fileId} positionInBook={positionInBook} onSeek={seekInBook} />
      )}
    </>
  );

  // Music links its artist and album; leaving for them closes the player, like the apps do.
  const goTo = (path: string) => {
    setExpanded(false);
    navigate(path);
  };
  const artistLink = track.kind === "music" && track.artist ? track.artist : null;
  const albumLink = track.kind === "music" && track.album ? track.album : null;

  const expandedView = (
    <div className="fixed inset-0 z-50 isolate flex flex-col overflow-hidden bg-bg text-fg">
        {/* The cover, blurred across the whole screen, so the screen takes on the record's colours. */}
        <div aria-hidden className="pointer-events-none absolute inset-0 -z-10">
          <Cover kind={track.kind} src={track.imageUrl} alt="" aspect="square" className="h-full w-full scale-150 rounded-none opacity-60 blur-3xl saturate-150" />
          <div className="absolute inset-0 bg-gradient-to-b from-bg/30 via-bg/60 to-bg" />
        </div>

        <div className="flex items-center justify-between px-6 py-4">
          <IconButton label={t("player.minimize")} onClick={() => setExpanded(false)}>
            <ChevronDown className="h-5 w-5" />
          </IconButton>
          {source ? (
            // Where this queue came from, and the way back to it.
            <button
              type="button"
              onClick={() => goTo(source.path)}
              title={t("player.from.open", { name: source.label })}
              className="flex min-w-0 max-w-[60%] flex-col items-center rounded-md px-2 text-center hover:text-fg focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent"
            >
              <span className="text-[11px] font-semibold uppercase tracking-wider text-muted">{t(`player.from.${source.kind}`)}</span>
              <span className="w-full truncate text-sm font-medium">{source.label}</span>
            </button>
          ) : (
            <span className="text-[11px] font-semibold uppercase tracking-wider text-muted">{t("player.nowPlaying")}</span>
          )}
          {canStop ? (
            <IconButton label={t("common.action.stop")} onClick={handleClose}>
              <X className="h-5 w-5" />
            </IconButton>
          ) : (
            <span className="w-9" />
          )}
        </div>

        <div className="flex min-h-0 flex-1 items-center justify-center px-6">
          <Cover
            kind={track.kind}
            src={track.imageUrl}
            alt={track.title}
            aspect={track.kind === "audiobook" ? "portrait" : "square"}
            className={cn(
              "max-h-full shadow-pop",
              track.kind === "audiobook" ? "w-[min(60vw,16rem)] sm:w-72" : "w-[min(82vw,22rem)] sm:w-96"
            )}
          />
        </div>

        <div className="mx-auto w-full max-w-xl px-8 pb-2 pt-6">
          <h2 className="truncate text-2xl font-semibold tracking-tight">{track.title}</h2>
          {artistLink || albumLink ? (
            <p className="mt-1 flex min-w-0 items-center gap-1.5 text-base text-muted">
              {artistLink && (
                <button
                  type="button"
                  onClick={() => goTo(`/music/artists/${encodeURIComponent(artistLink)}`)}
                  title={t("player.goToArtist", { artist: artistLink })}
                  className="min-w-0 truncate font-medium text-fg/80 hover:text-fg hover:underline"
                >
                  {artistLink}
                </button>
              )}
              {artistLink && albumLink && <span aria-hidden>·</span>}
              {albumLink && track.kind === "music" && (
                <button
                  type="button"
                  onClick={() => goTo(`/music/albums/${encodeURIComponent(albumKey(track.albumArtist ?? track.artist ?? "", albumLink))}`)}
                  title={t("player.goToAlbum", { album: albumLink })}
                  className="min-w-0 truncate hover:text-fg hover:underline"
                >
                  {albumLink}
                </button>
              )}
            </p>
          ) : (
            subtitle && <p className="mt-1 truncate text-base text-muted">{subtitle}</p>
          )}
          {stalled && <p className="mt-2 text-sm text-error">{stalled}</p>}
        </div>

        <div className="mx-auto w-full max-w-xl px-8 pt-2">
          <SeekBar currentTime={currentTime} duration={duration} onSeek={seekTo} />
          <div className="mt-1.5 flex justify-between text-xs tabular-nums text-muted">
            <span>{formatClock(currentTime)}</span>
            <span>{formatClock(duration)}</span>
          </div>
        </div>

        <div className="flex items-center justify-center py-5">{transport("lg")}</div>

        {track.kind === "music" && (
          <div className="mx-auto flex w-full max-w-xl items-center justify-center gap-3 px-8 pb-2">
            <MusicPlayerActions track={track} onSkip={skipToNext} />
          </div>
        )}

        <div className="mx-auto flex w-full max-w-xl items-center justify-between gap-2 px-8 pb-[max(2rem,env(safe-area-inset-bottom))]">
          <VolumeControl />
          <div className="flex items-center gap-1">
            {kindTools}
            <IconButton label={t("player.queue.title")} active={showQueue} onClick={() => setShowQueue(!showQueue)}>
              <ListMusic className="h-5 w-5" />
            </IconButton>
          </div>
        </div>
      <QueuePanel />
    </div>
  );

  const compactView = (
    <footer className="relative border-t border-border bg-card/90 text-fg backdrop-blur">
      <QueuePanel />
      <SeekBar currentTime={currentTime} duration={duration} onSeek={seekTo} />

      {stalled && (
        <p role="alert" className="border-b border-error/30 bg-error/10 px-4 py-1.5 text-xs text-error">
          {stalled}
        </p>
      )}

      <div className="flex h-[68px] items-center gap-3 px-4">
        <button
          onClick={() => setExpanded(true)}
          className="flex min-w-0 flex-1 items-center gap-3 text-left sm:w-64 sm:flex-none"
          title={t("player.expandPlayer")}
        >
          <Cover kind={track.kind} src={track.imageUrl} alt={track.title} aspect="square" className="h-11 w-11 shrink-0 rounded-lg" />
          <span className="min-w-0">
            <span className="block truncate text-sm font-semibold leading-tight">{track.title}</span>
            {subtitle && <span className="block truncate text-xs text-muted">{subtitle}</span>}
          </span>
        </button>

        <div className="flex flex-1 items-center justify-center gap-3">
          <span className="hidden text-xs tabular-nums text-muted sm:inline">{formatClock(currentTime)}</span>
          {transport("md")}
          <span className="hidden text-xs tabular-nums text-muted sm:inline">{formatClock(duration)}</span>
        </div>

        <div className="hidden items-center gap-1 sm:flex sm:w-64 sm:justify-end">
          {kindTools}
          <VolumeControl />
          <IconButton size="sm" label={t("player.queue.title")} active={showQueue} onClick={() => setShowQueue(!showQueue)}>
            <ListMusic className="h-4 w-4" />
          </IconButton>
          <IconButton size="sm" label={t("common.action.expand")} onClick={() => setExpanded(true)}>
            <ChevronUp className="h-4 w-4" />
          </IconButton>
          {canStop && (
            <IconButton size="sm" label={t("common.action.stop")} onClick={handleClose}>
              <X className="h-4 w-4" />
            </IconButton>
          )}
        </div>
      </div>
    </footer>
  );

  return (
    <>
      {/* The audio element lives here, above the branch, and never moves.
          Rendering it inside each view instead means switching to the Now
          Playing screen unmounts it and mounts an empty one — `src` is set
          imperatively, so nothing restores it and playback stops dead. */}
      <audio
        ref={audioRef}
        onTimeUpdate={handleTimeUpdate}
        onLoadedMetadata={handleTimeUpdate}
        onPlay={handlePlayEvent}
        onPause={handlePauseEvent}
        onEnded={handleEnded}
      />
      {/* Gold for a book, sky for a podcast, red for music — whatever page it sits over. */}
      {track ? (
        <SectionTheme cloud={cloudForKind(track.kind)}>
          {expanded ? expandedView : compactView}
          <Dialog open={notesOpen && !!notes} onOpenChange={setNotesOpen}>
            {notesOpen && notes && (
              <DialogContent title={t("player.episodeNotes")} description={track.title} variant="sheet">
                <p className="whitespace-pre-line text-sm leading-relaxed text-muted">{notes}</p>
              </DialogContent>
            )}
          </Dialog>
        </SectionTheme>
      ) : (
        expanded ? expandedView : compactView
      )}
    </>
  );
}
