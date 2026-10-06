// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { UserInfo } from "../api/types";

interface AuthState {
  token: string | null;
  /** Long-lived, **single-use and rotating**: every refresh returns a new one,
   *  and replaying a spent token makes the server kill the whole device chain.
   *  That is why refreshes are single-flight in `api/client.ts`. */
  refreshToken: string | null;
  user: UserInfo | null;
  setAuth: (token: string, user: UserInfo, refreshToken?: string | null) => void;
  setTokens: (token: string, refreshToken: string) => void;
  clearAuth: () => void;
}

export const useAuthStore = create<AuthState>()(
  persist(
    (set, get) => ({
      token: null,
      refreshToken: null,
      user: null,
      // A caller updating only the profile shouldn't drop the refresh token,
      // so an omitted value keeps whatever is stored.
      setAuth: (token, user, refreshToken) =>
        set({ token, user, refreshToken: refreshToken === undefined ? get().refreshToken : refreshToken }),
      setTokens: (token, refreshToken) => set({ token, refreshToken }),
      clearAuth: () => set({ token: null, refreshToken: null, user: null }),
    }),
    { name: "audio2-auth" }
  )
);
