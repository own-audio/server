// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AudioLines } from "lucide-react";
import { getAudioAnalysis, setAudioAnalysis } from "../../api/family";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, toast } from "../../components/ui";
import { useT } from "../../i18n";

/**
 * The switch that starts a library being measured.
 *
 * Three things this card has to get right, all of them wording rather than
 * code:
 *
 * 1. **It is not a privacy consent.** Nothing leaves the server, nothing is
 *    shared, no third party is involved. It is a question about work, and
 *    dressing it as a data-processing agreement would be both inaccurate and
 *    more alarming than the truth.
 * 2. **It is family-wide, not personal.** Tempo and loudness are properties of
 *    the recording, so one member enabling it measures the shared library once
 *    for everyone. A per-person checkbox would imply per-person results.
 * 3. **Off does not mean erased.** Disabling stops new work and keeps what
 *    exists, because deleting would mean fetching the whole library again.
 */
export default function AudioAnalysisCard({ isAdmin }: { isAdmin: boolean }) {
  const { t } = useT();
  const qc = useQueryClient();

  const { data: status } = useQuery({
    queryKey: ["audio-analysis"],
    queryFn: getAudioAnalysis,
    // While a pass is running the numbers move; once it settles, stop asking.
    refetchInterval: (q) => (q.state.data?.running ? 5_000 : false),
  });

  const toggle = useMutation({
    mutationFn: (enabled: boolean) => setAudioAnalysis(enabled),
    onSuccess: (next) => {
      qc.setQueryData(["audio-analysis"], next);
      toast.success(t(next.enabled ? "family.analysis.started" : "family.analysis.stopped"));
    },
    onError: (err) => toast.error(t("family.error.change"), apiErrorMessage(err, t("family.tryAgain"))),
  });

  if (!status) return null;

  const pct = status.total > 0 ? Math.round((status.measured / status.total) * 100) : 0;
  const remaining = Math.max(0, status.total - status.measured);

  return (
    <section className="mt-8">
      <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold uppercase tracking-wide text-muted">
        <AudioLines className="h-4 w-4" /> {t("family.analysis.title")}
      </h2>

      <div className="rounded-card border border-border p-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <p className="text-sm font-medium">{t("family.analysis.heading")}</p>
            <p className="mt-1 text-sm text-muted">{t("family.analysis.body")}</p>
          </div>

          {isAdmin && (
            <Button
              variant={status.enabled ? "secondary" : "primary"}
              disabled={toggle.isPending}
              onClick={() => toggle.mutate(!status.enabled)}
            >
              {t(status.enabled ? "family.analysis.stop" : "family.analysis.start")}
            </Button>
          )}
        </div>

        {status.enabled && (
          <div className="mt-4">
            <div className="mb-1.5 flex items-baseline justify-between text-sm">
              <span className="text-muted">
                {t("family.analysis.progress", { measured: status.measured, total: status.total })}
              </span>
              <span className="tabular-nums text-muted">{pct}%</span>
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-border">
              <div
                className="h-full rounded-full bg-accent transition-[width] duration-500"
                style={{ width: `${pct}%` }}
              />
            </div>
            {status.running && remaining > 0 && (
              // Worth saying, because it changes what a half-finished pass
              // means: the tracks measured first are the ones actually played,
              // so playlists are usable long before this reaches 100 %.
              <p className="mt-2 text-xs text-muted">
                {t("family.analysis.mostPlayedFirst", { remaining })}
              </p>
            )}
          </div>
        )}

        {!isAdmin && (
          <p className="mt-3 text-xs text-muted">
            {t("family.analysis.adminOnly")}
          </p>
        )}

        {status.enabled && (
          <p className="mt-3 text-xs text-muted">
            {t("family.analysis.stopKeeps")}
          </p>
        )}
      </div>
    </section>
  );
}
