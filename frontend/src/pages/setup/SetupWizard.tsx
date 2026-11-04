// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, useEffect, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { BookOpen, Check, CircleAlert, Database, FolderOpen, HardDrive, Loader2, Music } from "lucide-react";
import { getSetupStatus, completeSetup, getLibraryFolders, type SetupFolder } from "../../api/setup";
import { getServerInfo } from "../../api/server";
import { useAuthStore } from "../../store/authStore";
import AuthLayout, { AuthHeading, FormError } from "../../components/auth/AuthLayout";
import QrCode from "../../components/QrCode";
import { apiErrorMessage } from "../../lib/apiError";
import { Button, Input, PasswordInput } from "../../components/ui";
import { cn } from "../../lib/cn";
import type { UserInfo } from "../../api/types";
import { useT, type PlainKey } from "../../i18n";

type Step = "checks" | "admin" | "library" | "apps";
const STEPS: { key: Step; label: PlainKey }[] = [
  { key: "checks", label: "setup.step.server" },
  { key: "admin", label: "setup.step.firstAccount" },
  { key: "library", label: "setup.step.library" },
  { key: "apps", label: "setup.step.apps" },
];

/* First-run setup for a fresh instance. Only reachable while no user exists;
   the old "welcome" step was a second landing page and is folded into the
   checks step. After the first account the wizard goes on, signed in, with
   what the installer connected and how to reach the server from a phone. */
export default function SetupWizard() {
  const { t } = useT();
  const [step, setStep] = useState<Step>("checks");
  // Set once the account was made here: a visitor who was already signed in
  // has nothing to do on this page.
  const [madeHere, setMadeHere] = useState(false);
  const navigate = useNavigate();
  const { setAuth, token } = useAuthStore();

  useEffect(() => {
    if (token && !madeHere) navigate("/", { replace: true });
  }, [token, madeHere, navigate]);

  return (
    <AuthLayout aside={<p className="mt-6 text-sm text-muted">{t("setup.aside")}</p>}>
      <StepIndicator current={step} />
      {step === "checks" && <ChecksStep onNext={() => setStep("admin")} />}
      {step === "admin" && (
        <AdminStep
          onBack={() => setStep("checks")}
          onComplete={(t, u, r) => {
            setMadeHere(true);
            setAuth(t, u, r);
            setStep("library");
          }}
        />
      )}
      {step === "library" && <LibraryStep onNext={() => setStep("apps")} />}
      {step === "apps" && <AppsStep />}
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

function CheckRow({ icon, label, detail, ok }: { icon: React.ReactNode; label: string; detail: string; ok: boolean | null }) {
  return (
    <div className="flex items-center gap-3 rounded-card border border-border bg-card px-4 py-3">
      <span className="text-muted [&>svg]:h-5 [&>svg]:w-5">{icon}</span>
      <div className="min-w-0 flex-1">
        <p className="truncate text-sm font-medium">{label}</p>
        <p className="text-xs text-muted">{detail}</p>
      </div>
      {ok === null ? (
        <Loader2 className="h-5 w-5 animate-spin text-muted" />
      ) : ok ? (
        <Check className="h-5 w-5 text-success" />
      ) : (
        <CircleAlert className="h-5 w-5 text-error" />
      )}
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
    if (password.length < 12) return setError(t("auth.error.passwordShort"));
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

/* The folders the installer (or the compose override) connected, with their
   first scan as it runs. The scan starts once the admin exists, so this is
   where it is first seen. */
function LibraryStep({ onNext }: { onNext: () => void }) {
  const { t } = useT();
  const { data, isLoading } = useQuery({
    queryKey: ["setup-folders"],
    queryFn: getLibraryFolders,
    refetchInterval: (query) => {
      const folders = query.state.data?.folders ?? [];
      return folders.some((f) => !f.scan_finished_at && !f.scan_error) ? 2000 : false;
    },
  });
  const folders = data?.folders ?? [];

  function detail(f: SetupFolder): { text: string; ok: boolean | null } {
    if (f.scan_error) return { text: t("setup.library.error", { error: f.scan_error }), ok: false };
    if (f.scan_finished_at) return { text: t("setup.library.done", { seen: f.files_seen, added: f.files_added }), ok: true };
    if (f.scan_started_at) return { text: t("setup.library.scanning", { count: f.files_seen }), ok: null };
    return { text: t("setup.library.waiting"), ok: null };
  }

  return (
    <div>
      <AuthHeading title={t("setup.library.title")} subtitle={t("setup.library.subtitle")} />
      {isLoading && (
        <p className="flex items-center gap-2 py-6 text-sm text-muted"><Loader2 className="h-4 w-4 animate-spin" /></p>
      )}
      {data && folders.length === 0 && (
        <div className="flex items-start gap-3 rounded-card border border-border bg-card px-4 py-3 text-sm text-muted">
          <FolderOpen className="mt-0.5 h-5 w-5 shrink-0" />
          <p>{t("setup.library.none")}</p>
        </div>
      )}
      {folders.length > 0 && (
        <div className="space-y-2">
          {folders.map((f) => {
            const d = detail(f);
            return (
              <CheckRow
                key={f.id}
                icon={f.kind === "audiobooks" ? <BookOpen /> : <Music />}
                label={`${t(f.kind === "audiobooks" ? "setup.library.kind.audiobooks" : "setup.library.kind.music")} · ${f.path}`}
                detail={d.text}
                ok={d.ok}
              />
            );
          })}
          {folders.some((f) => !f.scan_finished_at && !f.scan_error) && (
            <p className="pt-2 text-sm text-muted">{t("setup.library.hint")}</p>
          )}
        </div>
      )}
      <div className="mt-8 flex">
        <Button className="ml-auto" onClick={onNext}>{t("setup.continue")}</Button>
      </div>
    </div>
  );
}

/* The server's address for the phone apps, as a QR code; the other addresses
   it answers on below it. */
function AppsStep() {
  const { t } = useT();
  const navigate = useNavigate();
  const { data } = useQuery({ queryKey: ["server-info"], queryFn: getServerInfo });
  const addresses = data?.addresses ?? [];
  const main = addresses.find((a) => a.scope === "public")?.url ?? window.location.origin;
  const others = addresses.map((a) => a.url).filter((u) => u !== main);

  return (
    <div>
      <AuthHeading title={t("setup.apps.title")} subtitle={t("setup.apps.subtitle")} />
      <div className="flex flex-col items-center gap-4 rounded-card border border-border bg-card px-4 py-6 sm:flex-row sm:items-start">
        <QrCode value={main} size={152} className="shrink-0" />
        <div className="min-w-0 flex-1 text-center sm:text-left">
          <p className="text-xs font-medium uppercase tracking-wide text-muted">{t("setup.apps.address")}</p>
          <p className="mt-1 break-all font-mono text-base font-semibold">{main}</p>
          {others.length > 0 && (
            <>
              <p className="mt-4 text-xs font-medium uppercase tracking-wide text-muted">{t("setup.apps.alsoAt")}</p>
              <ul className="mt-1 space-y-0.5 break-all font-mono text-sm text-muted">
                {others.map((u) => <li key={u}>{u}</li>)}
              </ul>
            </>
          )}
          <a href="https://www.own.audio/#apps" target="_blank" rel="noreferrer" className="mt-4 inline-block text-sm text-accent hover:underline">
            {t("setup.apps.where")}
          </a>
        </div>
      </div>
      <p className="mt-4 text-sm text-muted">{t("setup.apps.invite")}</p>
      <Button size="lg" className="mt-8 w-full" onClick={() => navigate("/", { replace: true })}>{t("setup.done.open")}</Button>
    </div>
  );
}
