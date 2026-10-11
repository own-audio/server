// SPDX-License-Identifier: AGPL-3.0-or-later
import type { LoginResponse } from "./types";
import api from "./client";

/* The family surface. W6 builds the management UI on top of this; W5 needs
   `my_role` alone, to decide whether family stats exist for this viewer.

   Note `my_role` is the *family* role and has nothing to do with `users.role`,
   which is the instance admin. Gating an admin control on the wrong one hands
   family administration to the server owner.

   The admin value is `"family_admin"`, not `"admin"` — checking for `"admin"`
   silently hides every management control, which is what happened the first
   time this page was built. Use `isFamilyAdmin`. */

export type FamilyRole = "family_admin" | "member";

export function isFamilyAdmin(role: string | undefined): boolean {
  return role === "family_admin";
}

export interface FamilyMember {
  user_id: string;
  email: string;
  display_name: string;
  /** In-family label ("Dad"); falls back to display_name. */
  display_label: string | null;
  role: FamilyRole | string;
  is_active: boolean;
  /** A provisioned account nobody has claimed yet. */
  pending: boolean;
  avatar_url: string | null;
  joined_at: string;
  age_bracket: "adult" | "teen" | "child" | string;
  can_upload: boolean;
  can_generate: boolean;
}

export interface Family {
  id: string;
  name: string;
  my_role: FamilyRole | string;
  my_user_id: string;
  my_can_upload: boolean;
  my_can_generate: boolean;
  avatar_url: string | null;
  members: FamilyMember[];
}

export async function getFamily(): Promise<Family> {
  const { data } = await api.get<Family>("/family");
  return data;
}

// ── Invites ───────────────────────────────────────────────────────────────

/** `email` and `link` are created directly; `claim` only comes back from
 *  provisioning an account for someone with no mailbox. */
export type InviteKind = "email" | "link" | "claim";

export interface Invite {
  id: string;
  kind: InviteKind;
  email: string | null;
  /** A bearer token. Never log it, never put it anywhere it can leak. */
  code: string;
  role: string;
  label: string | null;
  max_uses: number;
  use_count: number;
  created_at: string;
  expires_at: string;
  /** Present when the server knows its own base URL; otherwise build it from
   *  the current origin. */
  join_url: string | null;
  claim_user_id?: string | null;
}

export async function listInvites(): Promise<Invite[]> {
  const { data } = await api.get<Invite[]>("/family/invites");
  return data;
}

export async function createInvite(req: {
  kind: "email" | "link";
  email?: string;
  role?: string;
  max_uses?: number;
  label?: string;
}): Promise<Invite> {
  const { data } = await api.post<Invite>("/family/invites", req);
  return data;
}

export async function revokeInvite(id: string): Promise<void> {
  await api.delete(`/family/invites/${id}`);
}

export async function acceptInvite(code: string): Promise<void> {
  await api.post("/family/invites/accept", { code });
}

export interface ProvisionResult {
  member: FamilyMember;
  invite: Invite;
}

/** Creates an account for someone with no mailbox; they set the password by
 *  redeeming the returned `claim` invite. No mail is ever sent to `login_email`. */
export async function provisionMember(req: {
  display_name: string;
  login_email: string;
  display_label?: string;
}): Promise<ProvisionResult> {
  const { data } = await api.post<ProvisionResult>("/family/members/provision", req);
  return data;
}

// ── Members ───────────────────────────────────────────────────────────────

export async function updateMember(
  userId: string,
  req: { role?: string; display_label?: string | null; age_bracket?: string; can_upload?: boolean; can_generate?: boolean }
): Promise<void> {
  await api.put(`/family/members/${userId}`, req);
}

/** Removing a member takes them out of the family; their account keeps working
 *  on its own, and anything they shared goes back to private. */
export async function removeMember(userId: string): Promise<void> {
  await api.delete(`/family/members/${userId}`);
}

/** Blocking signs them out everywhere and stops them signing back in, without
 *  removing them from the family. */
export async function blockMember(userId: string): Promise<void> {
  await api.post(`/family/members/${userId}/block`);
}

export async function unblockMember(userId: string): Promise<void> {
  await api.post(`/family/members/${userId}/unblock`);
}

export async function updateFamily(req: { name: string }): Promise<void> {
  await api.put("/family", req);
}

// ── Joining ───────────────────────────────────────────────────────────────

export interface JoinPreview {
  /** `valid` | `expired` | `exhausted`. An unknown code is a 404 instead. */
  status: "valid" | "expired" | "exhausted";
  kind: InviteKind | null;
  family_name: string | null;
  inviter_name: string | null;
  role: string | null;
  member_count: number | null;
  expires_at: string | null;
  uses_left: number | null;
  /** `claim` codes only: who this signs the claimant in as. */
  claim: { display_name: string; login_email: string } | null;
}

/** Public — works signed out. */
export async function previewJoin(code: string): Promise<JoinPreview> {
  const { data } = await api.get<JoinPreview>(`/join/${encodeURIComponent(code)}`);
  return data;
}

/** Public — sets the password on a provisioned account and signs in. */
export async function claimAccount(code: string, password: string) {
  const { data } = await api.post(`/join/${encodeURIComponent(code)}/claim`, {
    password,
    device_kind: "web",
  });
  return data as LoginResponse;
}

// ── Age brackets and permissions ──────────────────────────────────────────

/** `child` is a compliance invariant, not a preference: the server refuses to
 *  let a child upload or generate, and the database has a CHECK saying so. */
export type AgeBracket = "adult" | "teen" | "child";

export const AGE_BRACKETS: { value: AgeBracket; label: string; description: string }[] = [
  { value: "adult", label: "Adult", description: "Can do everything, including adding to the library." },
  { value: "teen", label: "Teen", description: "Listens, and adds only if you allow it below." },
  { value: "child", label: "Child", description: "Listens only. Adding audio and making narrations are always off." },
];

/** Moving someone to `child` clears both flags server-side; sending them true
 *  is refused rather than ignored. */
export function permissionsFor(bracket: AgeBracket, current: { can_upload: boolean; can_generate: boolean }) {
  return bracket === "child" ? { can_upload: false, can_generate: false } : current;
}

// ── Per-member content access ─────────────────────────────────────────────

export type MediaKind = "audiobook" | "podcast" | "music";
/** `allow_all` is the implicit default; `deny_all` hides that kind entirely. */
export type AccessPolicy = "allow_all" | "deny_all";

export interface MemberAccess {
  user_id: string;
  policies: { media_kind: MediaKind | string; policy: AccessPolicy | string }[];
  /** Item-level exceptions on top of the per-kind default. */
  grants: { media_kind: MediaKind | string; item_id: string; effect: "allow" | "deny" | string }[];
}

export async function getMemberAccess(userId: string): Promise<MemberAccess> {
  const { data } = await api.get<MemberAccess>(`/family/members/${userId}/access`);
  return data;
}

export async function setMemberPolicy(userId: string, media_kind: MediaKind, policy: AccessPolicy): Promise<void> {
  await api.put(`/family/members/${userId}/policy`, { media_kind, policy });
}

/** A **full replace** of that kind's exceptions, not an incremental add —
 *  whatever isn't in these lists stops being an exception. */
export async function replaceMemberGrants(
  userId: string,
  media_kind: MediaKind,
  allow: string[],
  deny: string[]
): Promise<void> {
  await api.put(`/family/members/${userId}/grants`, { media_kind, allow, deny });
}

// ── Family identity ───────────────────────────────────────────────────────

export async function uploadFamilyAvatar(file: File): Promise<void> {
  const form = new FormData();
  form.append("avatar", file, file.name);
  await api.post("/family/avatar", form);
}

export async function deleteFamilyAvatar(): Promise<void> {
  await api.delete("/family/avatar");
}

/** Leaving is `DELETE /family/members/{me}` — the same call an admin uses to
 *  remove someone else, aimed at yourself. */
export async function leaveFamily(myUserId: string): Promise<void> {
  await api.delete(`/family/members/${myUserId}`);
}

// ── Who can hear this ─────────────────────────────────────────────────────

export interface AudienceEntry {
  user_id: string;
  display_name: string;
  display_label: string | null;
  can_listen: boolean;
  /** Access that cannot be taken away — the item's owner, or a family admin.
      Show the state, not a control that would do nothing. */
  locked: boolean;
}

/** Family admins only. Answers "who in the family can actually play this?",
 *  which is visibility and per-member access resolved together. */
export async function contentAudience(kind: MediaKind, itemId: string): Promise<AudienceEntry[]> {
  const { data } = await api.get<AudienceEntry[]>(`/family/content/${kind}/${itemId}/audience`);
  return data;
}

/**
 * Say who may hear one item. `canListen` is the whole audience, not a change to it.
 *
 * Answers with the audience as the server resolves it, which is not always what was sent:
 * owners and family admins keep access whatever the list says.
 */
/**
 * Ask the family's admins to change what `setContentAudience` controls, without being able to
 * set it yourself. For a member the item is currently hidden from — the item still has to be
 * shared with their family, or the server answers 404 the same as it would for a nonexistent one.
 */
export async function requestContentAccess(kind: MediaKind, itemId: string): Promise<void> {
  await api.post(`/family/content/${kind}/${itemId}/request-access`);
}

/**
 * Grant one more person access without disturbing anyone else's — the shape a "Grant" button on
 * an access-request notification needs, over an endpoint whose contract is "send the whole
 * audience". Reads the current one first and adds the requester to it; a second click (or two
 * admins clicking at once) is idempotent, not a double-grant.
 */
export async function grantContentAccess(
  kind: MediaKind,
  itemId: string,
  requesterId: string
): Promise<AudienceEntry[]> {
  const current = await contentAudience(kind, itemId);
  const next = current
    .filter((m) => !m.locked && (m.can_listen || m.user_id === requesterId))
    .map((m) => m.user_id);
  return setContentAudience(kind, itemId, next);
}

export async function setContentAudience(
  kind: MediaKind,
  itemId: string,
  canListen: string[]
): Promise<AudienceEntry[]> {
  const { data } = await api.put<AudienceEntry[]>(
    `/family/content/${kind}/${itemId}/audience`,
    { can_listen: canListen }
  );
  return data;
}

// ── Library audio analysis opt-in ──────────────────────────────────────────

export interface AudioAnalysisStatus {
  enabled: boolean;
  enabled_at: string | null;
  /** Tracks measured at the current extractor version. */
  measured: number;
  /** Tracks in this family's library worth measuring. */
  total: number;
  /** True while there is queued work — what a progress bar is for. */
  running: boolean;
}

export async function getAudioAnalysis(): Promise<AudioAnalysisStatus> {
  const { data } = await api.get<AudioAnalysisStatus>("/family/settings/audio-analysis");
  return data;
}

/**
 * Family admins only, and a family-wide setting rather than a personal one:
 * tempo and loudness are properties of the recording, so one member enabling
 * it measures the shared library once for everyone.
 *
 * Disabling stops new work and **keeps** every measurement already taken —
 * deleting them would mean fetching the whole library again if someone changed
 * their mind. Say that in the UI rather than implying "off" means "erased".
 */
export async function setAudioAnalysis(enabled: boolean): Promise<AudioAnalysisStatus> {
  const { data } = await api.put<AudioAnalysisStatus>("/family/settings/audio-analysis", {
    enabled,
  });
  return data;
}
