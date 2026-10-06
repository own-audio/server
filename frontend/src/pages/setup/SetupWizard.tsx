// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, useEffect, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Check, CircleAlert, Database, HardDrive, Loader2 } from "lucide-react";
import { getSetupStatus, completeSetup } from "../../api/setup";
import { useAuthStore } from "../../store/authStore";
import AuthLayout, { AuthHeading, FormError } from "../../components/auth/AuthLayout";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Input, PasswordInput } from "../../components/ui";
import { cn } from "../../lib/cn";
import type { UserInfo } from "../../api/types";
import { useT, type PlainKey } from "../../i18n";

type Step = "checks" | "admin" | "done";
const STEPS: { key: Step; label: PlainKey }[] = [
  { key: "checks", label: "setup.step.server" },
  { key: "admin", label: "setup.step.firstAccount" },
  { key: "done", label: "setup.step.ready" },
];

/* First-run setup for a fresh instance. Only reachable while no user exists;
   the old "welcome" step was a second landing page and is folded into the
   checks step. */
export default function SetupWizard() {
  const { t } = useT();
  const [step, setStep] = useState<Step>("checks");
  const navigate = useNavigate();
  const { setAuth, token } = useAuthStore();

  useEffect(() => {
    if (token && step !== "done") navigate("/", { replace: true });
  }, [token, step, navigate]);

  return (
    <AuthLayout aside={<p className="mt-6 text-sm text-muted">{t("setup.aside")}</p>}>
      <StepIndicator current={step} />
      {step === "checks" && <ChecksStep onNext={() => setStep("admin")} />}
      {step === "admin" && (
        <AdminStep
          onBack={() => setStep("checks")}
          onComplete={(t, u, r) => {
            setAuth(t, u, r);
            setStep("done");
          }}
        />
      )}
      {step === "done" && <DoneStep />}
    </AuthLayout>
  );
}

function StepIndicator({ current }: { current: Step }) {
  const { t } = useT();
  const idx = STEPS.findIndex((s) => s.key === current);
  return (
    <ol className="mb-8 flex items-center gap-2 text-xs">
      {STEPS.map((s, i) => (
        <li key={s.key} className="flex items-center gap-2">
          <span
            className={cn(
              "flex h-6 w-6 items-center justify-center rounded-pill text-[11px] font-semibold",
              i < idx ? "bg-success text-white" : i === idx ? "bg-accent text-on-accent" : "bg-bg-alt text-muted"
            )}
          >
            {i < idx ? <Check className="h-3.5 w-3.5" /> : i + 1}
          </span>
          <span className={cn("font-medium", i === idx ? "text-fg" : "text-muted")}>{t(s.label)}</span>
          {i < STEPS.length - 1 && <span className="mx-1 h-px w-6 bg-border" />}
        </li>
      ))}
    </ol>
  );
}

function CheckRow({ icon, label, detail, ok }: { icon: React.ReactNode; label: string; detail: string; ok: boolean }) {
  return (
    <div className="flex items-center gap-3 rounded-card border border-border bg-card px-4 py-3">
      <span className="text-muted [&>svg]:h-5 [&>svg]:w-5">{icon}</span>
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium">{label}</p>
        <p className="text-xs text-muted">{detail}</p>
      </div>
      {ok ? <Check className="h-5 w-5 text-success" /> : <CircleAlert className="h-5 w-5 text-error" />}
    </div>
  );
}

function ChecksStep({ onNext }: { onNext: () => void }) {
  const { data, isLoading, isError, refetch, isFetching } = useQuery({ queryKey: ["setup-status"], queryFn: getSetupStatus, retry: 1 });
  const allGood = !!data?.checks.database && !!data?.checks.storage;
  const { t } = useT();

  return (
    <div>
      <AuthHeading title={t("setup.checks.title")} subtitle={t("setup.checks.subtitle")} />

      {isLoading && (
        <p className="flex items-center gap-2 py-6 text-sm text-muted"><Loader2 className="h-4 w-4 animate-spin" /> {t("setup.checks.running")}</p>
      )}
      {isError && <FormError>{t("setup.checks.unreachable")}</FormError>}

      {data && (
        <div className="space-y-2">
          <CheckRow
            icon={<Database />}
            label={t("setup.checks.database")}
            detail={t(data.checks.database ? "setup.checks.databaseOk" : "setup.checks.databaseFailed")}
            ok={data.checks.database}
          />
          <CheckRow
            icon={<HardDrive />}
            label={t("setup.checks.storage", { backend: data.checks.storage_backend })}
            detail={t(
              data.checks.storage
                ? data.checks.storage_backend === "local"
                  ? "setup.checks.folderOk"
                  : "setup.checks.bucketOk"
                : "setup.checks.storageFailed"
            )}
            ok={data.checks.storage}
          />
        </div>
      )}

      {data && !allGood && (
        <p className="mt-4 text-sm text-muted">{t("setup.checks.fixHint")}</p>
      )}

      <div className="mt-8 flex gap-2">
        {(isError || (data && !allGood)) && (
          <Button variant="secondary" onClick={() => refetch()} loading={isFetching}>{t("common.action.retry")}</Button>
        )}
        <Button className="ml-auto" onClick={onNext} disabled={!allGood}>{t("setup.continue")}</Button>
      </div>
    </div>
  );
}

function AdminStep({ onBack, onComplete }: { onBack: () => void; onComplete: (token: string, user: UserInfo, refreshToken: string) => void }) {
  const { t } = useT();
  const [email, setEmail] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (!displayName.trim()) return setError(t("auth.error.nameMissing"));
    if (!email.includes("@")) return setError(t("setup.admin.badEmail"));
    if (password.length < 8) return setError(t("auth.error.passwordShort"));
    setLoading(true);
    try {
      const { token, user, refresh_token } = await completeSetup(email.trim(), password, displayName.trim());
      onComplete(token, user, refresh_token);
    } catch (err) {
      setError(apiErrorMessage(err, t("setup.admin.failed")));
    } finally {
      setLoading(false);
    }
  }

  return (
    <div>
      <AuthHeading title={t("setup.admin.title")} subtitle={t("setup.admin.subtitle")} />
      <form onSubmit={handleSubmit} className="space-y-4" noValidate>
        <Input label={t("auth.field.name")} autoCapitalize="words" autoComplete="name" autoFocus value={displayName} onChange={(e) => setDisplayName(e.target.value)} />
        <Input label={t("auth.field.email")} type="email" inputMode="email" autoCapitalize="none" autoCorrect="off" spellCheck={false} autoComplete="email" value={email} onChange={(e) => setEmail(e.target.value)} />
        <PasswordInput
          label={t("auth.field.password")}
          autoComplete="new-password"
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          hint={t("auth.field.passwordHint")}
        />
        <FormError>{error}</FormError>
        <div className="flex gap-2 pt-2">
          <Button type="button" variant="secondary" onClick={onBack}>{t("common.action.back")}</Button>
          <Button type="submit" className="ml-auto" loading={loading}>{t("common.action.createAccount")}</Button>
        </div>
      </form>
    </div>
  );
}

function DoneStep() {
  const { t } = useT();
  const navigate = useNavigate();
  return (
    <div>
      <AuthHeading title={t("setup.done.title")} subtitle={t("setup.done.subtitle")} />
      <ul className="space-y-2 text-sm text-muted">
        <li>{t("setup.done.tipAudiobook")}</li>
        <li>{t("setup.done.tipPodcast")}</li>
        <li>{t("setup.done.tipMusic")}</li>
      </ul>
      <Button size="lg" className="mt-8 w-full" onClick={() => navigate("/", { replace: true })}>{t("setup.done.open")}</Button>
    </div>
  );
}
