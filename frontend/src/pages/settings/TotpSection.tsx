// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { disableTotp, enableTotp, getTotp, setupTotp, type TotpSetup } from "../../api/auth";
import QrCode from "../../components/QrCode";
import { Button, Input, Skeleton, toast } from "../../components/ui";
import { apiErrorMessage } from "../../lib/apiError";
import { useT } from "../../i18n";

/* Two-factor sign-in with an authenticator app. Three states: off (offer to
   set up), setting up (QR + first code), on (recovery codes left, turn off
   with a code). The recovery codes are shown exactly once, right after
   enabling — the server keeps only hashes. */
export default function TotpSection() {
  const { t } = useT();
  const qc = useQueryClient();
  const { data, isLoading } = useQuery({ queryKey: ["totp"], queryFn: getTotp, retry: false });
  const [setup, setSetup] = useState<TotpSetup | null>(null);
  const [code, setCode] = useState("");
  const [recoveryCodes, setRecoveryCodes] = useState<string[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const start = useMutation({
    mutationFn: setupTotp,
    onSuccess: (s) => {
      setSetup(s);
      setCode("");
      setError(null);
    },
    onError: (err) => toast.error(apiErrorMessage(err, t("settings.totp.error"))),
  });
  const enable = useMutation({
    mutationFn: (c: string) => enableTotp(c),
    onSuccess: (r) => {
      setRecoveryCodes(r.recovery_codes);
      setSetup(null);
      setCode("");
      setError(null);
      qc.invalidateQueries({ queryKey: ["totp"] });
    },
    onError: (err) => setError(apiErrorMessage(err, t("settings.totp.wrongCode"))),
  });
  const disable = useMutation({
    mutationFn: (c: string) => disableTotp(c),
    onSuccess: () => {
      setCode("");
      setError(null);
      setRecoveryCodes(null);
      qc.invalidateQueries({ queryKey: ["totp"] });
      toast.success(t("settings.totp.disabled"));
    },
    onError: (err) => setError(apiErrorMessage(err, t("settings.totp.wrongCode"))),
  });

  if (isLoading) return <Skeleton className="h-24" />;

  function submitEnable(e: FormEvent) {
    e.preventDefault();
    enable.mutate(code);
  }
  function submitDisable(e: FormEvent) {
    e.preventDefault();
    disable.mutate(code);
  }

  return (
    <div className="rounded-card border border-border p-4">
      {recoveryCodes && (
        <div className="mb-4 rounded-md border border-accent/40 bg-accent/10 p-3 text-sm">
          <p className="font-semibold">{t("settings.totp.recoveryTitle")}</p>
          <p className="mt-1 text-muted">{t("settings.totp.recoveryBody")}</p>
          <ul className="mt-2 grid grid-cols-2 gap-x-6 gap-y-1 font-mono text-sm">
            {recoveryCodes.map((c) => (
              <li key={c} className="select-all">{c}</li>
            ))}
          </ul>
          <Button size="sm" variant="ghost" className="mt-2" onClick={() => setRecoveryCodes(null)}>
            {t("settings.totp.recoverySaved")}
          </Button>
        </div>
      )}

      {data?.enabled ? (
        <form onSubmit={submitDisable} className="space-y-3">
          <p className="text-sm">{t("settings.totp.onIntro", { left: data.recovery_codes_left })}</p>
          <Input
            label={t("settings.totp.codeToDisable")}
            autoComplete="one-time-code"
            inputMode="numeric"
            value={code}
            onChange={(e) => setCode(e.target.value)}
            error={error ?? undefined}
          />
          <Button type="submit" size="sm" variant="secondary" loading={disable.isPending}>
            {t("settings.totp.turnOff")}
          </Button>
        </form>
      ) : setup ? (
        <form onSubmit={submitEnable} className="space-y-3">
          <p className="text-sm">{t("settings.totp.scanIntro")}</p>
          <div className="flex flex-wrap items-start gap-4">
            <QrCode value={setup.otpauth_uri} size={160} className="rounded-md bg-white p-2" />
            <div className="min-w-0 text-xs text-muted">
              <p>{t("settings.totp.manualEntry")}</p>
              <code className="mt-1 block select-all break-all rounded-md bg-bg-alt px-2 py-1.5">{setup.secret}</code>
            </div>
          </div>
          <Input
            label={t("settings.totp.firstCode")}
            autoComplete="one-time-code"
            inputMode="numeric"
            autoFocus
            value={code}
            onChange={(e) => setCode(e.target.value)}
            error={error ?? undefined}
          />
          <div className="flex gap-2">
            <Button type="submit" size="sm" loading={enable.isPending}>
              {t("settings.totp.turnOn")}
            </Button>
            <Button type="button" size="sm" variant="ghost" onClick={() => setSetup(null)}>
              {t("common.action.cancel")}
            </Button>
          </div>
        </form>
      ) : (
        <div className="space-y-3">
          <p className="text-sm">{t("settings.totp.offIntro")}</p>
          <Button size="sm" variant="secondary" loading={start.isPending} onClick={() => start.mutate()}>
            {t("settings.totp.setUp")}
          </Button>
        </div>
      )}
    </div>
  );
}
