// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";

/**
 * Letting a keyboardless device in: the TV shows a short code, someone signed
 * in here says yes. The device polls meanwhile and gets its own tokens — none
 * of this hands the browser's session to it.
 */
export interface DeviceRequestInfo {
  user_code: string;
  device_name: string | null;
  device_kind: string;
  expires_at: string;
}

/** Only pending, unexpired requests resolve; anything else is a 404. */
export async function describeDeviceRequest(userCode: string): Promise<DeviceRequestInfo> {
  const { data } = await api.get<DeviceRequestInfo>(`/auth/device/${encodeURIComponent(userCode)}`);
  return data;
}

export async function approveDeviceRequest(userCode: string): Promise<void> {
  await api.post(`/auth/device/${encodeURIComponent(userCode)}/approve`);
}

export async function denyDeviceRequest(userCode: string): Promise<void> {
  await api.post(`/auth/device/${encodeURIComponent(userCode)}/deny`);
}

/**
 * Display only: upper-cased and split into two blocks, the way the TV shows it.
 *
 * The folding of look-alike characters (O for 0, I and L for 1, S for 5) is
 * deliberately *not* repeated here — the server does it on lookup, and a second
 * copy of that rule in another language is one that can drift out of step and
 * turn a correctly-read code into "no such code".
 */
export function formatDeviceCode(input: string): string {
  const cleaned = input.replace(/[^a-zA-Z0-9]/g, "").toUpperCase().slice(0, 8);
  return cleaned.length > 4 ? `${cleaned.slice(0, 4)}-${cleaned.slice(4)}` : cleaned;
}

interface DeviceStart {
  device_code: string;
  user_code: string;
}

type DevicePoll =
  | { token: string; refresh_token: string; user: import("./types").UserInfo }
  | { status: "authorization_pending" | "slow_down" | "denied" | "expired" };

export async function startDeviceRequest(deviceName: string): Promise<DeviceStart> {
  const { data } = await api.post<DeviceStart>("/auth/device/start", { device_name: deviceName, device_kind: "web" });
  return data;
}

export async function pollDeviceRequest(deviceCode: string): Promise<DevicePoll> {
  const { data } = await api.post<DevicePoll>("/auth/device/poll", { device_code: deviceCode });
  return data;
}
