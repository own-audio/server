// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Languages } from "lucide-react";
import { listLanguages, listVoiceProfiles } from "../../api/generation";
import { createTranslation, quoteTranslation } from "../../api/podcastTranslate";
import { Button, Dialog, DialogContent, Select, Skeleton, toast } from "../../components/ui";
import { apiErrorMessage } from "../../lib/apiError";
import { formatCents } from "../../lib/format";
import { useT } from "../../i18n";

/**
 * Personal-use podcast translation (`docs/android-client-guide.md` §11a).
 *
 * Only ever reachable for an episode whose feed published a transcript — there
 * is no speech-to-text anywhere in this feature, so an episode without one
 * simply never offers it.
 *
 * Two contract rules this screen exists to honour:
 *
 * - **The server's `notice` is shown verbatim**, not paraphrased, on the quote
 *   and on every finished result. It is the personal-use statement for a
 *   derivative work of someone else's podcast.
 * - **No share, export or publish affordance**, however natural one would look
 *   next to a finished file.
 */
export default function TranslateEpisodeModal({
  episodeId,
  sourceLanguage,
  onClose,
}: {
  episodeId: string;
  /** The feed's own language, so the target doesn't default to the source —
   *  the server rejects that pairing, and opening onto an error is a poor
   *  greeting for a feature nobody has used before. */
  sourceLanguage?: string | null;
  onClose: () => void;
}) {
  const { t } = useT();
  const qc = useQueryClient();
  const [target, setTarget] = useState<string | null>(null);
  const [voice, setVoice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const { data: languages = [] } = useQuery({ queryKey: ["generation-languages"], queryFn: listLanguages, staleTime: Infinity });
  const { data: voices = [] } = useQuery({ queryKey: ["voice-profiles"], queryFn: listVoiceProfiles, staleTime: Infinity });

  // A feed's language may be a full tag ("en-GB"); the language list is bare codes.
  const source = sourceLanguage?.slice(0, 2).toLowerCase();
  const targetLanguage = target ?? languages.find((l) => l.code !== source)?.code ?? "en";
  const setTargetLanguage = setTarget;

  const voicesForLanguage = useMemo(() => voices.filter((v) => v.language === targetLanguage), [voices, targetLanguage]);
  // Derived rather than synced in an effect: whatever is picked stays picked
  // until the language moves out from under it.
  const selectedVoice = voicesForLanguage.some((v) => v.id === voice) ? voice : (voicesForLanguage[0]?.id ?? null);

  const {
    data: quote,
    isLoading: quoteLoading,
    error: quoteError,
  } = useQuery({
    queryKey: ["podcast-translate-quote", episodeId, targetLanguage, selectedVoice],
    queryFn: () => quoteTranslation(episodeId, targetLanguage, selectedVoice as string),
    enabled: !!selectedVoice,
    retry: false,
  });

  const create = useMutation({
    mutationFn: () => createTranslation(episodeId, targetLanguage, selectedVoice as string),
    onSuccess: () => {
      // The episode's own list, and the show-wide and recent lists on the Translate page.
      qc.invalidateQueries({ queryKey: ["podcast-translations"] });
      toast.success(t("podcasts.translate.started"), t("podcasts.translate.startedBody"));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("podcasts.translate.error.start"))),
  });

  const quoteMessage =
    quoteError && apiErrorMessage(quoteError, t("podcasts.translate.error.quote"));

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("podcasts.translate.title")}
        description={t("podcasts.translate.description")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button
              icon={<Languages className="h-4 w-4" />}
              onClick={() => create.mutate()}
              loading={create.isPending}
              disabled={!quote}
            >
              {t("podcasts.translate.action")}
            </Button>
          </>
        }
      >
        <div className="grid gap-3 sm:grid-cols-2">
          <Select label={t("podcasts.translate.language")} value={targetLanguage} onChange={(e) => setTargetLanguage(e.target.value)}>
            {languages.map((lang) => (
              <option key={lang.code} value={lang.code}>
                {lang.label}
              </option>
            ))}
          </Select>

          {voicesForLanguage.length === 0 ? (
            <div>
              <span className="mb-1.5 block text-[13px] font-medium text-fg">{t("podcasts.translate.voice")}</span>
              <p className="text-sm text-muted">{t("podcasts.translate.noVoice")}</p>
            </div>
          ) : (
            <Select label={t("podcasts.translate.voice")} value={selectedVoice ?? ""} onChange={(e) => setVoice(e.target.value)}>
              {voicesForLanguage.map((v) => (
                <option key={v.id} value={v.id}>
                  {v.display_name} · {v.gender}
                </option>
              ))}
            </Select>
          )}
        </div>

        <div className="mt-4 rounded-card border border-border p-4">
          {quoteLoading ? (
            <Skeleton className="h-10" />
          ) : quoteMessage ? (
            <p className="text-sm text-error">{quoteMessage}</p>
          ) : quote ? (
            <>
              <div className="flex items-baseline justify-between">
                <span className="text-sm text-muted">{t("podcasts.translate.price")}</span>
                {/* The server states the currency; never assume dollars. */}
                <span className="text-lg font-semibold">{formatCents(quote.quoted_price_cents, quote.currency)}</span>
              </div>
              <p className="mt-1 text-xs text-muted">
                {t("podcasts.translate.chargeLine", { count: quote.char_count })}
              </p>
            </>
          ) : (
            <p className="text-sm text-muted">{t("podcasts.translate.pickForPrice")}</p>
          )}
        </div>

        {/* Verbatim, as the contract requires — this is the personal-use
            statement, not marketing copy to reword. */}
        {quote?.notice && <p className="mt-3 text-xs leading-relaxed text-muted">{quote.notice}</p>}

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
