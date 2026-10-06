// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { LoginResponse } from "./types";

export interface SetupStatus {
  setup_complete: boolean;
  checks: {
    database: boolean;
    storage: boolean;
    storage_backend: string;
  };
}

export async function getSetupStatus(): Promise<SetupStatus> {
  const { data } = await api.get<SetupStatus>("/setup/status");
  return data;
}

export async function completeSetup(
  email: string,
  password: string,
  displayName: string,
): Promise<LoginResponse> {
  const { data } = await api.post<LoginResponse>("/setup/complete", {
    email,
    password,
    display_name: displayName,
  });
  return data;
}
