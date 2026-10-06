// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { AlertTriangle, Package, Wallet } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../../components/ui/CloudIcon";
import { createTopup, formatBytes, formatMicro, getBilling, setBillingAlerts } from "../../api/billing";
import { getFamily, isFamilyAdmin } from "../../api/family";
import { listTrash } from "../../api/trash";
import { Link } from "react-router-dom";
import { Page } from "../../components/shell/SplitView";
import { Button, Input, Pill, Skeleton, toast } from "../../components/ui";
import { apiErrorMessage } from "../../lib/apiError";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";

const KINDS: { key: "audiobooks_bytes" | "podcasts_bytes" | "music_bytes" | "other_bytes"; label: PlainKey; bar: string; icon: React.ReactNode }[] = [
  { key: "audiobooks_bytes" as const, label: "common.kind.audiobooks", bar: "bg-book", icon: <BookIcon className="h-4 w-4 text-book" /> },
  { key: "podcasts_bytes" as const, label: "common.kind.podcasts", bar: "bg-podcast", icon: <PodcastIcon className="h-4 w-4 text-podcast" /> },
  { key: "music_bytes" as const, label: "common.kind.music", bar: "bg-music", icon: <MusicIcon className="h-4 w-4 text-music" /> },
  { key: "other_bytes" as const, label: "billing.usage.other", bar: "bg-accent", icon: <Package className="h-4 w-4 text-accent" /> },
];

function TopUp({ presets, currency, min, max }: { presets: number[]; currency: string; min: number; max: number }) {
  const { t } = useT();
  const [custom, setCustom] = useState("");
  const [busy, setBusy] = useState<number | null>(null);

  async function go(amountMicro: number) {
    setBusy(amountMicro);
    try {
      const { checkout_url } = await createTopup(amountMicro);
      // Stripe Checkout is hosted, so paying deliberately leaves the app.
      window.location.assign(checkout_url);
    } catch (err) {
      toast.error(t("billing.topup.error"), apiErrorMessage(err, t("billing.topup.errorDetail")));
      setBusy(null);
    }
  }

  return (
    <div className="mt-3 flex flex-wrap items-center gap-2">
      {presets.map((p) => (
        <Button key={p} size="sm" variant="secondary" loading={busy === p} onClick={() => go(p)}>
          {formatMicro(p, currency)}
        </Button>
      ))}
      <form
        className="flex items-center gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          const micro = Math.round(Number(custom) * 1_000_000);
          if (!Number.isFinite(micro) || micro < min || micro > max) {
            toast.error(t("billing.topup.range", { min: formatMicro(min, currency), max: formatMicro(max, currency) }));
            return;
          }
          void go(micro);
        }}
      >
        <Input
          aria-label={t("billing.topup.custom")}
          inputMode="decimal"
          value={custom}
          onChange={(e) => setCustom(e.target.value)}
          placeholder={t("billing.topup.other")}
          className="h-8 w-24 text-xs"
        />
        <Button size="sm" type="submit" disabled={!custom.trim()}>
          {t("common.action.add")}
        </Button>
      </form>
    </div>
  );
}

function AlertSettings({
  initial,
  currency,
}: {
  initial: { min_balance_micro: number | null; min_days_remaining: number | null };
  currency: string;
}) {
  const { t } = useT();
  const qc = useQueryClient();
  const [balance, setBalance] = useState(initial.min_balance_micro != null ? String(initial.min_balance_micro / 1_000_000) : "");
  const [days, setDays] = useState(initial.min_days_remaining != null ? String(initial.min_days_remaining) : "");

  const save = useMutation({
    // A full replace of both rules: empty means "turn that one off".
    mutationFn: () =>
      setBillingAlerts({
        min_balance_micro: balance.trim() ? Math.round(Number(balance) * 1_000_000) : null,
        min_days_remaining: days.trim() ? Math.round(Number(days)) : null,
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["billing"] });
      toast.success(t("billing.alerts.saved"));
    },
    onError: (err) => toast.error(t("billing.alerts.saveError"), apiErrorMessage(err, t("billing.tryAgain"))),
  });

  return (
    <section className="mt-8 rounded-card border border-border p-4">
      <p className="text-sm font-medium">{t("billing.alerts.title")}</p>
      <p className="mt-0.5 text-xs text-muted">{t("billing.alerts.description")}</p>
      <div className="mt-3 flex flex-wrap items-end gap-3">
        <Input
          label={t("billing.alerts.balance", { currency })}
          inputMode="decimal"
          value={balance}
          onChange={(e) => setBalance(e.target.value)}
          className="w-40"
        />
        <Input label={t("billing.alerts.days")} inputMode="numeric" value={days} onChange={(e) => setDays(e.target.value)} className="w-48" />
        <Button size="sm" loading={save.isPending} onClick={() => save.mutate()}>
          {t("common.action.save")}
        </Button>
      </div>
    </section>
  );
}

export default function BillingPage() {
  const { t, rich, locale } = useT();
  const { data: family } = useQuery({ queryKey: ["family"], queryFn: getFamily, retry: false });
  const { data, isLoading } = useQuery({ queryKey: ["billing"], queryFn: getBilling });
  const isAdmin = isFamilyAdmin(family?.my_role);
  const trashScope = isAdmin ? "family" : "mine";
  const { data: trash = [] } = useQuery({ queryKey: ["trash", trashScope], queryFn: () => listTrash(trashScope), retry: false });
  const trashBytes = trash.reduce((sum, i) => sum + i.size_bytes, 0);

  if (isLoading || !data) {
    return (
      <Page title={t("billing.page.title")} width="max-w-3xl">
        <Skeleton className="h-40" />
      </Page>
    );
  }

  const { storage, pricing, payments } = data;
  const currency = pricing.currency;
  const total = Math.max(1, storage.total_bytes);

  return (
    <Page title={t("billing.page.title")} width="max-w-3xl">
      {data.depleted && (
        <div className="mb-4 flex items-start gap-3 rounded-card border border-error/40 bg-error/10 px-4 py-3">
          <AlertTriangle className="mt-0.5 h-5 w-5 shrink-0 text-error" />
          <p className="text-sm">
            <strong>{t("billing.depleted.title")}</strong>
            <span className="block text-muted">{t("billing.depleted.body")}</span>
          </p>
        </div>
      )}

      <div className="grid gap-3 sm:grid-cols-3">
        <div className="rounded-card border border-border px-4 py-3">
          <p className="text-xs uppercase tracking-wide text-muted">{t("billing.credit")}</p>
          <p className={cn("mt-1 text-2xl font-semibold tabular-nums", data.balance_micro <= 0 && "text-error")}>
            {formatMicro(data.balance_micro, currency)}
          </p>
        </div>
        <div className="rounded-card border border-border px-4 py-3">
          <p className="text-xs uppercase tracking-wide text-muted">{t("billing.daysLeft")}</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{data.days_remaining ?? "—"}</p>
          {data.runs_out_on && (
            <p className="mt-0.5 text-xs text-muted">{t("billing.runsOut", { date: new Date(data.runs_out_on).toLocaleDateString(locale) })}</p>
          )}
        </div>
        <div className="rounded-card border border-border px-4 py-3">
          <p className="text-xs uppercase tracking-wide text-muted">{t("billing.costing")}</p>
          <p className="mt-1 text-2xl font-semibold tabular-nums">{formatMicro(data.estimated_monthly_cost_micro, currency)}</p>
          <p className="mt-0.5 text-xs text-muted">{t("billing.perMonthAt", { size: formatBytes(storage.total_bytes) })}</p>
        </div>
      </div>

      <section className="mt-4 rounded-card border border-border p-4">
        <div className="mb-3 flex items-center justify-between">
          <p className="text-xs uppercase tracking-wide text-muted">{t("billing.usage.title")}</p>
          <p className="text-xs text-muted">{t("billing.usage.price", { price: formatMicro(pricing.price_per_gb_month_micro, currency) })}</p>
        </div>
        <div className="space-y-2.5">
          {KINDS.map((k) => {
            const bytes = storage[k.key];
            if (bytes === 0) return null;
            const pct = Math.round((bytes / total) * 100);
            return (
              <div key={k.key}>
                <div className="mb-1 flex items-center gap-2 text-sm">
                  {k.icon}
                  <span className="font-medium">{t(k.label)}</span>
                  <span className="ml-auto tabular-nums text-muted">{formatBytes(bytes)}</span>
                  <span className="w-9 text-right text-xs tabular-nums text-muted">{t("billing.usage.share", { share: pct / 100 })}</span>
                </div>
                <div className="h-1.5 rounded-pill bg-bg-alt">
                  <div className={cn("h-full rounded-pill", k.bar)} style={{ width: `${pct}%` }} />
                </div>
              </div>
            );
          })}
        </div>
        {storage.unsized_objects > 0 && (
          <p className="mt-3 text-xs text-muted">{t("billing.usage.unsized", { count: storage.unsized_objects })}</p>
        )}
        {trash.length > 0 && (
          <p className="mt-3 border-t border-border pt-3 text-xs text-muted">
            {rich("billing.usage.trash", {
              size: formatBytes(trashBytes),
              count: trash.length,
              link: (c) => (
                <Link to="/trash" className="font-medium text-fg hover:underline">
                  {c}
                </Link>
              ),
            })}
          </p>
        )}
      </section>

      <section className="mt-8">
        <h2 className="mb-2 flex items-center gap-2 text-sm font-semibold uppercase tracking-wide text-muted">
          <Wallet className="h-4 w-4" /> {t("billing.topup.title")}
        </h2>
        {payments.enabled ? (
          <TopUp presets={payments.presets_micro} currency={payments.currency} min={payments.min_micro} max={payments.max_micro} />
        ) : (
          // Dormant until Stripe is configured — say so rather than showing a
          // button that can't do anything.
          <p className="text-sm text-muted">{t("billing.topup.unavailable")}</p>
        )}
      </section>

      {isAdmin && data.alerts && <AlertSettings initial={data.alerts} currency={currency} />}

      {data.entries.length > 0 && (
        <section className="mt-8">
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-muted">{t("billing.entries.title")}</h2>
          <div className="divide-y divide-border rounded-card border border-border">
            {data.entries.slice(0, 20).map((e) => (
              <div key={e.id} className="flex items-center gap-3 px-4 py-2.5">
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-sm">
                    {e.entry_type === "topup" ? t("billing.entries.topup") : e.note ?? t("billing.entries.storage")}
                  </span>
                  <span className="block text-xs text-muted">
                    {new Date(e.charge_date ?? e.created_at).toLocaleDateString(locale)}
                    {e.storage_bytes != null && ` · ${formatBytes(e.storage_bytes)}`}
                  </span>
                </span>
                <span className={cn("text-sm tabular-nums", e.amount_micro > 0 ? "text-success" : "text-muted")}>
                  {e.amount_micro > 0 ? "+" : ""}
                  {formatMicro(e.amount_micro, currency)}
                </span>
              </div>
            ))}
          </div>
        </section>
      )}

      {!isAdmin && (
        <p className="mt-6 text-xs text-muted">
          <Pill>{t("billing.readOnly")}</Pill> {t("billing.readOnlyNote")}
        </p>
      )}
    </Page>
  );
}
