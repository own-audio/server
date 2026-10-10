// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { MailCheck } from "lucide-react";
import { useQuery } from "@tanstack/react-query";
import { getMe, resendVerification } from "../../api/auth";
import { getServerInfo } from "../../api/server";
import { useAuthStore } from "../../store/authStore";
import { useT } from "../../i18n";

/* An account whose address nothing has vouched for yet: one line until the
   mailed link is used, with "resend" for the mail that never came. Only on a
   server that mails links — elsewhere every account is born verified. */
export default function VerifyEmailBanner() {
  const { user, token, setAuth } = useAuthStore();
  const [sent, setSent] = useState(false);
  const [busy, setBusy] = useState(false);
  const { t } = useT();

  const { data: server } = useQuery({ queryKey: ["server-info"], queryFn: getServerInfo, staleTime: 60_000, retry: false });
  const offered = server?.features?.auth?.email_verification === true;
  const show = offered && !!user && user.email_verified === false;

  // The link may be used on another device; ask again now and then while the banner shows.
  useQuery({
    queryKey: ["me-verified"],
    queryFn: async () => {
      const me = await getMe();
      if (me.email_verified && token) setAuth(token, me);
      return me;
    },
    enabled: show,
    refetchInterval: 30_000,
    retry: false,
  });

  if (!show) return null;

  async function resend() {
    setBusy(true);
    try {
      const r = await resendVerification();
      if (r.verified && token && user) setAuth(token, { ...user, email_verified: true });
      setSent(r.sent);
    } catch {
      setSent(false);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div role="status" className="flex shrink-0 flex-wrap items-center justify-center gap-2 border-b border-border bg-bg-alt px-3 py-1.5 text-xs text-muted">
      <MailCheck className="h-3.5 w-3.5" />
      <span>{t("shell.verifyEmail.message", { email: user!.email })}</span>
      {sent ? (
        <span className="font-medium">{t("shell.verifyEmail.sent")}</span>
      ) : (
        <button type="button" onClick={resend} disabled={busy} className="font-medium text-accent hover:underline disabled:opacity-60">
          {t("shell.verifyEmail.resend")}
        </button>
      )}
    </div>
  );
}
