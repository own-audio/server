// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Ban, Copy, ImageUp, Link2, LogOut, Mail, Pencil, Plus, QrCode as QrIcon, SlidersHorizontal, Trash2, UserPlus, Users } from "lucide-react";
import {
  blockMember,
  createInvite,
  getFamily,
  listInvites,
  provisionMember,
  removeMember,
  revokeInvite,
  unblockMember,
  updateMember,
  updateFamily,
  uploadFamilyAvatar,
  leaveFamily,
  isFamilyAdmin,
  type Invite,
  type FamilyMember,
  type Family,
} from "../../api/family";
import { useNavigate } from "react-router-dom";
import { Page } from "../../components/shell/SplitView";
import { Button, Dialog, DialogContent, EmptyState, IconButton, Input, Pill, Select, Skeleton, toast } from "../../components/ui";
import QrCode from "../../components/QrCode";
import AudioAnalysisCard from "./AudioAnalysisCard";
import AuthImage from "../../components/AuthImage";
import { apiErrorMessage } from "../../lib/apiError";
import { cn } from "../../lib/cn";
import { useServerFeatures } from "../../lib/features";
import MemberAccessSheet from "./MemberAccessSheet";
import { t, useT, type PlainKey } from "../../i18n";

/** The server supplies a join URL when it knows its own base URL; otherwise
 *  build one from wherever this page is being served. */
function joinUrl(invite: Invite): string {
  return invite.join_url ?? `${window.location.origin}/join/${invite.code}`;
}

async function copy(text: string, copied: PlainKey) {
  try {
    await navigator.clipboard.writeText(text);
    toast.success(t(copied));
  } catch {
    toast.error(t("family.copy.failed"), t("family.copy.failedHint"));
  }
}

function expiresIn(iso: string): string {
  const ms = new Date(iso).getTime() - Date.now();
  if (!isFinite(ms)) return "";
  if (ms <= 0) return t("family.invite.expired");
  const mins = Math.round(ms / 60_000);
  if (mins < 60) return t("family.invite.expiresInMinutes", { count: mins });
  const hours = Math.round(mins / 60);
  if (hours < 48) return t("family.invite.expiresInHours", { count: hours });
  return t("family.invite.expiresInDays", { count: Math.round(hours / 24) });
}

const roleKey = (role: string): PlainKey => (isFamilyAdmin(role) ? "family.role.admin" : "family.role.member");

function InviteCard({ invite, onRevoke }: { invite: Invite; onRevoke: () => void }) {
  const { t } = useT();
  const [showQr, setShowQr] = useState(false);
  const url = joinUrl(invite);
  const usesLeft = invite.max_uses - invite.use_count;

  return (
    <div className="rounded-card border border-border p-3">
      <div className="flex flex-wrap items-start gap-2">
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            {invite.kind === "email" ? <Mail className="h-4 w-4 text-muted" /> : invite.kind === "claim" ? <UserPlus className="h-4 w-4 text-muted" /> : <Link2 className="h-4 w-4 text-muted" />}
            <span className="truncate text-sm font-medium">
              {invite.email ?? invite.label ?? t(invite.kind === "claim" ? "family.invite.setupCode" : "family.invite.anyoneWithLink")}
            </span>
          </span>
          <span className="mt-1 flex flex-wrap items-center gap-1.5 text-xs text-muted">
            <Pill>{t(roleKey(invite.role))}</Pill>
            <span>{t("family.invite.usesLeft", { left: usesLeft, max: invite.max_uses })}</span>
            <span>· {expiresIn(invite.expires_at)}</span>
          </span>
        </span>

        <span className="flex items-center gap-1">
          <IconButton size="sm" label={t("family.invite.showQr")} active={showQr} onClick={() => setShowQr((v) => !v)}>
            <QrIcon className="h-4 w-4" />
          </IconButton>
          <IconButton size="sm" label={t("family.invite.copyLink")} onClick={() => copy(url, "family.copy.linkCopied")}>
            <Copy className="h-4 w-4" />
          </IconButton>
          <IconButton size="sm" label={t("family.invite.revoke")} onClick={onRevoke}>
            <Trash2 className="h-4 w-4" />
          </IconButton>
        </span>
      </div>

      {showQr && (
        <div className="mt-3 flex flex-col items-center gap-2 border-t border-border pt-3">
          <QrCode value={url} size={168} className="rounded-lg" />
          <code className="select-all rounded-md bg-bg-alt px-2 py-1 text-xs">{invite.code}</code>
          <p className="max-w-xs text-center text-xs text-muted">
            {t("family.invite.qrNote", { role: invite.role })}
          </p>
        </div>
      )}
    </div>
  );
}

function InviteDialog({ onClose }: { onClose: () => void }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [tab, setTab] = useState<"email" | "link" | "provision">("email");
  const [email, setEmail] = useState("");
  const [label, setLabel] = useState("");
  const [maxUses, setMaxUses] = useState("1");
  const [displayName, setDisplayName] = useState("");
  const [loginEmail, setLoginEmail] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [created, setCreated] = useState<Invite | null>(null);

  const create = useMutation({
    mutationFn: async (): Promise<Invite> => {
      if (tab === "provision") {
        const result = await provisionMember({ display_name: displayName.trim(), login_email: loginEmail.trim() });
        return result.invite;
      }
      return createInvite(
        tab === "email"
          ? { kind: "email", email: email.trim() }
          : { kind: "link", label: label.trim() || undefined, max_uses: Math.max(1, Math.min(20, Number(maxUses) || 1)) }
      );
    },
    onSuccess: (invite) => {
      qc.invalidateQueries({ queryKey: ["invites"] });
      qc.invalidateQueries({ queryKey: ["family"] });
      setCreated(invite);
    },
    onError: (err) => setError(apiErrorMessage(err, t("family.inviteDialog.createFailed"))),
  });

  if (created) {
    const url = joinUrl(created);
    return (
      <Dialog open onOpenChange={(v) => !v && onClose()}>
        <DialogContent
          title={t("family.inviteCreated.title")}
          description={t(created.kind === "claim" ? "family.inviteCreated.claimDescription" : "family.inviteCreated.description")}
          footer={<Button onClick={onClose}>{t("common.action.done")}</Button>}
        >
          <div className="flex flex-col items-center gap-3">
            <QrCode value={url} size={200} className="rounded-lg" />
            <code className="select-all break-all rounded-md bg-bg-alt px-2 py-1 text-center text-xs">{url}</code>
            <div className="flex gap-2">
              <Button size="sm" variant="secondary" icon={<Copy className="h-4 w-4" />} onClick={() => copy(url, "family.copy.linkCopied")}>
                {t("family.invite.copyLink")}
              </Button>
              <Button size="sm" variant="secondary" icon={<Copy className="h-4 w-4" />} onClick={() => copy(created.code, "family.copy.codeCopied")}>
                {t("family.invite.copyCode")}
              </Button>
            </div>
            <p className="text-center text-xs text-muted">
              {t("family.inviteCreated.keyWarning", { role: created.role })}
            </p>
          </div>
        </DialogContent>
      </Dialog>
    );
  }

  const canSubmit =
    tab === "email" ? email.includes("@") : tab === "link" ? true : displayName.trim() !== "" && loginEmail.includes("@");

  return (
    <Dialog open onOpenChange={(v) => !v && onClose()}>
      <DialogContent
        title={t("family.inviteDialog.title")}
        footer={
          <>
            <Button variant="ghost" onClick={onClose}>
              {t("common.action.cancel")}
            </Button>
            <Button onClick={() => create.mutate()} loading={create.isPending} disabled={!canSubmit}>
              {t("family.inviteDialog.create")}
            </Button>
          </>
        }
      >
        <div className="mb-4 flex gap-1 rounded-pill bg-bg-alt p-0.5">
          {(["email", "link", "provision"] as const).map((k) => (
            <button
              key={k}
              onClick={() => {
                setTab(k);
                setError(null);
              }}
              className={cn(
                "flex-1 rounded-pill px-3 py-1.5 text-xs font-medium transition-colors",
                k === tab ? "bg-card text-fg shadow-card" : "text-muted hover:text-fg"
              )}
            >
              {t(k === "email" ? "family.inviteDialog.tab.email" : k === "link" ? "family.inviteDialog.tab.link" : "family.inviteDialog.tab.provision")}
            </button>
          ))}
        </div>

        {tab === "email" && (
          <Input
            label={t("family.inviteDialog.email")}
            type="email"
            autoFocus
            value={email}
            onChange={(e) => setEmail(e.target.value)}
            hint={t("family.inviteDialog.emailHint")}
          />
        )}

        {tab === "link" && (
          <div className="grid gap-3 sm:grid-cols-2">
            <Input
              label={t("family.inviteDialog.note")}
              value={label}
              onChange={(e) => setLabel(e.target.value)}
              placeholder={t("family.inviteDialog.notePlaceholder")}
              hint={t("family.inviteDialog.noteHint")}
            />
            <Select label={t("family.inviteDialog.maxUses")} value={maxUses} onChange={(e) => setMaxUses(e.target.value)}>
              {[1, 2, 3, 5, 10, 20].map((n) => (
                <option key={n} value={n}>
                  {t("family.inviteDialog.people", { count: n })}
                </option>
              ))}
            </Select>
          </div>
        )}

        {tab === "provision" && (
          <div className="grid gap-3">
            <p className="text-xs text-muted">{t("family.inviteDialog.provisionIntro")}</p>
            <Input
              label={t("family.inviteDialog.name")}
              autoFocus
              value={displayName}
              onChange={(e) => setDisplayName(e.target.value)}
              placeholder={t("family.inviteDialog.namePlaceholder")}
            />
            <Input
              label={t("family.inviteDialog.loginEmail")}
              value={loginEmail}
              onChange={(e) => setLoginEmail(e.target.value)}
              placeholder={t("family.inviteDialog.loginEmailPlaceholder")}
              hint={t("family.inviteDialog.loginEmailHint")}
            />
          </div>
        )}

        {error && <p role="alert" className="mt-3 rounded-[10px] bg-error/10 px-3 py-2 text-sm text-error">{error}</p>}
      </DialogContent>
    </Dialog>
  );
}

const BRACKET_LABEL: Record<string, PlainKey> = { teen: "family.bracket.teen", child: "family.bracket.child" };

function FamilyIdentity({ family, isAdmin }: { family: Family; isAdmin: boolean }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [editing, setEditing] = useState(false);
  const [name, setName] = useState(family.name);
  const [photo, setPhoto] = useState<File | null>(null);

  const save = useMutation({
    mutationFn: async () => {
      if (name.trim() && name.trim() !== family.name) await updateFamily({ name: name.trim() });
      if (photo) await uploadFamilyAvatar(photo);
    },
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["family"] });
      setPhoto(null);
      setEditing(false);
      toast.success(t("family.identity.updated"));
    },
    onError: (err) => toast.error(t("family.error.save"), apiErrorMessage(err, t("family.tryAgain"))),
  });

  if (!editing) {
    return (
      <div className="mb-6 flex items-center gap-3">
        {family.avatar_url ? (
          <AuthImage src={family.avatar_url} alt="" className="h-12 w-12 shrink-0 rounded-card object-cover" />
        ) : (
          <span className="flex h-12 w-12 shrink-0 items-center justify-center rounded-card bg-accent/12 text-accent">
            <Users className="h-5 w-5" />
          </span>
        )}
        <p className="min-w-0 flex-1 truncate text-sm text-muted">
          {t("family.identity.memberCount", { count: family.members.length })}
        </p>
        {isAdmin && (
          <IconButton size="sm" label={t("family.identity.edit")} onClick={() => setEditing(true)}>
            <Pencil className="h-4 w-4" />
          </IconButton>
        )}
      </div>
    );
  }

  return (
    <div className="mb-6 rounded-card border border-border p-4">
      <div className="grid gap-3 sm:grid-cols-2">
        <Input label={t("family.identity.name")} value={name} onChange={(e) => setName(e.target.value)} />
        <label className="block">
          <span className="mb-1.5 block text-[13px] font-medium text-fg">{t("family.identity.photo")}</span>
          <label className="flex h-10 cursor-pointer items-center gap-2 rounded-[10px] border border-border bg-card px-3 text-sm text-muted hover:text-fg">
            <ImageUp className="h-4 w-4" />
            <span className="truncate">{photo ? photo.name : t("family.identity.choosePhoto")}</span>
            <input type="file" accept="image/*" hidden onChange={(e) => setPhoto(e.target.files?.[0] ?? null)} />
          </label>
        </label>
      </div>
      <div className="mt-3 flex gap-2">
        <Button size="sm" loading={save.isPending} onClick={() => save.mutate()} disabled={!name.trim()}>
          {t("common.action.save")}
        </Button>
        <Button size="sm" variant="ghost" onClick={() => { setEditing(false); setName(family.name); setPhoto(null); }}>
          {t("common.action.cancel")}
        </Button>
      </div>
    </div>
  );
}

/** Leaving is the same call an admin uses to remove someone, aimed at
 *  yourself — and it says the same things, because the effect is the same. */
function LeaveFamily({ family }: { family: Family }) {
  const { t } = useT();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const [open, setOpen] = useState(false);

  const leave = useMutation({
    mutationFn: () => leaveFamily(family.my_user_id),
    onSuccess: () => {
      qc.invalidateQueries();
      toast.success(t("family.leave.left"));
      navigate("/", { replace: true });
    },
    onError: (err) => toast.error(t("family.leave.failed"), apiErrorMessage(err, t("family.leave.failedHint"))),
  });

  return (
    <>
      <Button size="sm" variant="ghost" className="mt-6 text-error" icon={<LogOut className="h-4 w-4" />} onClick={() => setOpen(true)}>
        {t("family.leave.button")}
      </Button>
      <Dialog open={open} onOpenChange={setOpen}>
        <DialogContent
          title={t("family.leave.title", { family: family.name })}
          description={t("family.leave.description")}
          footer={
            <>
              <Button variant="ghost" onClick={() => setOpen(false)}>{t("family.leave.stay")}</Button>
              <Button variant="danger" loading={leave.isPending} onClick={() => leave.mutate()}>{t("family.leave.confirm")}</Button>
            </>
          }
        >
          <p className="text-sm text-muted">{t("family.leave.detail")}</p>
        </DialogContent>
      </Dialog>
    </>
  );
}

function MemberRow({ member, isMe, isAdmin }: { member: FamilyMember; isMe: boolean; isAdmin: boolean }) {
  const { t } = useT();
  const qc = useQueryClient();
  const [confirmRemove, setConfirmRemove] = useState(false);
  const oneFamily = useServerFeatures().features.one_family;
  const [editingAccess, setEditingAccess] = useState(false);
  const invalidate = () => qc.invalidateQueries({ queryKey: ["family"] });

  const setRole = useMutation({
    mutationFn: (role: string) => updateMember(member.user_id, { role }),
    onSuccess: invalidate,
    onError: () => toast.error(t("family.member.roleFailed")),
  });
  const block = useMutation({
    mutationFn: () => (member.is_active ? blockMember(member.user_id) : unblockMember(member.user_id)),
    onSuccess: invalidate,
    onError: () => toast.error(t("family.member.actionFailed")),
  });
  const remove = useMutation({
    mutationFn: () => removeMember(member.user_id),
    onSuccess: () => {
      invalidate();
      toast.success(t("family.member.removed"), t("family.member.removedHint"));
    },
    onError: () => toast.error(t("family.member.removeFailed")),
  });

  return (
    <div className="flex flex-wrap items-center gap-3 px-4 py-3">
      {member.avatar_url ? (
        <AuthImage src={member.avatar_url} alt="" className="h-9 w-9 shrink-0 rounded-pill object-cover" />
      ) : (
        <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-pill bg-accent/12 text-sm font-semibold text-accent">
          {(member.display_label ?? member.display_name).slice(0, 1).toUpperCase()}
        </span>
      )}
      <span className="min-w-0 flex-1">
        <span className="flex flex-wrap items-center gap-1.5">
          <span className="truncate text-sm font-medium">{member.display_label ?? member.display_name}</span>
          {isMe && <Pill>{t("family.member.you")}</Pill>}
          {isFamilyAdmin(member.role) && <Pill tone="accent">{t("family.role.admin")}</Pill>}
          {member.pending && <Pill tone="warning">{t("family.member.noPassword")}</Pill>}
          {!member.is_active && <Pill tone="error">{t("family.member.blocked")}</Pill>}
          {BRACKET_LABEL[member.age_bracket] && <Pill>{t(BRACKET_LABEL[member.age_bracket])}</Pill>}
          {!member.can_upload && member.age_bracket !== "child" && <Pill>{t("family.member.cantAdd")}</Pill>}
        </span>
        <span className="block truncate text-xs text-muted">{member.email}</span>
      </span>

      {isAdmin && !isMe && (
        <span className="flex items-center gap-1">
          <Select
            aria-label={t("family.member.roleFor", { name: member.display_name })}
            value={member.role}
            onChange={(e) => setRole.mutate(e.target.value)}
            className="h-8 w-28 text-xs"
          >
            <option value="member">{t("family.role.member")}</option>
            <option value="family_admin">{t("family.role.admin")}</option>
          </Select>
          <IconButton size="sm" label={t("family.member.permissionsFor", { name: member.display_name })} onClick={() => setEditingAccess(true)}>
            <SlidersHorizontal className="h-4 w-4" />
          </IconButton>
          <IconButton size="sm" label={t(member.is_active ? "family.member.block" : "family.member.unblock")} onClick={() => block.mutate()}>
            <Ban className={cn("h-4 w-4", !member.is_active && "text-error")} />
          </IconButton>
          {/* With one family there is nowhere to remove someone to; only an
              unclaimed account can go (it is deleted). Block covers the rest. */}
          {(!oneFamily || member.pending) && (
            <IconButton size="sm" label={t("family.member.removeFromFamily")} onClick={() => setConfirmRemove(true)}>
              <Trash2 className="h-4 w-4" />
            </IconButton>
          )}
        </span>
      )}

      {editingAccess && <MemberAccessSheet member={member} onClose={() => setEditingAccess(false)} />}

      <Dialog open={confirmRemove} onOpenChange={setConfirmRemove}>
        <DialogContent
          title={t("family.member.removeTitle", { name: member.display_label ?? member.display_name })}
          description={t("family.member.removeDescription")}
          footer={
            <>
              <Button variant="ghost" onClick={() => setConfirmRemove(false)}>
                {t("common.action.cancel")}
              </Button>
              <Button variant="danger" loading={remove.isPending} onClick={() => remove.mutate()}>
                {t("common.action.remove")}
              </Button>
            </>
          }
        >
          <p className="text-sm text-muted">{t("family.member.blockInstead")}</p>
        </DialogContent>
      </Dialog>
    </div>
  );
}

export default function FamilyPage() {
  const { t } = useT();
  const oneFamily = useServerFeatures().features.one_family;
  const qc = useQueryClient();
  const [inviting, setInviting] = useState(false);

  const { data: family, isLoading } = useQuery({ queryKey: ["family"], queryFn: getFamily });
  const isAdmin = isFamilyAdmin(family?.my_role);
  const { data: invites = [] } = useQuery({ queryKey: ["invites"], queryFn: listInvites, enabled: isAdmin, retry: false });

  const revoke = useMutation({
    mutationFn: (id: string) => revokeInvite(id),
    onSuccess: () => {
      qc.invalidateQueries({ queryKey: ["invites"] });
      toast.success(t("family.invite.revoked"));
    },
    onError: () => toast.error(t("family.invite.revokeFailed")),
  });

  if (isLoading || !family) {
    return (
      <Page title={t("shell.nav.family")} width="max-w-3xl">
        <Skeleton className="h-40" />
      </Page>
    );
  }

  return (
    <Page
      title={family.name}
      actions={
        isAdmin && (
          <Button size="sm" icon={<Plus className="h-4 w-4" />} onClick={() => setInviting(true)}>
            {t("common.action.invite")}
          </Button>
        )
      }
      width="max-w-3xl"
    >
      <FamilyIdentity family={family} isAdmin={!!isAdmin} />

      <section>
        <h2 className="mb-3 flex items-center gap-2 text-sm font-semibold uppercase tracking-wide text-muted">
          <Users className="h-4 w-4" /> {t("family.members")}
        </h2>
        <div className="divide-y divide-border rounded-card border border-border">
          {family.members.map((m) => (
            <MemberRow key={m.user_id} member={m} isMe={m.user_id === family.my_user_id} isAdmin={!!isAdmin} />
          ))}
        </div>
      </section>

      <AudioAnalysisCard isAdmin={!!isAdmin} />

      {isAdmin && (
        <section className="mt-8">
          <h2 className="mb-3 text-sm font-semibold uppercase tracking-wide text-muted">{t("family.openInvites")}</h2>
          {invites.length === 0 ? (
            <EmptyState
              title={t("family.noInvites.title")}
              description={t("family.noInvites.description")}
              action={<Button variant="secondary" onClick={() => setInviting(true)}>{t("family.inviteDialog.title")}</Button>}
            />
          ) : (
            <div className="space-y-2">
              {invites.map((i) => (
                <InviteCard key={i.id} invite={i} onRevoke={() => revoke.mutate(i.id)} />
              ))}
            </div>
          )}
        </section>
      )}

      {!oneFamily && <LeaveFamily family={family} />}

      {inviting && <InviteDialog onClose={() => setInviting(false)} />}
    </Page>
  );
}
