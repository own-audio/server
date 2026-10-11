// SPDX-License-Identifier: AGPL-3.0-or-later
import { useQuery } from "@tanstack/react-query";
import { getSecurityEvents, type SecurityEvent } from "../../api/auth";
import { Skeleton } from "../../components/ui";
import { timeAgo } from "../../lib/format";
import { useT, type PlainKey } from "../../i18n";

const KIND_KEYS: Record<string, PlainKey> = {
  "login.ok": "settings.activity.kind.loginOk",
  "login.failed": "settings.activity.kind.loginFailed",
  "login.locked": "settings.activity.kind.loginLocked",
  "login.mfa_pending": "settings.activity.kind.loginMfaPending",
  "login.mfa_ok": "settings.activity.kind.loginMfaOk",
  "login.mfa_failed": "settings.activity.kind.loginMfaFailed",
  "password.changed": "settings.activity.kind.passwordChanged",
  "password.reset": "settings.activity.kind.passwordReset",
  "password.reset_requested": "settings.activity.kind.passwordResetRequested",
  "email.verified": "settings.activity.kind.emailVerified",
  "totp.enabled": "settings.activity.kind.totpEnabled",
  "totp.disabled": "settings.activity.kind.totpDisabled",
  "session.signed_out": "settings.activity.kind.sessionSignedOut",
  "session.revoked": "settings.activity.kind.sessionRevoked",
  "session.revoked_all": "settings.activity.kind.sessionRevokedAll",
  "account.created": "settings.activity.kind.accountCreated",
  "account.changed": "settings.activity.kind.accountChanged",
  "account.deleted": "settings.activity.kind.accountDeleted",
  "family.member_changed": "settings.activity.kind.familyMemberChanged",
  "family.member_removed": "settings.activity.kind.familyMemberRemoved",
  "family.invite_created": "settings.activity.kind.familyInviteCreated",
  "family.invite_deleted": "settings.activity.kind.familyInviteDeleted"
};

/* The account's own security trail: sign-ins and failures, password and
   two-factor changes, sessions, what an admin did to the account. Read-only;
   its point is that a sign-in you did not do stands out. */
export default function SecurityActivity() {
  const { t } = useT();
  const { data, isLoading } = useQuery({ queryKey: ["security-events"], queryFn: getSecurityEvents, retry: false });

  if (isLoading) return <Skeleton className="h-24" />;
  const events = data ?? [];
  if (events.length === 0) return <p className="text-sm text-muted">{t("settings.activity.empty")}</p>;

  return (
    <ul className="divide-y divide-border rounded-card border border-border">
      {events.slice(0, 30).map((e) => (
        <li key={e.id} className="flex flex-wrap items-baseline gap-x-3 gap-y-0.5 px-4 py-2.5 text-sm">
          <span className="font-medium">{label(e, t)}</span>
          <span className="text-xs text-muted">{timeAgo(e.at)}</span>
          {(e.ip || e.user_agent) && (
            <span className="min-w-0 flex-1 truncate text-xs text-muted" title={e.user_agent ?? undefined}>
              {[e.ip, shortAgent(e.user_agent)].filter(Boolean).join(" · ")}
            </span>
          )}
        </li>
      ))}
    </ul>
  );
}

function label(e: SecurityEvent, t: ReturnType<typeof useT>["t"]): string {
  const key = KIND_KEYS[e.kind];
  // An unknown kind (a newer server) shows as it is rather than as nothing.
  const shown = key ? t(key) : e.kind;
  return e.actor_id ? t("settings.activity.byAdmin", { what: shown }) : shown;
}

/** "Chrome on macOS" is not derivable without a parser; the first product token is enough to tell devices apart. */
function shortAgent(ua: string | null | undefined): string | null {
  if (!ua) return null;
  const m = ua.match(/^([A-Za-z][\w.-]*\/[\w.-]+)/);
  return m ? m[1] : ua.slice(0, 40);
}
