// SPDX-License-Identifier: AGPL-3.0-or-later
import { intlLocale, t } from "../i18n";

/** The original file's audio, from `POST /api/v2/playback-info` (`source`). */
export interface SourceAudio {
  content_type: string;
  codec?: string;
  sample_rate_hz?: number;
  bit_depth?: number;
  channels?: number;
  bitrate_kbps?: number;
}

/** What the player was given for a song: the original or the server's conversion. */
export interface PlaybackInfo {
  url: string;
  decision: "direct" | "transcode" | string;
  content_type?: string | null;
  bitrate_kbps?: number | null;
  source?: SourceAudio;
}

const CODEC_NAMES: Record<string, string> = {
  mp1: "MP1", mp2: "MP2", mp3: "MP3", aac: "AAC", alac: "ALAC", flac: "FLAC", opus: "Opus",
  vorbis: "Vorbis", speex: "Speex", wav: "WAV", aiff: "AIFF", ape: "APE", wavpack: "WavPack",
  mpc: "Musepack", wma: "WMA", "wma pro": "WMA Pro", "wma lossless": "WMA Lossless",
};

/* Where a lossless file's bitrate says little and its bit depth and sample rate say it all. */
const LOSSLESS = new Set(["flac", "alac", "wav", "aiff", "ape", "wavpack", "wma lossless"]);

/** By type, for a file the server has not read yet. `audio/mpeg` is left
 *  unnamed: it can be MP2 as well as MP3. */
const TYPE_NAMES: Record<string, string> = {
  "audio/flac": "FLAC", "audio/x-flac": "FLAC", "audio/ogg": "Ogg", "audio/opus": "Opus",
  "audio/wav": "WAV", "audio/x-wav": "WAV", "audio/aiff": "AIFF", "audio/x-aiff": "AIFF",
  "audio/x-ms-wma": "WMA", "audio/aac": "AAC",
};

export function codecName(codec: string | undefined, contentType?: string | null): string | null {
  if (codec) return CODEC_NAMES[codec] ?? codec.toUpperCase();
  return contentType ? TYPE_NAMES[contentType] ?? null : null;
}

const khz = (hz: number) =>
  `${new Intl.NumberFormat(intlLocale(), { maximumFractionDigits: 1 }).format(hz / 1000)} kHz`;

/** "FLAC · 24-bit / 96 kHz", "MP2 · 128 kbps · 44.1 kHz · mono". Null when nothing is known. */
export function describeSource(s: SourceAudio | undefined): string | null {
  if (!s) return null;
  const parts: string[] = [];
  const name = codecName(s.codec, s.content_type);
  if (name) parts.push(name);
  if (s.codec && LOSSLESS.has(s.codec) && s.bit_depth && s.sample_rate_hz) {
    parts.push(`${s.bit_depth}-bit / ${khz(s.sample_rate_hz)}`);
  } else {
    if (s.bitrate_kbps) parts.push(`${s.bitrate_kbps} kbps`);
    if (s.sample_rate_hz) parts.push(khz(s.sample_rate_hz));
  }
  if (s.channels === 1) parts.push(t("player.quality.mono"));
  else if (s.channels && s.channels > 2) parts.push(t("player.quality.channels", { n: s.channels }));
  return parts.length > 0 ? parts.join(" · ") : null;
}

/** "playing as MP3 · 192 kbps" when the server converts; null for the original. */
export function describeDelivery(info: PlaybackInfo | undefined): string | null {
  if (!info || info.decision !== "transcode") return null;
  const format = [info.content_type === "audio/aac" ? "AAC" : "MP3", info.bitrate_kbps ? `${info.bitrate_kbps} kbps` : null]
    .filter(Boolean)
    .join(" · ");
  return t("player.quality.converted", { format });
}

/* The answers for the links the player holds, so it can say what it is playing
   without asking again. Links expire in hours; a page lives far fewer songs. */
const byUrl = new Map<string, PlaybackInfo>();

export function rememberPlaybackInfo(info: PlaybackInfo): void {
  byUrl.set(info.url, info);
}

/** The answer behind a link, also when the player took the proxy route to it (lib/mediaRoute). */
export function playbackInfoFor(url: string | null | undefined): PlaybackInfo | undefined {
  if (!url) return undefined;
  return byUrl.get(url) ?? byUrl.get(url.replace(/&route=proxy$/, ""));
}
