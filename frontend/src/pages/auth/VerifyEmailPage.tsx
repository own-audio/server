// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { Link, useSearchParams } from "react-router-dom";
import { verifyEmail, getMe } from "../../api/auth";
import AuthLayout, { AuthHeading } from "../../components/auth/AuthLayout";
import { useAuthStore } from "../../store/authStore";
import { useT } from "../../i18n";

const SITEKEY = import.meta.env.VITE_TURNSTILE_SITEKEY;
/** How long the human check may take before the address is confirmed without it. */
const CHECK_TIMEOUT_MS = 20_000;

interface Turnstile {
  render(el: HTMLElement, opts: Record<string, unknown>): string;
  remove(id: string): void;
}
declare global {
  interface Window {
    turnstile?: Turnstile;
  }
}

let script: Promise<Turnstile> | null = null;
function loadTurnstile(): Promise<Turnstile> {
  script ??= new Promise((resolve, reject) => {
    const s = document.createElement("script");
    s.src = "https://challenges.cloudflare.com/turnstile/v0/api.js?render=explicit";
    s.async = true;
    s.onload = () => (window.turnstile ? resolve(window.turnstile) : reject(new Error("turnstile")));
    s.onerror = () => reject(new Error("turnstile"));
    document.head.appendChild(s);
  });
  return script;
}

/** The page behind the mailed confirmation link: `/verify-email?token=…`. With a Turnstile site
 *  key in the build, a human check comes first and its token goes along; whatever happens to
 *  the check, the address is confirmed (only what the server grants for it may differ). */
export default function VerifyEmailPage() {
  const [params] = useSearchParams();
  const token = params.get("token") ?? "";
  const [state, setState] = useState<"check" | "working" | "done" | "invalid">(
    token ? (SITEKEY ? "check" : "working") : "invalid"
  );
  const [humanCheck, setHumanCheck] = useState<string>();
  const widget = useRef<HTMLDivElement>(null);
  const { token: session, setAuth } = useAuthStore();
  const { t } = useT();

  useEffect(() => {
    if (state !== "check" || !SITEKEY || !widget.current) return;
    const el = widget.current;
    let id: string | undefined;
    let api: Turnstile | undefined;
    const proceed = (check?: string) => {
      setHumanCheck(check);
      setState("working");
    };
    const timer = window.setTimeout(() => proceed(), CHECK_TIMEOUT_MS);
    loadTurnstile()
      .then((ts) => {
        api = ts;
        id = ts.render(el, {
          sitekey: SITEKEY,
          theme: "auto",
          action: "verify-email",
          callback: (check: string) => proceed(check),
          "error-callback": () => proceed(),
          "timeout-callback": () => proceed(),
        });
      })
      .catch(() => proceed());
    return () => {
      window.clearTimeout(timer);
      if (api && id) api.remove(id);
    };
  }, [state]);

  useEffect(() => {
    if (state !== "working") return;
    let cancelled = false;
    (async () => {
      try {
        await verifyEmail(token, humanCheck);
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
  }, [state, token, humanCheck, session, setAuth]);

  return (
    <AuthLayout>
      <AuthHeading title={t("auth.verify.title")} />
      <p className="text-sm text-muted">
        {state === "check"
          ? t("auth.verify.check")
          : state === "working"
            ? t("auth.verify.working")
            : state === "done"
              ? t("auth.verify.done")
              : t("auth.verify.invalid")}
      </p>
      {state === "check" && <div ref={widget} className="mt-4 flex justify-center" />}
      {(state === "done" || state === "invalid") && (
        <Link to={session ? "/" : "/auth/login"} className="mt-6 block text-center text-sm font-medium text-accent hover:underline">
          {session ? t("auth.verify.open") : t("common.action.signIn")}
        </Link>
      )}
    </AuthLayout>
  );
}
