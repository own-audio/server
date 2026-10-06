// SPDX-License-Identifier: AGPL-3.0-or-later
import type { Language } from "../../../api/types";
import { useT } from "../../../i18n";

export default function LanguageStep({
  languages,
  sourceLanguage,
  targetLanguage,
  translate,
  onSourceLanguageChange,
  onTargetLanguageChange,
  onTranslateChange,
}: {
  languages: Language[];
  sourceLanguage: string;
  targetLanguage: string;
  translate: boolean;
  onSourceLanguageChange: (code: string) => void;
  onTargetLanguageChange: (code: string) => void;
  onTranslateChange: (translate: boolean) => void;
}) {
  const { t } = useT();
  return (
    <div>
      <h2 className="mb-1 text-lg font-semibold">{t("generate.language.title")}</h2>
      <p className="mb-5 text-sm text-muted">{t("generate.language.intro")}</p>

      <div className="grid gap-4 sm:grid-cols-2">
        <div>
          <label className="mb-1 block text-sm font-medium text-fg">{t("generate.language.source")}</label>
          <select
            value={sourceLanguage}
            onChange={(e) => onSourceLanguageChange(e.target.value)}
            className="w-full rounded-lg border border-border px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent"
          >
            {languages.map((lang) => (
              <option key={lang.code} value={lang.code}>
                {lang.label}
              </option>
            ))}
          </select>
        </div>

        {translate && (
          <div>
            <label className="mb-1 block text-sm font-medium text-fg">{t("generate.language.target")}</label>
            <select
              value={targetLanguage}
              onChange={(e) => onTargetLanguageChange(e.target.value)}
              className="w-full rounded-lg border border-border px-3 py-2 text-sm focus:outline-none focus:ring-2 focus:ring-accent"
            >
              {languages.filter((lang) => lang.code !== sourceLanguage).map((lang) => (
                <option key={lang.code} value={lang.code}>
                  {lang.label}
                </option>
              ))}
            </select>
          </div>
        )}
      </div>

      <label className="mt-5 flex items-center gap-2 text-sm">
        <input
          type="checkbox"
          checked={translate}
          onChange={(e) => onTranslateChange(e.target.checked)}
          className="h-4 w-4 rounded accent-[var(--accent)]"
        />
        {t("generate.language.translate")}
      </label>
      {!translate && (
        <p className="mt-1 text-xs text-muted">{t("generate.language.sourceNote")}</p>
      )}
    </div>
  );
}
