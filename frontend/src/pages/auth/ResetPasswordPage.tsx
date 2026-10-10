// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { resetPassword } from "../../api/auth";
import AuthLayout, { AuthHeading, FormError } from "../../components/auth/AuthLayout";
import { apiErrorMessage, isRateLimited } from "../../lib/apiError";
import { Button, PasswordInput } from "../../components/ui";
import { useT } from "../../i18n";

/** The page behind the mailed "forgot password" link: `/reset-password?token=…`. */
export default function ResetPasswordPage() {
  const [params] = useSearchParams();
  const token = params.get("token") ?? "";
  const [password, setPassword] = useState("");
  const [again, setAgain] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [done, setDone] = useState(false);
  const { t } = useT();

  async function handleSubmit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (password.length < 12) return setError(t("auth.error.passwordShort"));
    if (password !== again) return setError(t("auth.reset.mismatch"));
    setLoading(true);
    try {
      await resetPassword(token, password);
      setDone(true);
    } catch (err) {
      if (isRateLimited(err)) return setError(t("auth.error.tooManyAttempts"));
      setError(apiErrorMessage(err, t("auth.reset.invalid")));
    } finally {
      setLoading(false);
    }
  }

  return (
    <AuthLayout>
      <AuthHeading title={t("auth.reset.title")} />
      {done || !token ? (
        <div className="space-y-4">
          <p className="text-sm text-muted">{done ? t("auth.reset.done") : t("auth.reset.invalid")}</p>
          <Link to="/auth/login" className="block text-center text-sm font-medium text-accent hover:underline">
            {t("common.action.signIn")}
          </Link>
        </div>
      ) : (
        <form onSubmit={handleSubmit} className="space-y-4" noValidate>
          <PasswordInput
            label={t("auth.reset.newPassword")}
            autoComplete="new-password"
            required
            autoFocus
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            hint={t("auth.field.passwordHint")}
          />
          <PasswordInput
            label={t("auth.reset.again")}
            autoComplete="new-password"
            required
            value={again}
            onChange={(e) => setAgain(e.target.value)}
          />
          <FormError>{error}</FormError>
          <Button type="submit" size="lg" className="w-full" loading={loading}>
            {t("auth.reset.submit")}
          </Button>
        </form>
      )}
    </AuthLayout>
  );
}
