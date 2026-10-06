// SPDX-License-Identifier: AGPL-3.0-or-later
import { useMemo, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { BarChart3, Sparkles, Upload } from "lucide-react";
import { BookIcon, MusicIcon, PodcastIcon } from "../../components/ui/CloudIcon";
import {
  AGE_BRACKETS,
  getMemberAccess,
  permissionsFor,
  replaceMemberGrants,
  setMemberPolicy,
  updateMember,
  type AccessPolicy,
  type AgeBracket,
  type FamilyMember,
  type MediaKind,
} from "../../api/family";
import { getFamilyStats, setMemberStatsVisibility } from "../../api/stats";
import { listBooks } from "../../api/audiobooks";
import { listFeeds } from "../../api/podcasts";
import { listTracks } from "../../api/music";
import { Button, Dialog, DialogContent, SearchField, Skeleton, toast } from "../../components/ui";
import { apiErrorMessage } from "../../lib/apiError";
import { cn } from "../../lib/cn";
import { useT, type PlainKey } from "../../i18n";

const KINDS: { kind: MediaKind; label: PlainKey; icon: React.ReactNode }[] = [
  { kind: "audiobook", label: "common.kind.audiobooks", icon: <BookIcon className="h-4 w-4 text-book" /> },
  { kind: "podcast", label: "common.kind.podcasts", icon: <PodcastIcon className="h-4 w-4 text-podcast" /> },
  { kind: "music", label: "common.kind.music", icon: <MusicIcon className="h-4 w-4 text-music" /> },
];

function Toggle({
  checked,
  onChange,
  disabled,
  label,
  description,
  icon,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
  label: string;
  description: string;
  icon: React.ReactNode;
}) {
  return (
    <label className={cn("flex items-start gap-3 rounded-card border border-border p-3", disabled && "opacity-60")}>
      <input
        type="checkbox"
        checked={checked}
        disabled={disabled}
        onChange={(e) => onChange(e.target.checked)}
        className="mt-0.5 h-4 w-4 accent-[var(--accent)]"
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-center gap-1.5 text-sm font-medium">
          {icon}
          {label}
        </span>
        <span className="mt-0.5 block text-xs text-muted">{description}</span>
      </span>
    </label>
  );
}

const BRACKET_TEXT: Record<AgeBracket, { label: PlainKey; description: PlainKey }> = {
  adult: { label: "family.bracket.adult", description: "family.access.bracket.adult" },
  teen: { label: "family.bracket.teen", description: "family.access.bracket.teen" },
  child: { label: "family.bracket.child", description: "family.access.bracket.child" },
};

/** The per-kind default plus its item-level exceptions, for one media kind. */
function KindAccess({
  member,
  kind,
  label,
  icon,
  policy,
  deniedIds,
  onChanged,
}: {
  member: FamilyMember;
  kind: MediaKind;
  label: string;
  icon: React.ReactNode;
  policy: AccessPolicy;
  deniedIds: Set<string>;
  onChanged: () => void;
}) {
  const { t } = useT();
  const [filter, setFilter] = useState("");
  const [expanded, setExpanded] = useState(false);

  const books = useQuery({ queryKey: ["books"], queryFn: listBooks, enabled: expanded && kind === "audiobook" });
  const feeds = useQuery({ queryKey: ["feeds"], queryFn: listFeeds, enabled: expanded && kind === "podcast" });
  const tracks = useQuery({ queryKey: ["music-tracks"], queryFn: listTracks, enabled: expanded && kind === "music" });

  const items = useMemo(() => {
    const raw =
      kind === "audiobook"
        ? (books.data ?? []).map((b) => ({ id: b.id, title: b.title, sub: b.author }))
        : kind === "podcast"
          ? (feeds.data ?? []).map((f) => ({ id: f.id, title: f.title, sub: f.author }))
          : (tracks.data ?? []).map((t) => ({ id: t.id, title: t.title, sub: t.artist }));
    const q = filter.trim().toLowerCase();
    return q ? raw.filter((i) => i.title.toLowerCase().includes(q) || (i.sub ?? "").toLowerCase().includes(q)) : raw;
  }, [kind, books.data, feeds.data, tracks.data, filter]);

  const setPolicy = useMutation({
    mutationFn: (p: AccessPolicy) => setMemberPolicy(member.user_id, kind, p),
    onSuccess: onChanged,
    onError: () => toast.error(t("family.error.change")),
  });

  // Grants are a full replace per kind: send the whole exception list every time.
  const setDenied = useMutation({
    mutationFn: (ids: string[]) => replaceMemberGrants(member.user_id, kind, [], ids),
    onSuccess: onChanged,
    onError: () => toast.error(t("family.access.exceptionsFailed")),
  });

  const blocked = policy === "deny_all";

  return (
    <div className="rounded-card border border-border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <span className="flex items-center gap-2 text-sm font-medium">
          {icon}
          {label}
        </span>
        <div className="ml-auto flex gap-1 rounded-pill bg-bg-alt p-0.5">
          {(["allow_all", "deny_all"] as const).map((p) => (
            <button
              key={p}
              onClick={() => setPolicy.mutate(p)}
              className={cn(
                "rounded-pill px-2.5 py-1 text-xs font-medium transition-colors",
                p === policy ? "bg-card text-fg shadow-card" : "text-muted hover:text-fg"
              )}
            >
              {t(p === "allow_all" ? "family.access.allOfIt" : "family.access.noneOfIt")}
            </button>
          ))}
        </div>
      </div>

      <p className="mt-1.5 text-xs text-muted">
        {t(blocked ? "family.access.hiddenExcept" : "family.access.visibleExcept", {
          kind,
          name: member.display_label ?? member.display_name,
        })}
      </p>

      <button onClick={() => setExpanded((v) => !v)} className="mt-2 text-xs font-medium text-accent hover:underline">
        {expanded
          ? t("family.access.hideExceptions")
          : deniedIds.size > 0
            ? t("family.access.exceptionsCount", { count: deniedIds.size })
            : t("family.access.exceptions")}
      </button>

      {expanded && (
        <div className="mt-2 border-t border-border pt-2">
          <SearchField value={filter} onChange={(e) => setFilter(e.target.value)} placeholder={t("family.access.find", { kind })} className="w-full" />
          {books.isLoading || feeds.isLoading || tracks.isLoading ? (
            <Skeleton className="mt-2 h-24" />
          ) : (
            <ul className="mt-2 max-h-48 space-y-0.5 overflow-y-auto">
              {items.slice(0, 200).map((i) => {
                const listed = deniedIds.has(i.id);
                return (
                  <li key={i.id} className="flex items-center gap-2 rounded-lg px-2 py-1.5 hover:bg-bg-alt">
                    <input
                      type="checkbox"
                      checked={listed}
                      onChange={(e) => {
                        const next = new Set(deniedIds);
                        if (e.target.checked) next.add(i.id);
                        else next.delete(i.id);
                        setDenied.mutate([...next]);
                      }}
                      className="h-4 w-4 accent-[var(--accent)]"
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-sm">{i.title}</span>
                      {i.sub && <span className="block truncate text-xs text-muted">{i.sub}</span>}
                    </span>
                  </li>
                );
              })}
              {items.length === 0 && <li className="px-2 py-4 text-center text-xs text-muted">{t("family.access.nothingToList")}</li>}
            </ul>
          )}
          <p className="mt-2 text-xs text-muted">
            {t(blocked ? "family.access.tickedAllowed" : "family.access.tickedHidden")}
          </p>
        </div>
      )}
    </div>
  );
}

/**
 * What one family member is allowed to do and see.
 *
 * Two separate things, deliberately kept apart:
 *
 * - **What they can do** — age bracket, and whether they may add audio or make
 *   AI narrations. A `child` can never do either; the server refuses it and the
 *   database has a CHECK to match, so the toggles are shown off and disabled
 *   rather than pretending to be settable.
 * - **What they can hear** — a default per media kind, with per-item
 *   exceptions.
 *
 * Every control here is a server-side rule. Nothing on this screen is UX-only.
 */
export default function MemberAccessSheet({ member, onClose }: { member: FamilyMember; onClose: () => void }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [bracket, setBracket] = useState<AgeBracket>((member.age_bracket as AgeBracket) ?? "adult");
  const [canUpload, setCanUpload] = useState(member.can_upload);
  const [canGenerate, setCanGenerate] = useState(member.can_generate);
  const [error, setError] = useState<string | null>(null);

  const { data: access, isLoading } = useQuery({
    queryKey: ["member-access", member.user_id],
    queryFn: () => getMemberAccess(member.user_id),
  });

  const isChild = bracket === "child";

  /* The server allows this only for a member who already carries a `deny_all`
     policy — an account this admin manages. Mirroring that test here keeps a
     control that would 403 off the screen entirely. */
  const restricted = (access?.policies ?? []).some((p) => p.policy === "deny_all");

  /* There is no per-member "get visibility"; the family roll-up carries it as
     `hidden`, and it is admin-only, which is exactly who is on this screen. */
  const { data: familyStats } = useQuery({
    queryKey: ["family-stats"],
    queryFn: () => getFamilyStats(),
    enabled: restricted,
  });
  const statsShared = familyStats?.find((e) => e.user_id === member.user_id)?.hidden === false;

  const setVisibility = useMutation({
    mutationFn: (share: boolean) => setMemberStatsVisibility(member.user_id, share ? "family_admin" : "private"),
    onSuccess: () => qc.invalidateQueries({ queryKey: ["family-stats"] }),
    onError: (err) => toast.error(t("family.error.change"), apiErrorMessage(err, "")),
  });
  const effective = permissionsFor(bracket, { can_upload: canUpload, can_generate: canGenerate });

  const save = useMutation({
    mutationFn: () =>
      updateMember(member.user_id, {
        age_bracket: bracket,
        can_upload: effective.can_upload,
        can_generate: effective.can_generate,
      }),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["family"] });
      toast.success(t("family.access.saved"), t("family.access.savedDetail", { name: member.display_label ?? member.display_name }));
      onClose();
    },
    onError: (err) => setError(apiErrorMessage(err, t("family.access.saveFailed"))),
  });

  const policyFor = (kind: MediaKind): AccessPolicy =>
    (access?.policies.find((p) => p.media_kind === kind)?.policy as AccessPolicy) ?? "allow_all";

  const deniedFor = (kind: MediaKind) =>
    new Set(
      (access?.grants ?? [])
        .filter((g) => g.media_kind === kind && g.effect === (policyFor(kind) === "deny_all" ? "allow" : "deny"))
        .map((g) => g.item_id)
    );

  const refreshAccess = () => qc.invalidateQueries({ queryKey: ["member-access", member.user_id] });

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("family.access.title", { name: member.display_label ?? member.display_name })}
        description={t("family.access.description")}
        className="sm:max-w-2xl"
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.close")}
            </Button>
            <Button onClick={() => save.mutate()} loading={save.isPending}>
              {t("common.action.save")}
            </Button>
          </>
        }
      >
        <section>
          <p className="text-[13px] font-medium">{t("family.access.whoTheyAre")}</p>
          <div className="mt-2 grid gap-1.5 sm:grid-cols-3">
            {AGE_BRACKETS.map((b) => (
              <button
                key={b.value}
                onClick={() => setBracket(b.value)}
                className={cn(
                  "rounded-card border px-3 py-2 text-left transition-colors",
                  b.value === bracket ? "border-accent bg-accent/8" : "border-border hover:bg-bg-alt"
                )}
              >
                <span className="block text-sm font-medium">{t(BRACKET_TEXT[b.value].label)}</span>
                <span className="mt-0.5 block text-xs text-muted">{t(BRACKET_TEXT[b.value].description)}</span>
              </button>
            ))}
          </div>
          <p className="mt-2 text-xs text-muted">
            {t("family.access.bracketNote")}
          </p>
        </section>

        <section className="mt-5">
          <p className="text-[13px] font-medium">{t("family.access.whatTheyAdd")}</p>
          <div className="mt-2 grid gap-1.5 sm:grid-cols-2">
            <Toggle
              icon={<Upload className="h-4 w-4 text-muted" />}
              label={t("family.access.addAudio")}
              description={t(isChild ? "family.access.alwaysOffForChild" : "family.access.addAudioHint")}
              checked={effective.can_upload}
              disabled={isChild}
              onChange={setCanUpload}
            />
            <Toggle
              icon={<Sparkles className="h-4 w-4 text-muted" />}
              label={t("family.access.narrate")}
              description={t(isChild ? "family.access.alwaysOffForChild" : "family.access.narrateHint")}
              checked={effective.can_generate}
              disabled={isChild}
              onChange={setCanGenerate}
            />
          </div>
        </section>

        <section className="mt-5">
          <p className="text-[13px] font-medium">{t("family.access.whatTheyHear")}</p>
          <p className="mt-0.5 text-xs text-muted">{t("family.access.savedAsYouGo")}</p>
          {isLoading ? (
            <Skeleton className="mt-2 h-40" />
          ) : (
            <div className="mt-2 space-y-2">
              {KINDS.map((k) => (
                <KindAccess
                  key={k.kind}
                  member={member}
                  kind={k.kind}
                  label={t(k.label)}
                  icon={k.icon}
                  policy={policyFor(k.kind)}
                  deniedIds={deniedFor(k.kind)}
                  onChanged={refreshAccess}
                />
              ))}
            </div>
          )}
        </section>

        {restricted && (
          <section className="mt-5">
            <p className="text-[13px] font-medium">{t("family.access.theirListening")}</p>
            <div className="mt-2">
              <Toggle
                icon={<BarChart3 className="h-4 w-4 text-muted" />}
                label={t("family.access.showListening")}
                description={t("family.access.showListeningHint")}
                checked={statsShared ?? false}
                disabled={setVisibility.isPending || familyStats === undefined}
                onChange={(v) => setVisibility.mutate(v)}
              />
            </div>
          </section>
        )}

        {error && <p role="alert" className="mt-4 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}
