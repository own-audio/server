// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { useSearchParams } from "react-router-dom";
import { useQuery } from "@tanstack/react-query";
import { Check, Tv, X } from "lucide-react";
import {
  approveDeviceRequest,
  denyDeviceRequest,
  describeDeviceRequest,
  formatDeviceCode,
} from "../../api/deviceAuth";
import AuthLayout, { AuthHeading, FormError } from "../../components/auth/AuthLayout";
import { Button, Input, Skeleton } from "../../components/ui";
import { apiErrorMessage } from "../../lib/apiError";
import { useT } from "../../i18n";

/**
 * Where the code shown on a TV gets approved. The QR on the TV opens this page
 * with `?code=` already filled in, so the usual path is: read the screen, point
 * a camera at it, press one button.
 *
 * Behind the sign-in gate on purpose — approving is an account action, and the
 * visitor signs in here with whatever they normally use, including Google,
 * which is exactly what tvOS cannot offer.
 *
 * Nothing of this browser's session goes to the device: approving only marks
 * the request, and the device's own poll is what mints its tokens.
 */
export default function LinkPage() {
  const { t } = useT();
  const [params, setParams] = useSearchParams();
  const code = formatDeviceCode(params.get("code") ?? "");

  const [typed, setTyped] = useState("");
  const [decision, setDecision] = useState<"approved" | "denied" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const {
    data: request,
    isLoading,
    isError,
  } = useQuery({
    queryKey: ["device-request", code],
    queryFn: () => describeDeviceRequest(code),
    retry: false,
    enabled: code.length > 0 && decision === null,
  });

  function submitCode(e: FormEvent) {
    e.preventDefault();
    setError(null);
    setParams({ code: formatDeviceCode(typed) }, { replace: true });
  }

  async function decide(approve: boolean) {
    setBusy(true);
    setError(null);
    try {
      if (approve) {
        await approveDeviceRequest(code);
      } else {
        await denyDeviceRequest(code);
      }
      setDecision(approve ? "approved" : "denied");
    } catch (err) {
      setError(apiErrorMessage(err, t("link.error.answerFailed")));
    } finally {
      setBusy(false);
    }
  }

  // ── Done ────────────────────────────────────────────────────────────────
  if (decision) {
    return (
      <AuthLayout>
        <AuthHeading
          title={t(decision === "approved" ? "link.approved.title" : "link.denied.title")}
          subtitle={t(decision === "approved" ? "link.approved.subtitle" : "link.denied.subtitle")}
        />
      </AuthLayout>
    );
  }

  // ── No code yet: type the one on the screen ─────────────────────────────
  if (!code) {
    return (
      <AuthLayout>
        <AuthHeading title={t("link.enter.title")} subtitle={t("link.enter.subtitle")} />
        <form onSubmit={submitCode} className="space-y-4">
          <Input
            value={typed}
            onChange={(e) => setTyped(formatDeviceCode(e.target.value))}
            placeholder="ABCD-EFGH"
            autoFocus
            aria-label={t("link.enter.codeLabel")}
            className="text-center text-2xl tracking-widest"
          />
          <Button type="submit" className="w-full" disabled={typed.length < 9}>
            {t("link.continue")}
          </Button>
        </form>
      </AuthLayout>
    );
  }

  if (isLoading) {
    return (
      <AuthLayout>
        <Skeleton className="h-8 w-48" />
        <Skeleton className="mt-4 h-24 w-full" />
      </AuthLayout>
    );
  }

  // A code that is unknown, already used, denied or past its ten minutes is one
  // answer on purpose: nothing here helps anyone work out which codes exist.
  if (isError || !request) {
    return (
      <AuthLayout>
        <AuthHeading
          title={t("link.expired.title")}
          subtitle={t("link.expired.subtitle")}
        />
        <Button className="w-full" onClick={() => setParams({}, { replace: true })}>
          {t("link.expired.enterAnother")}
        </Button>
      </AuthLayout>
    );
  }

  // ── The decision ────────────────────────────────────────────────────────
  return (
    <AuthLayout>
      <AuthHeading
        title={t("link.decide.title")}
        subtitle={t("link.decide.subtitle")}
      />

      <div className="flex items-center gap-3 rounded-lg border border-subtle p-4">
        <Tv className="h-5 w-5 text-muted" aria-hidden />
        <div className="min-w-0">
          <div className="truncate font-medium">{request.device_name ?? "Apple TV"}</div>
          <div className="text-sm text-muted">{t("link.decide.code", { code: request.user_code })}</div>
        </div>
      </div>

      {error && <FormError>{error}</FormError>}

      <div className="mt-4 flex gap-3">
        <Button className="flex-1" disabled={busy} onClick={() => decide(true)}>
          <Check className="mr-2 h-4 w-4" aria-hidden />
          {t("link.decide.approve")}
        </Button>
        <Button variant="secondary" className="flex-1" disabled={busy} onClick={() => decide(false)}>
          <X className="mr-2 h-4 w-4" aria-hidden />
          {t("link.decide.deny")}
        </Button>
      </div>
    </AuthLayout>
  );
}
