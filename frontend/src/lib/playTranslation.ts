// SPDX-License-Identifier: AGPL-3.0-or-later
import type { PodcastEpisodeTranslation } from "../api/types";
import { usePlayerStore } from "../store/playerStore";
import { getTranslationProgress, rememberTranslation } from "./translationProgress";

/**
 * Play a finished translation of an episode, or pause/resume it when it is already the one
 * playing — the same button stops it rather than sending someone to the player bar.
 *
 * Its position is kept in this browser (`translationProgress`): the server has no progress
 * endpoint for a translation that the player could report to, and `epId` is the original
 * episode, whose own position belongs to the original recording.
 */
export function playTranslation(
  tr: PodcastEpisodeTranslation,
  episode: { id: string; title: string; durationSecs: number | null; imageUrl?: string | null; feedId?: string },
  showTitle: string | null
) {
  if (!tr.stream_url) return;
  const store = usePlayerStore.getState();
  if (store.track?.kind === "podcast" && store.track.translationId === tr.id) {
    store.setPlaying(!store.playing);
    return;
  }
  const known = getTranslationProgress(tr.id);
  rememberTranslation({
    translationId: tr.id,
    episodeId: episode.id,
    episodeTitle: episode.title,
    showTitle,
    targetLanguage: tr.target_language,
    streamUrl: tr.stream_url,
    durationSecs: known?.durationSecs ?? episode.durationSecs,
  });
  store.play({
    kind: "podcast",
    epId: episode.id,
    translationId: tr.id,
    feedId: episode.feedId ?? "",
    title: episode.title,
    feedTitle: showTitle,
    imageUrl: episode.imageUrl ?? null,
    streamUrl: tr.stream_url,
    durationSecs: known?.durationSecs ?? episode.durationSecs,
    resumePosition: known && !known.completed ? known.positionSecs : 0,
  });
}
