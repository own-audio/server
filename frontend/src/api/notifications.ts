// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { Notification } from "./types";

/* There is no push delivery server-side — clients poll this inbox. That is the
   design, not a stopgap, so no service worker is involved. */

export async function listNotifications(limit = 50): Promise<Notification[]> {
  const { data } = await api.get<Notification[]>("/devices/notifications", { params: { limit } });
  return data;
}

export async function ackNotifications(ids: string[]): Promise<void> {
  if (ids.length === 0) return;
  await api.post("/devices/notifications/ack", { ids });
}
