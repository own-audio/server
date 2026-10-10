// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { LoginResponse } from "./types";

export async function login(email: string, password: string): Promise<LoginResponse> {
  const { data } = await api.post<LoginResponse>("/auth/login", { email, password, device_kind: "web" });
  return data;
}

export async function register(
  email: string,
  password: string,
  displayName: string,
  inviteCode?: string
): Promise<LoginResponse> {
  const { data } = await api.post<LoginResponse>("/auth/register", {
    email,
    password,
    display_name: displayName,
    invite_code: inviteCode,
    device_kind: "web",
  });
  return data;
}

export interface AuthProviders {
  local: boolean;
  google: { enabled: boolean; desktop_client_id: string | null; web_client_id: string | null };
  apple: { enabled: boolean; web_client_id: string | null };
}

/**
 * Which sign-in methods this server offers. Best-effort at every call site: a
 * failure must never block the password form.
 */
export async function getAuthProviders(): Promise<AuthProviders> {
  const { data } = await api.get<AuthProviders>("/auth/providers");
  return data;
}

/**
 * Google Identity Services hands back an ID token; the backend verifies its
 * `aud` against AUTH__GOOGLE__CLIENT_IDS. The authorization-code form of this
 * endpoint is macOS-only — it requires a loopback redirect_uri, which a
 * browser cannot supply.
 */
/** Always resolves, whatever the email: the server never says whether it has an account. */
export async function forgotPassword(email: string): Promise<void> {
  await api.post("/auth/password/forgot", { email });
}

export async function resetPassword(token: string, password: string): Promise<void> {
  await api.post("/auth/password/reset", { token, password });
}

export async function verifyEmail(token: string): Promise<void> {
  await api.post("/auth/email/verify", { token });
}

export async function resendVerification(): Promise<{ sent: boolean; verified: boolean }> {
  const { data } = await api.post<{ sent: boolean; verified: boolean }>("/auth/email/resend");
  return data;
}

export async function loginWithGoogle(idToken: string): Promise<LoginResponse> {
  const { data } = await api.post<LoginResponse>("/auth/google", {
    id_token: idToken,
    device_kind: "web",
  });
  return data;
}

/** Sign in with Apple JS hands back an identity token; `fullName` is only
 *  present on the user's first-ever authorization (Apple's rule). */
export async function loginWithApple(identityToken: string, fullName?: string): Promise<LoginResponse> {
  const { data } = await api.post<LoginResponse>("/auth/apple", {
    identity_token: identityToken,
    full_name: fullName,
    device_kind: "web",
  });
  return data;
}

export async function checkRegistrationStatus(): Promise<{ registration_open: boolean }> {
  const { data } = await api.get<{ registration_open: boolean }>("/auth/registration-status");
  return data;
}

export async function changePassword(
  currentPassword: string,
  newPassword: string
): Promise<void> {
  await api.post("/auth/password", {
    current_password: currentPassword,
    new_password: newPassword,
  });
}

export async function logout(): Promise<void> {
  await api.post("/auth/logout");
}

export async function getMe(): Promise<LoginResponse["user"]> {
  const { data } = await api.get<LoginResponse["user"]>("/auth/me");
  return data;
}

// ── User self-management ──────────────────────────────────────────────────

export async function updateMyProfile(displayName: string): Promise<void> {
  await api.patch("/users/me", { display_name: displayName });
}

/**
 * Turn listening-derived recommendations on or off for yourself.
 *
 * Deliberately only for the signed-in user: an admin can deactivate an
 * account or change its role, but cannot decide for someone else that
 * their listening may be used to suggest things.
 */
export async function setRecommendationsEnabled(enabled: boolean): Promise<void> {
  await api.patch("/users/me", { recommendations_enabled: enabled });
}

export async function deleteMyAccount(): Promise<void> {
  await api.delete("/users/me");
}

// ── Subsonic API key ──────────────────────────────────────────────────────

export interface DeviceSession {
  chain_id: string;
  device_name: string | null;
  device_kind: string;
  signed_in_at: string;
  last_used_at: string | null;
  expires_at: string;
  /** The device making this request — never offer to revoke it from here. */
  current: boolean;
}

export async function listSessions(): Promise<DeviceSession[]> {
  const { data } = await api.get<DeviceSession[]>("/auth/sessions");
  return data;
}

/** Revoking a session signs that device out immediately. */
export async function revokeSession(chainId: string): Promise<void> {
  await api.delete(`/auth/sessions/${chainId}`);
}

export async function uploadMyAvatar(file: File): Promise<void> {
  const form = new FormData();
  form.append("avatar", file, file.name);
  await api.post("/users/me/avatar", form);
}

export interface SubsonicKey {
  username: string;
  api_key: string;
}

export async function getSubsonicKey(): Promise<SubsonicKey> {
  const { data } = await api.get<SubsonicKey>("/users/me/subsonic-key");
  return data;
}

export async function regenerateSubsonicKey(): Promise<SubsonicKey> {
  const { data } = await api.post<SubsonicKey>("/users/me/subsonic-key/regenerate");
  return data;
}

// ── Admin user management ─────────────────────────────────────────────────

export interface AdminUserInfo {
  id: string;
  email: string;
  display_name: string;
  role: string;
  is_active: boolean;
  created_at: string;
}

export async function adminListUsers(): Promise<AdminUserInfo[]> {
  const { data } = await api.get<AdminUserInfo[]>("/users");
  return data;
}

export async function adminUpdateUser(
  id: string,
  update: { display_name?: string; role?: string; is_active?: boolean }
): Promise<void> {
  await api.patch(`/users/${id}`, update);
}

export async function adminDeleteUser(id: string): Promise<void> {
  await api.delete(`/users/${id}`);
}

export async function adminRevokeSessions(id: string): Promise<void> {
  await api.post(`/users/${id}/revoke-sessions`);
}
