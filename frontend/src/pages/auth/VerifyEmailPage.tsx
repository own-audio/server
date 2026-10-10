// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { verifyEmail, getMe } from "../../api/auth";
import AuthLayout, { AuthHeading } from "../../components/auth/AuthLayout";
import { useAuthStore } from "../../store/authStore";
import { useT } from "../../i18n";

/** The page behind the mailed confirmation link: `/verify-email?token=…`. */
export default function VerifyEmailPage() {
  const [params] = useSearchParams();
  const token = params.get("token") ?? "";
  const [state, setState] = useState<"working" | "done" | "invalid">(token ? "working" : "invalid");
  const { token: session, setAuth } = useAuthStore();
  const { t } = useT();

  useEffect(() => {
    if (!token) return;
    let cancelled = false;
    (async () => {
      try {
        await verifyEmail(token);
        if (cancelled) return;
        setState("done");
        // Signed in on this device: the banner goes away without a reload.
        if (session) {
          try {
            setAuth(session, await getMe());
          } catch {
            /* the next load picks it up */
          }
        }
      } catch {
        if (!cancelled) setState("invalid");
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [token, session, setAuth]);

  return (
    <AuthLayout>
      <AuthHeading title={t("auth.verify.title")} />
      <p className="text-sm text-muted">
        {state === "working" ? t("auth.verify.working") : state === "done" ? t("auth.verify.done") : t("auth.verify.invalid")}
      </p>
      {state !== "working" && (
        <Link to={session ? "/" : "/auth/login"} className="mt-6 block text-center text-sm font-medium text-accent hover:underline">
          {session ? t("auth.verify.open") : t("common.action.signIn")}
        </Link>
      )}
    </AuthLayout>
  );
}
