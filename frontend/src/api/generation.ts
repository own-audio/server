// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { GenerationJobStatus, GenerationQuote, Language, VoiceProfile } from "./types";

const ACCEPTED_EXTENSIONS = [".epub", ".mobi", ".txt"];

export async function listLanguages(): Promise<Language[]> {
  const { data } = await api.get<Language[]>("/audiobook-gen/languages");
  return data;
}

export type CoverMode = "auto" | "none" | "custom";

export interface CreateGenerationJobRequest {
  file: File;
  title: string;
  author?: string;
  sourceLanguage: string;
  targetLanguage: string;
  voiceProfileId: string;
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

export async function listVoiceProfiles(): Promise<VoiceProfile[]> {
  const { data } = await api.get<VoiceProfile[]>("/audiobook-gen/voices");
  return data;
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
  voiceProfileId: string
): Promise<GenerationQuote> {
  const { data } = await api.post<GenerationQuote>("/audiobook-gen/quote", {
    char_count: charCount,
    source_language: sourceLanguage,
    target_language: targetLanguage,
    voice_profile_id: voiceProfileId,
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
