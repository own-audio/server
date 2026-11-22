// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import { isAxiosError } from "axios";
import type { CoverModeId, GenerationJobStatus, GenerationQuote, Language, VoiceProfile, VoiceTier, VoiceTierId } from "./types";

const ACCEPTED_EXTENSIONS = [".epub", ".mobi", ".txt"];

export async function listLanguages(): Promise<Language[]> {
  const { data } = await api.get<Language[]>("/audiobook-gen/languages");
  return data;
}

export type CoverMode = CoverModeId;

/** Only an EPUB or MOBI can carry a cover of its own. */
export function canHaveOwnCover(file: File | null): boolean {
  return !!file && /\.(epub|mobi)$/i.test(file.name);
}

export interface CreateGenerationJobRequest {
  file: File;
  title: string;
  author?: string;
  sourceLanguage: string;
  targetLanguage: string;
  voiceProfileId: string;
  /** Sent only when the server offers tiers; without it the server reads with Chirp 3: HD. */
  voiceTier?: VoiceTierId;
  outputMode: "single_m4b" | "multi_file";
  coverMode: CoverMode;
  /** Required when `coverMode` is "custom" — the server rejects the job without
   *  it rather than quietly falling back to the default brief. */
  coverPrompt?: string;
  coverShowTitle: boolean;
}

/** The brief the generator itself uses, so "custom" starts from the real
 *  default instead of a copy that can drift out of sync with the server. */
export async function getDefaultCoverPrompt(): Promise<string> {
  const { data } = await api.get<{ prompt: string }>("/audiobook-gen/cover-prompt");
  return data.prompt;
}

export function isSupportedSourceFile(file: File): boolean {
  const lower = file.name.toLowerCase();
  return ACCEPTED_EXTENSIONS.some((ext) => lower.endsWith(ext));
}

/** The voices of one tier; without `tier`, the server's default list (Chirp 3: HD). */
export async function listVoiceProfiles(tier?: VoiceTierId): Promise<VoiceProfile[]> {
  const { data } = tier
    ? await api.get<VoiceProfile[]>("/audiobook-gen/voices", { params: { tier } })
    : await api.get<VoiceProfile[]>("/audiobook-gen/voices");
  return data;
}

/** The voice tiers this server offers, or none on a server that has no tiers (Chirp only). */
export async function listVoiceTiers(): Promise<VoiceTier[]> {
  try {
    const { data } = await api.get<VoiceTier[]>("/audiobook-gen/voice-tiers");
    return Array.isArray(data) ? data : [];
  } catch (err) {
    const status = isAxiosError(err) ? err.response?.status : undefined;
    if (status === 404 || status === 405 || status === 501) return [];
    throw err;
  }
}

/** Upper-bound char-count estimate from the raw file, used for the up-front quote. */
export async function estimateCharCount(file: File): Promise<number> {
  const formData = new FormData();
  formData.append("file", file, file.name);
  const { data } = await api.post<{ estimated_char_count: number }>(
    "/audiobook-gen/estimate",
    formData
  );
  return data.estimated_char_count;
}

export async function getQuote(
  charCount: number,
  sourceLanguage: string,
  targetLanguage: string,
  voiceProfileId: string,
  coverMode: CoverMode = "auto",
  voiceTier?: VoiceTierId
): Promise<GenerationQuote> {
  const { data } = await api.post<GenerationQuote>("/audiobook-gen/quote", {
    char_count: charCount,
    source_language: sourceLanguage,
    target_language: targetLanguage,
    voice_profile_id: voiceProfileId,
    cover_mode: coverMode,
    ...(voiceTier ? { voice_tier: voiceTier } : {}),
  });
  return data;
}

export async function createGenerationJob(
  req: CreateGenerationJobRequest
): Promise<{ id: string }> {
  const formData = new FormData();
  formData.append("file", req.file, req.file.name);
  formData.append("title", req.title);
  if (req.author?.trim()) formData.append("author", req.author.trim());
  formData.append("source_language", req.sourceLanguage);
  formData.append("target_language", req.targetLanguage);
  formData.append("voice_profile_id", req.voiceProfileId);
  if (req.voiceTier) formData.append("voice_tier", req.voiceTier);
  formData.append("output_mode", req.outputMode);
  formData.append("cover_mode", req.coverMode);
  if (req.coverMode === "custom" && req.coverPrompt?.trim()) {
    formData.append("cover_prompt", req.coverPrompt.trim());
  }
  // The overlay is opt-in server-side; absent means false.
  if (req.coverShowTitle) formData.append("cover_show_title", "true");

  const { data } = await api.post<GenerationJobStatus>("/audiobook-gen/jobs", formData);
  return { id: data.id };
}

export async function getGenerationStatus(id: string): Promise<GenerationJobStatus> {
  const { data } = await api.get<GenerationJobStatus>(`/audiobook-gen/jobs/${id}`);
  return data;
}

export interface SetCoverRequest {
  /** `auto`/`custom` make a new cover (free, a bounded number per book), `book` takes the file's
   *  own, `none` takes the cover off. */
  coverMode: CoverMode;
  /** Required for `custom`. */
  coverPrompt?: string;
  coverShowTitle?: boolean;
}

/** Changes a finished narration's cover; answers the job with the new cover on its book. A
 *  generated cover takes 10–20 s. */
export async function setJobCover(id: string, req: SetCoverRequest): Promise<GenerationJobStatus> {
  const { data } = await api.post<GenerationJobStatus>(
    `/audiobook-gen/jobs/${id}/cover`,
    {
      cover_mode: req.coverMode,
      ...(req.coverMode === "custom" && req.coverPrompt?.trim() ? { cover_prompt: req.coverPrompt.trim() } : {}),
      cover_show_title: req.coverMode === "auto" || req.coverMode === "custom" ? !!req.coverShowTitle : false,
    }
  );
  return data;
}
