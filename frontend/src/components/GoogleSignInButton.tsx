// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { GoogleOAuthProvider, GoogleLogin } from "@react-oauth/google";
import { loginWithGoogle, type AuthProviders } from "../api/auth";
import type { LoginResponse } from "../api/types";
import { useTheme, resolvedTheme } from "../lib/theme";
import { t, useI18n } from "../i18n";

interface Props {
  providers: AuthProviders | undefined;
  onSignedIn: (result: LoginResponse) => void;
  onError: (message: string) => void;
}

/**
 * Google sign-in for the web console. Renders nothing unless the server both
 * advertises Google and knows its web client id — the id arrives from
 * `/auth/providers` rather than a build-time env var, because this UI ships
 * inside the backend and self-hosters must not have to rebuild it.
 *
 * Google publishes no official React SDK; `@react-oauth/google` is a thin
 * community wrapper over Google Identity Services, which is Google's own.
 */
export default function GoogleSignInButton({ providers, onSignedIn, onError }: Props) {
  const clientId = providers?.google.enabled ? providers.google.web_client_id : null;
  const preference = useTheme((s) => s.preference);
  const locale = useI18n((s) => s.locale);
  const [width, setWidth] = useState<number | null>(null);
  const measureRef = useRef<HTMLDivElement>(null);

  /* Google draws the button in an iframe of a fixed pixel width (200–400), so
     it cannot shrink with the page — a fixed 384 pushed the whole form past
     the edge of a phone. Measure the column and hand Google that width. */
  useEffect(() => {
    const el = measureRef.current;
    if (!el) return;
    const observer = new ResizeObserver(([entry]) => {
      setWidth(Math.max(200, Math.min(400, Math.floor(entry.contentRect.width))));
    });
    observer.observe(el);
    return () => observer.disconnect();
  }, [clientId]);

  if (!clientId) return null;
  const dark = resolvedTheme(preference) === "dark";

  return (
    <div>
      <GoogleOAuthProvider clientId={clientId} locale={locale}>
        <div ref={measureRef} className="flex min-h-10 justify-center">
          {width != null && (
            <GoogleLogin
              width={String(width)}
              size="large"
              shape="pill"
              // The same words on sign-in and sign-up — Google does either —
              // and in the app's language, not the browser's.
              text="continue_with"
              theme={dark ? "filled_black" : "outline"}
              onSuccess={async (credentialResponse) => {
                const idToken = credentialResponse.credential;
                if (!idToken) {
                  onError(t("common.error.googleNoToken"));
                  return;
                }
                try {
                  onSignedIn(await loginWithGoogle(idToken));
                } catch (err: unknown) {
                  // The server's own wording is more use than ours here: it is
                  // usually something to act on ("registration is not open")
                  // rather than something to retry.
                  onError(
                    (err as { response?: { data?: { error?: string } } })?.response?.data?.error ??
                      t("common.error.googleSignIn"),
                  );
                }
              }}
              onError={() => onError(t("common.error.googleSignIn"))}
            />
          )}
        </div>
      </GoogleOAuthProvider>
    </div>
  );
}
