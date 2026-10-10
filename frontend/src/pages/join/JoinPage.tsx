// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { useNavigate, useParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Users } from "lucide-react";
import { acceptInvite, claimAccount, previewJoin } from "../../api/family";
import { login, register } from "../../api/auth";
import { useAuthStore } from "../../store/authStore";
import AuthLayout, { AuthHeading, FormError } from "../../components/auth/AuthLayout";
import { Button, Input, PasswordInput, Skeleton, toast } from "../../components/ui";
import { apiErrorMessage, isRateLimited } from "../../lib/apiError";
import { useT } from "../../i18n";

/**
 * Where an invite link or QR lands. Public: it has to work signed out, since
 * that is the whole point of handing someone a code.
 *
 * Four states, from `GET /join/{code}`: a valid code for someone new, a valid
 * code for someone already signed in, a `claim` code that activates an account
 * an admin created, and a code that is expired, used up or unknown.
 */
export default function JoinPage() {
  const { code = "" } = useParams<{ code: string }>();
  const { t, rich } = useT();
  const navigate = useNavigate();
  const { token, setAuth } = useAuthStore();

  const [mode, setMode] = useState<"register" | "signin">("register");
  const [name, setName] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const { data: preview, isLoading, isError } = useQuery({
    queryKey: ["join", code],
    queryFn: () => previewJoin(code),
    retry: false,
    enabled: code.length > 0,
  });

  function landInside(familyName: string | null) {
    toast.success(familyName ? t("join.welcome", { family: familyName }) : t("join.youreIn"));
    navigate("/", { replace: true });
  }

  // ── Already signed in: one button ───────────────────────────────────────
  async function joinAsMe() {
    setBusy(true);
    setError(null);
    try {
      await acceptInvite(code);
      landInside(preview?.family_name ?? null);
    } catch (err) {
      setError(apiErrorMessage(err, t("join.error.joinFailed")));
    } finally {
      setBusy(false);
    }
  }

  // ── New account, or sign in and then accept ─────────────────────────────
  async function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (mode === "register") {
      if (!name.trim()) return setError(t("auth.error.nameMissing"));
      if (password.length < 12) return setError(t("auth.error.passwordShort"));
    }
    setBusy(true);
    try {
      if (mode === "register") {
        const result = await register(email.trim(), password, name.trim(), code);
        setAuth(result.token, result.user, result.refresh_token);
      } else {
        const result = await login(email.trim(), password);
        setAuth(result.token, result.user, result.refresh_token);
        // Signing in doesn't redeem the code, so accept it explicitly.
        await acceptInvite(code);
      }
      landInside(preview?.family_name ?? null);
    } catch (err) {
      if (isRateLimited(err)) return setError(t("auth.error.tooManyAttempts"));
      setError(apiErrorMessage(err, t(mode === "register" ? "auth.error.registerFailed" : "auth.error.badLogin")));
    } finally {
      setBusy(false);
    }
  }

  // ── Claim: set a password on an account someone made for you ────────────
  async function claim(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (password.length < 12) return setError(t("auth.error.passwordShort"));
    setBusy(true);
    try {
      const result = await claimAccount(code, password);
      setAuth(result.token, result.user, result.refresh_token);
      landInside(preview?.family_name ?? null);
    } catch (err) {
      setError(apiErrorMessage(err, t("join.error.claimFailed")));
    } finally {
      setBusy(false);
    }
  }

  if (isLoading) {
    return (
      <AuthLayout>
        <Skeleton className="h-40" />
      </AuthLayout>
    );
  }

  // An unknown code is a 404; expired and used-up codes report their status.
  if (isError || !preview || preview.status !== "valid") {
    const reason = t(
      preview?.status === "expired"
        ? "join.invalid.expired"
        : preview?.status === "exhausted"
          ? "join.invalid.exhausted"
          : "join.invalid.unknown"
    );
    return (
      <AuthLayout>
        <AuthHeading title={t("join.invalid.title")} subtitle={reason} />
        <p className="text-sm text-muted">{t("join.invalid.askAgain")}</p>
        <Button variant="secondary" className="mt-6" onClick={() => navigate("/auth/login")}>
          {t("join.invalid.goToSignIn")}
        </Button>
      </AuthLayout>
    );
  }

  const familyLine = (
    <div className="mb-6 flex items-start gap-3 rounded-card border border-border bg-bg-alt px-4 py-3">
      <Users className="mt-0.5 h-5 w-5 shrink-0 text-accent" />
      <p className="text-sm">
        {preview.inviter_name
          ? rich("join.invitedBy", { inviter: preview.inviter_name, family: preview.family_name ?? "", b: (c) => <strong>{c}</strong> })
          : rich("join.invitedBySomeone", { family: preview.family_name ?? "", b: (c) => <strong>{c}</strong> })}
        {preview.member_count != null && (
          <span className="block text-xs text-muted">{t("join.alreadyThere", { count: preview.member_count })}</span>
        )}
      </p>
    </div>
  );

  if (preview.kind === "claim" && preview.claim) {
    return (
      <AuthLayout>
        {familyLine}
        <AuthHeading
          title={t("join.claim.title")}
          subtitle={t("join.claim.subtitle", { email: preview.claim.login_email })}
        />
        <form onSubmit={claim} className="space-y-4" noValidate>
          <PasswordInput
            label={t("auth.field.password")}
            autoComplete="new-password"
            autoFocus
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            hint={t("auth.field.passwordHint")}
          />
          <FormError>{error}</FormError>
          <Button type="submit" size="lg" className="w-full" loading={busy}>
            {t("join.claim.start")}
          </Button>
        </form>
      </AuthLayout>
    );
  }

  if (token) {
    return (
      <AuthLayout>
        {familyLine}
        <AuthHeading title={t("join.signedIn.title")} subtitle={t("join.signedIn.subtitle")} />
        <FormError>{error}</FormError>
        <Button size="lg" className="w-full" loading={busy} onClick={joinAsMe}>
          {t("join.signedIn.join", { family: preview.family_name ?? "" })}
        </Button>
        <Button variant="ghost" className="mt-2 w-full" onClick={() => navigate("/")}>
          {t("join.notNow")}
        </Button>
      </AuthLayout>
    );
  }

  return (
    <AuthLayout>
      {familyLine}
      <AuthHeading
        title={t(mode === "register" ? "auth.register.title" : "join.signIn.title")}
        subtitle={t(mode === "register" ? "join.register.subtitle" : "join.signIn.subtitle")}
      />
      <form onSubmit={submit} className="space-y-4" noValidate>
        {mode === "register" && <Input label={t("auth.field.name")} autoCapitalize="words" autoComplete="name" autoFocus value={name} onChange={(e) => setName(e.target.value)} />}
        <Input label={t("auth.field.email")} type="email" inputMode="email" autoCapitalize="none" autoCorrect="off" spellCheck={false} autoComplete="email" required value={email} onChange={(e) => setEmail(e.target.value)} />
        <PasswordInput
          label={t("auth.field.password")}
          autoComplete={mode === "register" ? "new-password" : "current-password"}
          required
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          hint={mode === "register" ? t("auth.field.passwordHint") : undefined}
        />
        <FormError>{error}</FormError>
        <Button type="submit" size="lg" className="w-full" loading={busy}>
          {t(mode === "register" ? "join.register.submit" : "join.signIn.submit")}
        </Button>
      </form>

      <p className="mt-6 text-center text-sm text-muted">
        {mode === "register"
          ? rich("auth.haveAccount", {
              link: (c) => (
                <button type="button" onClick={() => { setMode("signin"); setError(null); }} className="font-medium text-accent hover:underline">
                  {c}
                </button>
              ),
            })
          : rich("auth.newHere", {
              link: (c) => (
                <button type="button" onClick={() => { setMode("register"); setError(null); }} className="font-medium text-accent hover:underline">
                  {c}
                </button>
              ),
            })}
      </p>
    </AuthLayout>
  );
}
