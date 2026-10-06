// SPDX-License-Identifier: AGPL-3.0-or-later
import { approveDeviceRequest, pollDeviceRequest, startDeviceRequest } from "../../api/deviceAuth";
import { useAuthStore } from "../../store/authStore";

/*
 * Signing in a playlist's Home Screen icon without asking again.
 *
 * On an iPhone the icon gets its own storage, so it starts signed out. This
 * reuses the TV pairing flow with both ends on the same phone: the signed-in
 * app asks for a pairing code and approves it itself, the code travels in the
 * link the icon is made from, and the icon trades it for a session of its own
 * the first time it opens. The code works once and only for ten minutes; the
 * icon then shows up as its own device in the account, and can be signed out
 * like any other.
 */

export async function createIconHandoff(playlistName: string): Promise<string> {
  const { device_code, user_code } = await startDeviceRequest(`${playlistName} (Home Screen)`.slice(0, 80));
  await approveDeviceRequest(user_code);
  return device_code;
}

/** True when the icon is now signed in. */
export async function claimIconHandoff(deviceCode: string): Promise<boolean> {
  try {
    const res = await pollDeviceRequest(deviceCode);
    if (!("token" in res)) return false;
    useAuthStore.getState().setAuth(res.token, res.user, res.refresh_token);
    return true;
  } catch {
    return false;
  }
}

/** The link an icon is made from: the playlist, its name for the icon, the
 *  one-time code that signs the icon in, and the icon itself. Safari isn't
 *  signed in and can't fetch the cover, so the picture travels in the link
 *  (a 180 px JPEG, a few KB) and the server puts it in the page before Safari
 *  reads it. */
export function iconLink(playlistId: string, name: string, code: string, icon?: string | null): string {
  const q = new URLSearchParams({ n: name, k: code });
  const i = iconParam(icon);
  if (i) q.set("i", i);
  return `${window.location.origin}/play/${playlistId}?${q}`;
}

/** A JPEG or WebP data URL as the base64url text that travels in a link.
 *  Anything else (Safari hands back PNG when asked for WebP) is too big to. */
export function iconParam(icon: string | null | undefined): string | null {
  const m = icon?.match(/^data:image\/(?:jpeg|webp);base64,(.+)$/);
  const b64 = m?.[1] ?? null;
  return b64 ? b64.replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "") : null;
}

/** The `i` parameter back into something an <img> can show. */
export function iconFromParam(param: string | null): string | null {
  if (!param || !/^[A-Za-z0-9_-]+$/.test(param)) return null;
  const b64 = param.replace(/-/g, "+").replace(/_/g, "/");
  return `data:image/jpeg;base64,${b64}${"=".repeat((4 - (b64.length % 4)) % 4)}`;
}
