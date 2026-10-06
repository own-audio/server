// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useQuery } from "@tanstack/react-query";
import { Check } from "lucide-react";
import { listVoiceProfiles } from "../../../api/generation";
import { useT, type PlainKey } from "../../../i18n";

type OutputMode = "single_m4b" | "multi_file";

export default function VoiceStep({
  voiceProfileId,
  outputMode,
  language,
  onVoiceChange,
  onOutputModeChange,
}: {
  voiceProfileId: string | null;
  outputMode: OutputMode;
  /** The language the audiobook will be read in; its voices come first. */
  language: string;
  onVoiceChange: (id: string) => void;
  onOutputModeChange: (mode: OutputMode) => void;
}) {
  const { t } = useT();
  const { data: all = [], isLoading } = useQuery({
    queryKey: ["voice-profiles"],
    queryFn: listVoiceProfiles,
  });
  // A German voice reading an English book was one tap away; the book's own language comes first.
  const base = language.slice(0, 2).toLowerCase();
  const matching = all.filter((v) => v.language.slice(0, 2).toLowerCase() === base);
  const others = all.filter((v) => v.language.slice(0, 2).toLowerCase() !== base);
  // Decided once the voices are in: with none in the book's language there is nothing to hide.
  const [expanded, setExpanded] = useState(false);
  const pickedOther = others.some((v) => v.id === voiceProfileId);
  const showOthers = expanded || pickedOther || matching.length === 0;
  const voices = showOthers ? [...matching, ...others] : matching;

  return (
    <div>
      <h2 className="mb-1 text-lg font-semibold">{t("generate.voice.title")}</h2>
      <p className="mb-5 text-sm text-muted">{t("generate.voice.intro")}</p>

      {isLoading && <p className="text-sm text-muted">{t("generate.voice.loading")}</p>}

      <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">
        {voices.map((voice) => {
          const selected = voice.id === voiceProfileId;
          return (
            <button
              key={voice.id}
              type="button"
              onClick={() => onVoiceChange(voice.id)}
              className={`rounded-xl border p-4 text-left transition ${
 selected
 ? "border-accent bg-accent/6 "
 : "border-border bg-card hover:border-accent "
 }`}
            >
              <div className="flex items-center justify-between">
                <p className="font-semibold">{voice.display_name}</p>
                {selected && <Check className="h-4 w-4 text-accent-text" />}
              </div>
              <p className="mt-1 text-xs text-muted">
                {t("generate.voice.meta", { language: voice.language.toUpperCase(), gender: voice.gender })}
              </p>
              {voice.preview_url ? (
                <audio
                  controls
                  preload="none"
                  src={voice.preview_url}
                  onClick={(e) => e.stopPropagation()}
                  className="mt-3 h-8 w-full"
                />
              ) : (
                <p className="mt-3 text-xs text-muted">{t("generate.voice.noPreview")}</p>
              )}
            </button>
          );
        })}
      </div>

      {others.length > 0 && !showOthers && (
        <button type="button" onClick={() => setExpanded(true)} className="mt-3 py-2 text-sm font-medium text-accent-text">
          {t("generate.voice.otherLanguages", { count: others.length })}
        </button>
      )}

      <div className="mt-6">
        <p className="mb-2 text-sm font-medium text-fg">{t("generate.voice.output")}</p>
        <div className="flex gap-3">
          <OutputModeOption
            mode="single_m4b"
            label="generate.voice.singleM4b"
            description="generate.voice.singleM4bHint"
            selected={outputMode === "single_m4b"}
            onSelect={onOutputModeChange}
          />
          <OutputModeOption
            mode="multi_file"
            label="generate.voice.perChapter"
            description="generate.voice.perChapterHint"
            selected={outputMode === "multi_file"}
            onSelect={onOutputModeChange}
          />
        </div>
      </div>
    </div>
  );
}

function OutputModeOption({
  mode,
  label,
  description,
  selected,
  onSelect,
}: {
  mode: OutputMode;
  label: PlainKey;
  description: PlainKey;
  selected: boolean;
  onSelect: (mode: OutputMode) => void;
}) {
  const { t } = useT();
  return (
    <button
      type="button"
      onClick={() => onSelect(mode)}
      className={`flex-1 rounded-xl border p-3 text-left transition ${
 selected
 ? "border-accent bg-accent/6 "
 : "border-border bg-card hover:border-accent "
 }`}
    >
      <p className="text-sm font-semibold">{t(label)}</p>
      <p className="text-xs text-muted">{t(description)}</p>
    </button>
  );
}
