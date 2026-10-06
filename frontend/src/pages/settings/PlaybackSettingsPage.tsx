// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { getUserSettings, updateAudiobookDefaults } from "../../api/playback";
import { usePlayerStore, SPEED_MAX, SPEED_MIN } from "../../store/playerStore";
import { Page } from "../../components/shell/SplitView";
import { Button, Skeleton, toast } from "../../components/ui";
import { apiErrorMessage } from "../../lib/apiError";
import { cn } from "../../lib/cn";
import { useT } from "../../i18n";

const SKIP_OPTIONS = [10, 15, 30, 45, 60, 90];
const SPEED_OPTIONS = [0.75, 1, 1.25, 1.5, 1.75, 2, 2.5, 3];

function Choice<T extends number>({
  label,
  hint,
  options,
  value,
  onChange,
  format,
}: {
  label: string;
  hint?: string;
  options: T[];
  value: T;
  onChange: (v: T) => void;
  format: (v: T) => string;
}) {
  return (
    <div className="py-4">
      <p className="text-sm font-medium">{label}</p>
      {hint && <p className="mt-0.5 text-xs text-muted">{hint}</p>}
      <div className="mt-2.5 flex flex-wrap gap-1.5">
        {options.map((o) => (
          <button
            key={o}
            onClick={() => onChange(o)}
            className={cn(
              "rounded-pill px-3 py-1.5 text-xs font-medium tabular-nums transition-colors",
              o === value ? "bg-accent text-on-accent" : "bg-bg-alt text-fg hover:bg-border"
            )}
          >
            {format(o)}
          </button>
        ))}
      </div>
    </div>
  );
}

/**
 * Audiobook playback defaults, stored server-side so every device agrees.
 *
 * The server clamps what it stores (skips 1–120 s, speed 0.5–3×), so the saved
 * values are re-read rather than assumed — the same rule the Mac follows.
 */
export default function PlaybackSettingsPage() {
  const { t } = useT();
  const qc = useQueryClient();
  const setSpeed = usePlayerStore((s) => s.setSpeed);
  const setSkipSecs = usePlayerStore((s) => s.setSkipSecs);

  const { data, isLoading } = useQuery({ queryKey: ["playback-settings"], queryFn: getUserSettings });

  /* The form is the fetched settings until the user touches something; only
     then does it become local state. Copying the response into state in an
     effect would render twice and fight the refetch after saving. */
  const [draft, setDraft] = useState<{ forward: number; backward: number; speed: number } | null>(null);
  const forward = draft?.forward ?? data?.ab_skip_forward_secs ?? 30;
  const backward = draft?.backward ?? data?.ab_skip_backward_secs ?? 15;
  const speed = draft?.speed ?? data?.ab_playback_speed ?? 1;
  const dirty = draft !== null;

  const save = useMutation({
    mutationFn: () => updateAudiobookDefaults(forward, backward, speed),
    onSuccess: async () => {
      // Re-read: the server clamps, so what we sent is not necessarily what it kept.
      const fresh = await qc.fetchQuery({ queryKey: ["playback-settings"], queryFn: getUserSettings });
      setDraft(null);
      setSkipSecs(fresh.ab_skip_forward_secs, fresh.ab_skip_backward_secs);
      setSpeed(fresh.ab_playback_speed);
      toast.success(t("settings.playback.saved"));
    },
    onError: (err) => toast.error(t("settings.playback.saveError"), apiErrorMessage(err, t("settings.error.tryAgain"))),
  });

  const seconds = (v: number) => t("settings.playback.seconds", { n: v });
  const edit = (patch: Partial<{ forward: number; backward: number; speed: number }>) =>
    setDraft({ forward, backward, speed, ...patch });

  if (isLoading) {
    return (
      <Page title={t("settings.playback.title")} back="/settings" width="max-w-2xl">
        <Skeleton className="h-40" />
      </Page>
    );
  }

  return (
    <Page title={t("settings.playback.title")} back="/settings" width="max-w-2xl">
      <div className="divide-y divide-border rounded-card border border-border px-5">
        <Choice
          label={t("settings.playback.skipForward")}
          hint={t("settings.playback.skipForwardHint")}
          options={SKIP_OPTIONS}
          value={forward}
          onChange={(v) => edit({ forward: v })}
          format={seconds}
        />
        <Choice
          label={t("settings.playback.skipBack")}
          options={SKIP_OPTIONS}
          value={backward}
          onChange={(v) => edit({ backward: v })}
          format={seconds}
        />
        <Choice
          label={t("settings.playback.speed")}
          hint={t("settings.playback.speedHint", { min: SPEED_MIN, max: SPEED_MAX })}
          options={SPEED_OPTIONS}
          value={speed}
          onChange={(v) => edit({ speed: v })}
          format={(v) => t("settings.playback.speedValue", { n: v })}
        />
      </div>

      <p className="mt-4 text-xs text-muted">{t("settings.playback.everyDevice")}</p>

      <div className="mt-5 flex items-center gap-2">
        <Button onClick={() => save.mutate()} loading={save.isPending} disabled={!dirty}>
          {t("common.action.save")}
        </Button>
        {dirty && <span className="text-xs text-muted">{t("settings.playback.unsaved")}</span>}
      </div>
    </Page>
  );
}
