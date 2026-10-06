// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useMutation, useQuery } from "@tanstack/react-query";
import { createGenerationJob, estimateCharCount, getQuote, listLanguages, type CoverMode } from "../../api/generation";
import { Page } from "../../components/shell/SplitView";
import { Button } from "../../components/ui";
import { cn } from "../../lib/cn";
import SectionTheme from "../../components/shell/SectionTheme";
import { Check } from "lucide-react";
import UploadStep from "./steps/UploadStep";
import LanguageStep from "./steps/LanguageStep";
import VoiceStep from "./steps/VoiceStep";
import CoverStep from "./steps/CoverStep";
import QuoteStep from "./steps/QuoteStep";
import { useT, type PlainKey } from "../../i18n";

type OutputMode = "single_m4b" | "multi_file";

const STEP_LABELS: PlainKey[] = [
  "generate.step.upload",
  "generate.step.language",
  "generate.step.voice",
  "generate.step.cover",
  "generate.step.confirm",
];
const CONFIRM = STEP_LABELS.length - 1;

export default function GenerateWizardPage() {
  const navigate = useNavigate();
  const { t } = useT();
  const [step, setStep] = useState(0);

  const [file, setFile] = useState<File | null>(null);
  const [sourceLanguage, setSourceLanguage] = useState("en");
  // "Translate and narrate" in the Add audiobook dialog opens this with ?translate=1.
  const [params] = useSearchParams();
  const [translate, setTranslate] = useState(params.get("translate") === "1");
  const [targetLanguage, setTargetLanguage] = useState("cs");
  const [voiceProfileId, setVoiceProfileId] = useState<string | null>(null);
  const [outputMode, setOutputMode] = useState<OutputMode>("single_m4b");
  const [title, setTitle] = useState("");
  const [author, setAuthor] = useState("");
  const [coverMode, setCoverMode] = useState<CoverMode>("auto");
  const [coverPrompt, setCoverPrompt] = useState("");
  const [coverShowTitle, setCoverShowTitle] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);

  /* The server falls back to the filename when no title is sent; showing that
     same guess up front means the box is never blank and the fallback is never
     a surprise. Only until it is edited — retyping over someone's title when
     they swap the file would be worse. */
  const [titleTouched, setTitleTouched] = useState(false);
  const effectiveTitle = titleTouched ? title : (file?.name.replace(/\.[^.]+$/, "") ?? "");

  const effectiveTargetLanguage = translate ? targetLanguage : sourceLanguage;

  const { data: languages = [] } = useQuery({
    queryKey: ["generation-languages"],
    queryFn: listLanguages,
    staleTime: Infinity,
  });

  const { data: charCount } = useQuery({
    queryKey: ["char-count", file?.name, file?.size],
    queryFn: () => estimateCharCount(file as File),
    enabled: !!file && step >= CONFIRM,
  });

  const { data: quote, isLoading: quoteLoading } = useQuery({
    queryKey: ["quote", charCount, sourceLanguage, effectiveTargetLanguage, voiceProfileId],
    queryFn: () => getQuote(charCount as number, sourceLanguage, effectiveTargetLanguage, voiceProfileId as string),
    enabled: !!charCount && !!voiceProfileId && step >= CONFIRM,
  });

  const submitMutation = useMutation({
    mutationFn: async () => {
      if (!file || !voiceProfileId) throw new Error(t("generate.error.missing"));
      return createGenerationJob({
        file,
        title: effectiveTitle,
        author,
        sourceLanguage,
        targetLanguage: effectiveTargetLanguage,
        voiceProfileId,
        outputMode,
        coverMode,
        coverPrompt,
        coverShowTitle,
      });
    },
    onSuccess: ({ id }) => navigate(`/generate/${id}`),
    onError: (err: unknown) => {
      setSubmitError((err as Error)?.message || t("generate.error.start"));
    },
  });

  function canAdvance(): boolean {
    if (step === 0) return !!file;
    if (step === 1) return !translate || targetLanguage !== sourceLanguage;
    if (step === 2) return !!voiceProfileId;
    // The server rejects a custom cover with no brief; don't let them walk into it.
    if (step === 3) return effectiveTitle.trim().length > 0 && (coverMode !== "custom" || coverPrompt.trim().length > 0);
    return true;
  }

  function goNext() {
    if (!canAdvance()) return;
    setStep((s) => Math.min(s + 1, STEP_LABELS.length - 1));
  }

  function goBack() {
    setStep((s) => Math.max(s - 1, 0));
  }

  return (
    <SectionTheme cloud="book">
    <Page title={t("shell.nav.narrate")} width="max-w-2xl">
      <p className="-mt-4 mb-6 text-sm text-muted">{t("generate.intro")}</p>

      {/* A phone has room for where you are, not for five labelled steps. */}
      <div className="mb-6 sm:hidden">
        <div className="flex items-baseline justify-between text-sm">
          <span className="font-medium">{t(STEP_LABELS[step])}</span>
          <span className="text-muted">{t("generate.stepOf", { n: step + 1, total: STEP_LABELS.length })}</span>
        </div>
        <div className="mt-2 h-1.5 overflow-hidden rounded-pill bg-bg-alt">
          <div className="h-full rounded-pill bg-accent transition-[width] duration-300" style={{ width: `${((step + 1) / STEP_LABELS.length) * 100}%` }} />
        </div>
      </div>
      <ol className="mb-8 hidden items-center gap-2 sm:flex">
        {STEP_LABELS.map((label, i) => (
          <li key={label} className="flex flex-1 items-center gap-2">
            <span
              className={cn(
                "flex h-7 w-7 shrink-0 items-center justify-center rounded-pill text-xs font-semibold",
                i <= step ? "bg-accent text-on-accent" : "bg-bg-alt text-muted"
              )}
            >
              {i < step ? <Check className="h-3.5 w-3.5" /> : i + 1}
            </span>
            <span className={cn("text-sm", i === step ? "font-medium text-fg" : "text-muted")}>{t(label)}</span>
            {i < STEP_LABELS.length - 1 && <span className={cn("h-px flex-1", i < step ? "bg-accent" : "bg-border")} />}
          </li>
        ))}
      </ol>

      <div className="sm:rounded-card sm:border sm:border-border sm:p-6">
        {step === 0 && <UploadStep file={file} onFileSelected={setFile} />}
        {step === 1 && (
          <LanguageStep
            languages={languages}
            sourceLanguage={sourceLanguage}
            targetLanguage={targetLanguage}
            translate={translate}
            onSourceLanguageChange={setSourceLanguage}
            onTargetLanguageChange={setTargetLanguage}
            onTranslateChange={setTranslate}
          />
        )}
        {step === 2 && (
          <VoiceStep
            voiceProfileId={voiceProfileId}
            outputMode={outputMode}
            language={effectiveTargetLanguage}
            onVoiceChange={setVoiceProfileId}
            onOutputModeChange={setOutputMode}
          />
        )}
        {step === 3 && (
          <CoverStep
            title={effectiveTitle}
            author={author}
            coverMode={coverMode}
            coverPrompt={coverPrompt}
            coverShowTitle={coverShowTitle}
            onTitleChange={(v) => {
              setTitleTouched(true);
              setTitle(v);
            }}
            onAuthorChange={setAuthor}
            onCoverModeChange={setCoverMode}
            onCoverPromptChange={setCoverPrompt}
            onCoverShowTitleChange={setCoverShowTitle}
          />
        )}
        {step === 4 && <QuoteStep quote={quote} isLoading={quoteLoading} />}

        {submitError && (
          <p role="alert" className="mt-4 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{submitError}</p>
        )}

        {/* On a phone the buttons stay at the thumb, whatever the step's length. */}
        <div className="sticky bottom-0 -mx-4 mt-8 flex justify-between gap-3 border-t border-border bg-bg/90 px-4 py-3 backdrop-blur sm:static sm:mx-0 sm:border-0 sm:bg-transparent sm:p-0 sm:backdrop-blur-none">
          <Button variant="secondary" onClick={goBack} disabled={step === 0}>
            {t("common.action.back")}
          </Button>
          {step < STEP_LABELS.length - 1 ? (
            <Button onClick={goNext} disabled={!canAdvance()}>
              {t("common.action.next")}
            </Button>
          ) : (
            <Button onClick={() => submitMutation.mutate()} disabled={!quote} loading={submitMutation.isPending}>
              {t("generate.start")}
            </Button>
          )}
        </div>
      </div>

      {/* Narration spends the family's credit, and a failed job can't be
          retried in place — only submitted again. Say so before they commit. */}
      <p className="mt-4 text-xs text-muted">{t("generate.chargeNote")}</p>
    </Page>
    </SectionTheme>
  );
}
