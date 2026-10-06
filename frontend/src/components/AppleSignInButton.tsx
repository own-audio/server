// SPDX-License-Identifier: AGPL-3.0-or-later
import { useEffect, useRef, useState } from "react";
import { loginWithApple, type AuthProviders } from "../api/auth";
import type { LoginResponse } from "../api/types";
import { Button } from "./ui/Button";
import { useT, type Locale } from "../i18n";

interface Props {
  providers: AuthProviders | undefined;
  onSignedIn: (result: LoginResponse) => void;
  onError: (message: string) => void;
}

interface AppleAuthResponse {
  authorization: { id_token: string; code: string };
  user?: { name?: { firstName?: string; lastName?: string } };
}

declare global {
  interface Window {
    AppleID?: {
      auth: {
        init: (cfg: { clientId: string; scope: string; redirectURI: string; usePopup: boolean }) => void;
        signIn: () => Promise<AppleAuthResponse>;
      };
    };
  }
}

// Apple's popup speaks the SDK's language, which is fixed by the script's path.
const sdkUrl = (locale: Locale) =>
  `https://appleid.cdn-apple.com/appleauth/static/jsapi/appleid/1/${locale === "cs" ? "cs_CZ" : "en_US"}/appleid.auth.js`;

/**
 * Sign in with Apple for the web. Renders nothing unless the server advertises
 * Apple *and* a Services ID — Apple's JS SDK needs the id as `clientId`, and
 * the redirect URI must be registered against that Services ID in the Apple
 * developer console (the popup flow still requires one). The SDK is loaded
 * on demand so a server without Apple never contacts Apple at all.
 */
export default function AppleSignInButton({ providers, onSignedIn, onError }: Props) {
  const clientId = providers?.apple.enabled ? providers.apple.web_client_id : null;
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(false);
  const initialised = useRef(false);
  const { t, locale } = useT();

  useEffect(() => {
    if (!clientId) return;
    let cancelled = false;
    const init = () => {
      if (cancelled || !window.AppleID || initialised.current) return;
      window.AppleID.auth.init({
        clientId,
        scope: "name email",
        redirectURI: `${window.location.origin}/auth/login`,
        usePopup: true,
      });
      initialised.current = true;
      setReady(true);
    };
    if (window.AppleID) {
      init();
    } else {
      const script = document.createElement("script");
      script.src = sdkUrl(locale);
      script.async = true;
      script.onload = init;
      document.head.appendChild(script);
    }
    return () => {
      cancelled = true;
    };
  }, [clientId, locale]);

  if (!clientId) return null;

  async function signIn() {
    if (!window.AppleID) return;
    setBusy(true);
    try {
      const res = await window.AppleID.auth.signIn();
      const name = res.user?.name;
      const fullName = name ? [name.firstName, name.lastName].filter(Boolean).join(" ") || undefined : undefined;
      onSignedIn(await loginWithApple(res.authorization.id_token, fullName));
    } catch (err: unknown) {
      // Apple rejects a closed popup with {error: "popup_closed_by_user"}; that's not an error to show.
      const code = (err as { error?: string })?.error;
      if (code === "popup_closed_by_user") return;
      onError((err as { response?: { data?: { error?: string } } })?.response?.data?.error ?? t("common.error.appleSignIn"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Button
      type="button"
      variant="secondary"
      size="lg"
      className="w-full bg-fg text-bg hover:bg-fg/90"
      onClick={signIn}
      disabled={!ready}
      loading={busy}
      icon={
        <svg viewBox="0 0 24 24" className="h-4 w-4 fill-current" aria-hidden="true">
          <path d="M16.37 12.7c-.03-2.6 2.13-3.85 2.22-3.91-1.21-1.77-3.09-2.01-3.76-2.04-1.6-.16-3.12.94-3.93.94-.81 0-2.06-.92-3.39-.9-1.74.03-3.35 1.01-4.25 2.58-1.81 3.14-.46 7.79 1.3 10.34.86 1.25 1.89 2.65 3.24 2.6 1.3-.05 1.79-.84 3.36-.84 1.57 0 2.01.84 3.39.81 1.4-.02 2.29-1.27 3.14-2.52.99-1.45 1.4-2.85 1.42-2.92-.03-.01-2.73-1.05-2.74-4.14zM13.8 5.06c.72-.87 1.2-2.08 1.07-3.29-1.03.04-2.29.69-3.03 1.56-.66.77-1.25 2-1.09 3.18 1.15.09 2.33-.58 3.05-1.45z" />
        </svg>
      }
    >
      {t("common.action.continueWithApple")}
    </Button>
  );
}
