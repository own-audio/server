// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, useEffect, type FormEvent } from "react";
import { useNavigate, useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { login, register, forgotPassword, checkRegistrationStatus, getAuthProviders } from "../../api/auth";
import { getServerInfo } from "../../api/server";
import GoogleSignInButton from "../../components/GoogleSignInButton";
import AppleSignInButton from "../../components/AppleSignInButton";
import AuthLayout, { AuthHeading, FormError } from "../../components/auth/AuthLayout";
import { apiErrorMessage, isRateLimited } from "../../lib/apiError";
import { Button, Input, PasswordInput } from "../../components/ui";
import { useAuthStore } from "../../store/authStore";
import { safeReturnTo } from "../../lib/returnTo";
import { useT } from "../../i18n";

type Mode = "login" | "register" | "forgot";

export default function AuthPage() {
  const [mode, setMode] = useState<Mode>("login");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [displayName, setDisplayName] = useState("");
  const [inviteCode, setInviteCode] = useState("");
  const [showInvite, setShowInvite] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [resetSent, setResetSent] = useState(false);
  const navigate = useNavigate();
  const [params] = useSearchParams();
  const { setAuth, token } = useAuthStore();
  const next = safeReturnTo(params.get("next"));
  const { t, rich } = useT();

  const { data: regStatus } = useQuery({ queryKey: ["registration-status"], queryFn: checkRegistrationStatus, staleTime: 60_000 });
  const registrationOpen = regStatus?.registration_open ?? false;

  // Best-effort: a failure here hides Google rather than blocking the form.
  const { data: providers } = useQuery({ queryKey: ["auth-providers"], queryFn: getAuthProviders, staleTime: 60_000, retry: false });
  const hasSocial = !!(
    (providers?.google.enabled && providers.google.web_client_id) ||
    (providers?.apple.enabled && providers.apple.web_client_id)
  );

  // Best-effort too: without it the screen only loses the demo box and version line.
  const { data: server } = useQuery({ queryKey: ["server-info"], queryFn: getServerInfo, staleTime: 60_000, retry: false });
  const demo = mode === "login" ? server?.demo : undefined;
  const canResetPassword = server?.features?.auth?.password_reset === true;

  useEffect(() => {
    if (token) navigate(next, { replace: true });
  }, [token, navigate, next]);

  function switchMode(m: Mode) {
    setMode(m);
    setError(null);
    setResetSent(false);
  }

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (mode === "register") {
      if (!displayName.trim()) return setError(t("auth.error.nameMissing"));
      if (password.length < 12) return setError(t("auth.error.passwordShort"));
    }
    setLoading(true);
    try {
      if (mode === "forgot") {
        await forgotPassword(email.trim());
        setResetSent(true);
        return;
      }
      const result =
        mode === "login"
          ? await login(email, password)
          : await register(email, password, displayName.trim(), inviteCode.trim() || undefined);
      setAuth(result.token, result.user, result.refresh_token);
    } catch (err) {
      if (isRateLimited(err)) return setError(t("auth.error.tooManyAttempts"));
      setError(apiErrorMessage(err, mode === "login" ? t("auth.error.badLogin") : t("auth.error.registerFailed")));
    } finally {
      setLoading(false);
    }
  }

  return (
    <AuthLayout>
      <AuthHeading
        title={mode === "login" ? t("auth.signIn.title") : mode === "register" ? t("auth.register.title") : t("auth.forgot.title")}
        subtitle={mode === "forgot" ? t("auth.forgot.body") : undefined}
      />

      {demo && (
        <div className="mb-6 rounded-[10px] border border-accent/40 bg-accent/10 px-4 py-3 text-sm">
          <p className="font-semibold">{t("auth.demo.title")}</p>
          <p className="mt-1 text-muted">{t("auth.demo.body")}</p>
          <dl className="mt-2 grid grid-cols-[auto_1fr] gap-x-3 gap-y-0.5">
            <dt className="text-muted">{t("auth.field.email")}</dt>
            <dd className="select-all break-all font-mono">{demo.email}</dd>
            <dt className="text-muted">{t("auth.field.password")}</dt>
            <dd className="select-all break-all font-mono">{demo.password}</dd>
          </dl>
          <button
            type="button"
            onClick={() => {
              setEmail(demo.email);
              setPassword(demo.password);
              setError(null);
            }}
            className="mt-2 font-medium text-accent-text hover:underline"
          >
            {t("auth.demo.fill")}
          </button>
        </div>
      )}

      {/* One tap, nothing to type or remember — so it comes first. */}
      {hasSocial && mode !== "forgot" && (
        <>
          <div className="space-y-3">
            <GoogleSignInButton providers={providers} onSignedIn={(r) => setAuth(r.token, r.user, r.refresh_token)} onError={setError} />
            <AppleSignInButton providers={providers} onSignedIn={(r) => setAuth(r.token, r.user, r.refresh_token)} onError={setError} />
          </div>
          <div className="my-5 flex items-center gap-3">
            <span className="h-px flex-1 bg-border" />
            <span className="text-xs text-muted">{t("auth.orWithEmail")}</span>
            <span className="h-px flex-1 bg-border" />
          </div>
        </>
      )}

      <form onSubmit={handleSubmit} className="space-y-4" noValidate>
        {mode === "register" && (
          <Input
            label={t("auth.field.name")}
            autoComplete="name"
            autoCapitalize="words"
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            placeholder={t("auth.field.namePlaceholder")}
          />
        )}
        <Input
          label={t("auth.field.email")}
          type="email"
          inputMode="email"
          autoComplete={mode === "login" ? "username" : "email"}
          autoCapitalize="none"
          autoCorrect="off"
          spellCheck={false}
          required
          value={email}
          onChange={(e) => setEmail(e.target.value)}
          placeholder="you@example.com"
        />
        {mode !== "forgot" && (
          <PasswordInput
            label={t("auth.field.password")}
            autoComplete={mode === "login" ? "current-password" : "new-password"}
            required
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            hint={mode === "register" ? t("auth.field.passwordHint") : undefined}
          />
        )}
        {mode === "login" && canResetPassword && (
          <button type="button" onClick={() => switchMode("forgot")} className="text-sm font-medium text-accent hover:underline">
            {t("auth.forgot.link")}
          </button>
        )}
        {mode === "forgot" && resetSent && <p className="text-sm text-muted">{t("auth.forgot.sent")}</p>}
        {mode === "register" &&
          (showInvite ? (
            <Input
              label={t("auth.field.inviteCode")}
              autoComplete="off"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
              autoFocus
              value={inviteCode}
              onChange={(e) => setInviteCode(e.target.value)}
              hint={t("auth.field.inviteCodeHint")}
            />
          ) : (
            <button type="button" onClick={() => setShowInvite(true)} className="text-sm font-medium text-accent hover:underline">
              {t("auth.haveInviteCode")}
            </button>
          ))}

        <FormError>{error}</FormError>

        <Button type="submit" size="lg" className="w-full" loading={loading} disabled={mode === "forgot" && resetSent}>
          {mode === "login" ? t("common.action.signIn") : mode === "register" ? t("common.action.createAccount") : t("auth.forgot.submit")}
        </Button>
      </form>

      {mode === "forgot" && (
        <p className="mt-6 text-center text-sm text-muted">
          <button type="button" onClick={() => switchMode("login")} className="font-medium text-accent hover:underline">
            {t("auth.forgot.back")}
          </button>
        </p>
      )}

      {registrationOpen && mode !== "forgot" && (
        <p className="mt-6 text-center text-sm text-muted">
          {mode === "login"
            ? rich("auth.newHere", {
                link: (c) => (
                  <button type="button" onClick={() => switchMode("register")} className="font-medium text-accent hover:underline">
                    {c}
                  </button>
                ),
              })
            : rich("auth.haveAccount", {
                link: (c) => (
                  <button type="button" onClick={() => switchMode("login")} className="font-medium text-accent hover:underline">
                    {c}
                  </button>
                ),
              })}
        </p>
      )}

      {server?.version && (
        <p className="mt-8 text-center text-xs text-muted">{t("auth.version", { version: server.version })}</p>
      )}
    </AuthLayout>
  );
}
