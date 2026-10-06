// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect } from "react";
import { useQuery } from "@tanstack/react-query";
import { getDefaultCoverPrompt, type CoverMode } from "../../../api/generation";
import { Input, Textarea } from "../../../components/ui";
import { cn } from "../../../lib/cn";
import { useT, type PlainKey } from "../../../i18n";

const MODES: { mode: CoverMode; label: PlainKey; description: PlainKey }[] = [
  { mode: "auto", label: "generate.cover.auto", description: "generate.cover.autoHint" },
  { mode: "custom", label: "generate.cover.custom", description: "generate.cover.customHint" },
  { mode: "none", label: "generate.cover.none", description: "generate.cover.noneHint" },
];

/**
 * Title, author and cover art.
 *
 * The custom brief is seeded from `GET /audiobook-gen/cover-prompt` — the same
 * constant the generator uses — so editing starts from what would have happened
 * anyway rather than from an empty box.
 */
export default function CoverStep({
  title,
  author,
  coverMode,
  coverPrompt,
  coverShowTitle,
  onTitleChange,
  onAuthorChange,
  onCoverModeChange,
  onCoverPromptChange,
  onCoverShowTitleChange,
}: {
  title: string;
  author: string;
  coverMode: CoverMode;
  coverPrompt: string;
  coverShowTitle: boolean;
  onTitleChange: (v: string) => void;
  onAuthorChange: (v: string) => void;
  onCoverModeChange: (v: CoverMode) => void;
  onCoverPromptChange: (v: string) => void;
  onCoverShowTitleChange: (v: boolean) => void;
}) {
  const { t } = useT();
  const { data: defaultPrompt, isLoading } = useQuery({
    queryKey: ["cover-prompt"],
    queryFn: getDefaultCoverPrompt,
    staleTime: Infinity,
  });

  // Seed the box once, and only while it is untouched — retyping over someone's
  // edited brief because a query re-settled would be worse than not seeding.
  useEffect(() => {
    if (defaultPrompt && !coverPrompt) onCoverPromptChange(defaultPrompt);
  }, [defaultPrompt, coverPrompt, onCoverPromptChange]);

  return (
    <div>
      <h2 className="mb-1 text-lg font-semibold">{t("generate.cover.title")}</h2>
      <p className="mb-5 text-sm text-muted">{t("generate.cover.intro")}</p>

      <div className="grid gap-3 sm:grid-cols-2">
        <Input label={t("audiobooks.field.title")} value={title} onChange={(e) => onTitleChange(e.target.value)} />
        <Input label={t("audiobooks.field.author")} value={author} onChange={(e) => onAuthorChange(e.target.value)} placeholder={t("audiobooks.optional")} />
      </div>

      <p className="mb-2 mt-6 text-sm font-medium text-fg">{t("audiobooks.field.cover")}</p>
      <div className="flex flex-col gap-2 sm:flex-row">
        {MODES.map((m) => (
          <button
            key={m.mode}
            type="button"
            aria-pressed={coverMode === m.mode}
            onClick={() => onCoverModeChange(m.mode)}
            className={cn(
              "flex-1 rounded-xl border p-3 text-left transition",
              coverMode === m.mode ? "border-accent bg-accent/6" : "border-border bg-card hover:border-accent"
            )}
          >
            <span className="block text-sm font-semibold">{t(m.label)}</span>
            <span className="block text-xs text-muted">{t(m.description)}</span>
          </button>
        ))}
      </div>

      {coverMode === "custom" && (
        <div className="mt-4">
          <Textarea
            label={t("generate.cover.prompt")}
            rows={5}
            value={coverPrompt}
            onChange={(e) => onCoverPromptChange(e.target.value)}
            placeholder={isLoading ? t("generate.cover.loadingBrief") : undefined}
          />
          {defaultPrompt && coverPrompt !== defaultPrompt && (
            <button
              type="button"
              onClick={() => onCoverPromptChange(defaultPrompt)}
              className="mt-1.5 text-xs text-accent-text hover:underline"
            >
              {t("generate.cover.resetBrief")}
            </button>
          )}
        </div>
      )}

      {coverMode !== "none" && (
        <label className="mt-4 flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={coverShowTitle}
            onChange={(e) => onCoverShowTitleChange(e.target.checked)}
            className="h-4 w-4 rounded accent-[var(--accent)]"
          />
          {t("generate.cover.printTitle")}
        </label>
      )}
    </div>
  );
}
