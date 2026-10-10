// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState, type FormEvent } from "react";
import { useNavigate } from "react-router-dom";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Copy, ImageUp, Laptop, LogOut, Monitor, RefreshCw, Smartphone, Sparkles, Tv } from "lucide-react";
import {
  changePassword,
  deleteMyAccount,
  getSubsonicKey,
  listSessions,
  logout,
  regenerateSubsonicKey,
  revokeSession,
  setRecommendationsEnabled,
  updateMyProfile,
  uploadMyAvatar,
  type DeviceSession,
} from "../../api/auth";
import { clearTrackFeedback, listTrackFeedback, listTracks } from "../../api/music";
import { useAuthStore } from "../../store/authStore";
import { Page } from "../../components/shell/SplitView";
import { Button, Dialog, DialogContent, IconButton, Input, Pill, SegmentedControl, Skeleton, toast } from "../../components/ui";
import { useI18n, useT, type LocalePreference } from "../../i18n";
import { apiErrorMessage } from "../../lib/apiError";
import { timeAgo } from "../../lib/format";
import { cn } from "../../lib/cn";
import { clearDownloads } from "../../lib/offline/downloads";

function Section({ title, description, children }: { title: string; description?: string; children: React.ReactNode }) {
  return (
    <section className="mt-8 first:mt-0">
      <h2 className="text-sm font-semibold uppercase tracking-wide text-muted">{title}</h2>
      {description && <p className="mt-1 text-xs text-muted">{description}</p>}
      <div className="mt-3">{children}</div>
    </section>
  );
}

/**
 * The recommendations switch.
 *
 * Off for every account until the person here turns it on. The copy says
 * where the work happens because that is the honest answer and it is
 * also the reassuring one: the matching runs in this browser, against
 * a public catalogue, and the listening history never goes anywhere to
 * produce a suggestion.
 *
 * No confirmation dialog and no celebration on either edge — it is a
 * preference, and both directions are equally fine to pick.
 */
function Recommendations() {
  const { t } = useT();
  const { user, setAuth, token } = useAuthStore();
  const enabled = user?.recommendations_enabled ?? false;

  const save = useMutation({
    mutationFn: setRecommendationsEnabled,
    onSuccess: (_d, next) => {
      if (user && token) setAuth(token, { ...user, recommendations_enabled: next });
    },
    onError: (err) => toast.error(t("settings.error.change"), apiErrorMessage(err, t("settings.error.tryAgain"))),
  });

  return (
    <label className="flex items-start gap-3 rounded-card border border-border p-4">
      <input
        type="checkbox"
        checked={enabled}
        disabled={save.isPending}
        onChange={(e) => save.mutate(e.target.checked)}
        className="mt-0.5 h-4 w-4 accent-[var(--accent)]"
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5 text-sm font-medium">
          <Sparkles className="h-4 w-4" />
          {t("settings.recommendations.label")}
        </span>
        <span className="mt-1 block text-xs text-muted">{t("settings.recommendations.description")}</span>
      </span>
    </label>
  );
}

function Profile() {
  const { t } = useT();
  const { user, setAuth, token } = useAuthStore();
  const [name, setName] = useState(user?.display_name ?? "");
  const [avatar, setAvatar] = useState<File | null>(null);

  const save = useMutation({
    mutationFn: async () => {
      await updateMyProfile(name.trim());
      if (avatar) await uploadMyAvatar(avatar);
    },
    onSuccess: () => {
      if (user && token) setAuth(token, { ...user, display_name: name.trim() });
      setAvatar(null);
      toast.success(t("settings.profile.saved"));
    },
    onError: (err) => toast.error(t("settings.profile.saveError"), apiErrorMessage(err, t("settings.error.tryAgain"))),
  });

  return (
    <div className="rounded-card border border-border p-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <Input label={t("settings.profile.name")} value={name} onChange={(e) => setName(e.target.value)} />
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-fg">{t("settings.profile.photo")}</span>
          <label className="flex h-10 cursor-pointer items-center gap-2 rounded-[10px] border border-border bg-card px-3 text-sm text-muted hover:text-fg">
            <ImageUp className="h-4 w-4" />
            <span className="truncate">{avatar ? avatar.name : t("settings.profile.chooseImage")}</span>
            <input type="file" accept="image/*" hidden onChange={(e) => setAvatar(e.target.files?.[0] ?? null)} />
          </label>
        </label>
      </div>
      <p className="mt-3 text-xs text-muted">{t("settings.profile.signedInAs", { email: user?.email ?? "" })}</p>
      <Button size="sm" className="mt-3" loading={save.isPending} onClick={() => save.mutate()}>
        {t("common.action.save")}
      </Button>
    </div>
  );
}

function Password() {
  const { t } = useT();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);

  const change = useMutation({
    mutationFn: () => changePassword(current, next),
    onSuccess: () => {
      setCurrent("");
      setNext("");
      setConfirm("");
      setError(null);
      toast.success(t("settings.password.changed"), t("settings.password.changedDetail"));
    },
    onError: (err) => setError(apiErrorMessage(err, t("settings.password.error"))),
  });

  function submit(e: FormEvent) {
    e.preventDefault();
    setError(null);
    if (next.length < 12) return setError(t("settings.password.tooShort"));
    if (next !== confirm) return setError(t("settings.password.mismatch"));
    change.mutate();
  }

  return (
    <form onSubmit={submit} className="rounded-card border border-border p-4" noValidate>
      <div className="grid gap-3 sm:grid-cols-3">
        <Input label={t("settings.password.current")} type="password" autoComplete="current-password" value={current} onChange={(e) => setCurrent(e.target.value)} />
        <Input label={t("settings.password.new")} type="password" autoComplete="new-password" value={next} onChange={(e) => setNext(e.target.value)} />
        <Input label={t("settings.password.confirm")} type="password" autoComplete="new-password" value={confirm} onChange={(e) => setConfirm(e.target.value)} />
      </div>
      {/* The server drops every other session on a password change; saying so
          after the fact would look like a bug. */}
      <p className="mt-3 text-xs text-muted">{t("settings.password.hint")}</p>
      {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      <Button size="sm" type="submit" className="mt-3" loading={change.isPending} disabled={!current || !next}>
        {t("settings.password.change")}
      </Button>
    </form>
  );
}

function deviceIcon(kind: string) {
  if (kind === "ios" || kind === "android") return <Smartphone className="h-4 w-4" />;
  if (kind === "macos" || kind === "windows") return <Laptop className="h-4 w-4" />;
  if (kind === "tvos") return <Tv className="h-4 w-4" />;
  return <Monitor className="h-4 w-4" />;
}

function Devices() {
  const { t } = useT();
  const qc = useQueryClient();
  const { data: sessions = [], isLoading } = useQuery({ queryKey: ["sessions"], queryFn: listSessions });

  const revoke = useMutation({
    mutationFn: (chainId: string) => revokeSession(chainId),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["sessions"] });
      toast.success(t("settings.devices.signedOut"));
    },
    onError: () => toast.error(t("settings.devices.signOutError")),
  });

  if (isLoading) return <Skeleton className="h-32" />;

  // This device first, so the one row that can't be revoked is where it's expected.
  const ordered = [...sessions].sort((a, b) => Number(b.current) - Number(a.current));

  return (
    <div className="divide-y divide-border rounded-card border border-border">
      {ordered.map((s: DeviceSession) => (
        <div key={s.chain_id} className="flex items-center gap-3 px-4 py-3">
          <span className="text-muted">{deviceIcon(s.device_kind)}</span>
          <span className="min-w-0 flex-1">
            <span className="flex items-center gap-1.5">
              <span className="truncate text-sm font-medium">{s.device_name ?? s.device_kind}</span>
              {s.current && <Pill tone="accent">{t("settings.devices.thisDevice")}</Pill>}
            </span>
            <span className="block text-xs text-muted">
              {s.last_used_at
                ? t("settings.devices.signedInLastUsed", { when: timeAgo(s.signed_in_at), lastUsed: timeAgo(s.last_used_at) })
                : t("settings.devices.signedIn", { when: timeAgo(s.signed_in_at) })}
            </span>
          </span>
          {/* Never offer to revoke the session making the request — that is
              "sign out", and it lives in the account menu. */}
          {!s.current && (
            <IconButton size="sm" label={t("settings.devices.signOut", { device: s.device_name ?? s.device_kind })} onClick={() => revoke.mutate(s.chain_id)}>
              <LogOut className="h-4 w-4" />
            </IconButton>
          )}
        </div>
      ))}
      {sessions.length === 0 && <p className="px-4 py-6 text-center text-sm text-muted">{t("settings.devices.none")}</p>}
    </div>
  );
}

function SubsonicKeySection() {
  const { t, rich } = useT();
  const qc = useQueryClient();
  const [revealed, setRevealed] = useState(false);
  const { data, isLoading } = useQuery({ queryKey: ["subsonic-key"], queryFn: getSubsonicKey, retry: false });

  const regenerate = useMutation({
    mutationFn: regenerateSubsonicKey,
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["subsonic-key"] });
      toast.success(t("settings.subsonic.regenerated"), t("settings.subsonic.regeneratedDetail"));
    },
    onError: () => toast.error(t("settings.subsonic.regenerateError")),
  });

  if (isLoading) return <Skeleton className="h-24" />;

  const key = data?.api_key ?? "";

  return (
    <div className="rounded-card border border-border p-4">
      <p className="text-sm">{t("settings.subsonic.intro")}</p>
      {data?.username && (
        <p className="mt-1 text-xs text-muted">
          {rich("settings.subsonic.username", { username: data.username, code: (c) => <code className="select-all">{c}</code> })}
        </p>
      )}
      <div className="mt-3 flex flex-wrap items-center gap-2">
        <code className={cn("select-all rounded-md bg-bg-alt px-2 py-1.5 text-xs", !revealed && "blur-sm")}>
          {key || "—"}
        </code>
        <Button size="sm" variant="ghost" onClick={() => setRevealed((v) => !v)}>
          {revealed ? t("settings.subsonic.hide") : t("settings.subsonic.show")}
        </Button>
        <Button
          size="sm"
          variant="secondary"
          icon={<Copy className="h-4 w-4" />}
          onClick={() => {
            void navigator.clipboard.writeText(key).then(
              () => toast.success(t("settings.subsonic.copied")),
              () => toast.error(t("settings.subsonic.copyError"))
            );
          }}
        >
          {t("settings.subsonic.copy")}
        </Button>
        <Button size="sm" variant="ghost" icon={<RefreshCw className="h-4 w-4" />} loading={regenerate.isPending} onClick={() => regenerate.mutate()}>
          {t("settings.subsonic.regenerate")}
        </Button>
      </div>
    </div>
  );
}

/**
 * The undo list for "never play this again".
 *
 * This section is not optional politeness. Nothing about a banned track looks
 * different anywhere in the library — that is the whole point, since banning
 * is a preference rather than a permission and must not change what anyone
 * else sees — so without a list here a mis-click is unrecoverable.
 *
 * Dislikes are shown too, but quietly: they fade on their own, so there is
 * rarely a reason to act on one.
 */
function MutedTracks() {
  const { t } = useT();
  const qc = useQueryClient();
  const { data: feedback, isLoading } = useQuery({
    queryKey: ["track-feedback"],
    queryFn: () => listTrackFeedback(),
  });
  const { data: tracks } = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks });

  const undo = useMutation({
    mutationFn: (trackId: string) => clearTrackFeedback(trackId),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["track-feedback"] }),
    onError: () => toast.error(t("settings.muted.undoError"), t("settings.error.tryAgain")),
  });

  if (isLoading) return <Skeleton className="h-16" />;
  if (!feedback || feedback.length === 0) {
    return (
      <p className="text-sm text-muted">{t("settings.muted.empty")}</p>
    );
  }

  const titleOf = (id: string) => {
    const track = tracks?.find((x) => x.id === id);
    if (!track) return t("settings.muted.unknownTrack");
    return track.artist ? t("settings.muted.artistTitle", { artist: track.artist, title: track.title }) : track.title;
  };

  return (
    <div className="divide-y divide-border rounded-card border border-border">
      {feedback.map((f) => (
        <div key={f.track_id} className="flex items-center justify-between gap-3 px-3 py-2">
          <div className="min-w-0">
            <p className="truncate text-sm">{titleOf(f.track_id)}</p>
            <p className="text-xs text-muted">
              {f.kind === "banned" ? t("settings.muted.banned") : t("settings.muted.disliked")}
            </p>
          </div>
          <Button size="sm" variant="ghost" disabled={undo.isPending} onClick={() => undo.mutate(f.track_id)}>
            {t("settings.muted.undo")}
          </Button>
        </div>
      ))}
    </div>
  );
}

function DeleteAccount() {
  const { t } = useT();
  const navigate = useNavigate();
  const clearAuth = useAuthStore((s) => s.clearAuth);
  const [open, setOpen] = useState(false);
  const [typed, setTyped] = useState("");
  const PHRASE = t("settings.delete.phrase");

  const remove = useMutation({
    mutationFn: deleteMyAccount,
    onSuccess: async () => {
      try {
        await logout();
      } catch {
        // the account is gone; the session went with it
      }
      // Someone else may sign in on this device next.
      void clearDownloads();
      clearAuth();
      navigate("/auth/login", { replace: true });
    },
    onError: () => toast.error(t("settings.delete.error")),
  });

  return (
    <>
      <div className="rounded-card border border-error/40 p-4">
        <p className="text-sm font-medium">{t("settings.delete.title")}</p>
        <p className="mt-0.5 text-xs text-muted">{t("settings.delete.description")}</p>
        <Button size="sm" variant="danger" className="mt-3" onClick={() => setOpen(true)}>
          {t("settings.delete.button")}
        </Button>
      </div>

      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent
          title={t("settings.delete.confirmTitle")}
          description={t("settings.delete.confirmDescription")}
          footer={
            <>
              <Button variant="ghost" onClick={() => setOpen(false)}>
                {t("settings.delete.keep")}
              </Button>
              <Button variant="danger" disabled={typed !== PHRASE} loading={remove.isPending} onClick={() => remove.mutate()}>
                {t("settings.delete.confirm")}
              </Button>
            </>
          }
        >
          <Input
            label={t("settings.delete.typeToConfirm", { phrase: PHRASE })}
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            autoComplete="off"
            autoFocus
          />
        </DialogContent>
      </Dialog>
    </>
  );
}

/* The choice is stored per browser, like the theme: the same account may be
   read in Czech on one device and English on another. */
function Language() {
  const { t } = useT();
  const preference = useI18n((s) => s.preference);
  const setPreference = useI18n((s) => s.setPreference);
  return (
    <SegmentedControl<LocalePreference>
      value={preference}
      onChange={setPreference}
      segments={[
        { value: "system", label: t("common.language.system") },
        { value: "en", label: t("common.language.en") },
        { value: "cs", label: t("common.language.cs") },
      ]}
    />
  );
}

export default function SettingsPage() {
  const { t } = useT();
  return (
    <Page title={t("settings.page.title")} width="max-w-3xl">
      <Section title={t("settings.section.profile")}>
        <Profile />
      </Section>
      <Section title={t("common.language.title")} description={t("settings.language.description")}>
        <Language />
      </Section>
      <Section title={t("settings.section.recommendations")}>
        <Recommendations />
      </Section>
      <Section title={t("settings.section.password")}>
        <Password />
      </Section>
      <Section title={t("settings.section.devices")} description={t("settings.section.devicesDescription")}>
        <Devices />
      </Section>
      <Section title={t("settings.section.muted")} description={t("settings.section.mutedDescription")}>
        <MutedTracks />
      </Section>
      <Section title={t("settings.section.subsonic")}>
        <SubsonicKeySection />
      </Section>
      <Section title={t("settings.section.danger")}>
        <DeleteAccount />
      </Section>
    </Page>
  );
}
