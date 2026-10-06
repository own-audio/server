// SPDX-License-Identifier: AGPL-3.0-or-later
import axios, { AxiosError, type InternalAxiosRequestConfig } from "axios";
import { useAuthStore } from "../store/authStore";
import { loginPathFor } from "../lib/returnTo";
import { singleFlight } from "../lib/singleFlight";

// Same-origin by default: the console is served by the backend itself from
// /app/ui/dist. Cloudflare Pages builds set VITE_API_BASE_URL to the absolute
// API origin, which is baked in at build time — changing it needs a rebuild,
// not just a redeploy.
export const apiBaseUrl = import.meta.env.VITE_API_BASE_URL ?? "/api/v1";

const api = axios.create({ baseURL: apiBaseUrl });

/** Origin of the API, or "" when it is same-origin with the page. */
const apiOrigin = /^https?:\/\//.test(apiBaseUrl) ? new URL(apiBaseUrl).origin : "";

/**
 * Resolve a media URL the API handed us.
 *
 * Covers and podcast art come back **root-relative** (`/api/v1/…/cover`), and
 * a browser resolves those against the *page* origin — which is the API only
 * when the backend serves the console itself. The hosted build is a Cloudflare
 * Pages site with no API on its origin, so every cover fell through to the SPA
 * fallback, came back as `index.html` with a 200, and decoded as a broken
 * image. Point them at the API origin instead.
 *
 * URLs that are already absolute (podcast directory results point at whatever
 * host the show publishes its art on) are passed through untouched.
 */
export function mediaUrl(url: string | null | undefined): string | undefined {
  if (!url) return undefined;
  return url.startsWith("/") ? apiOrigin + url : url;
}

api.interceptors.request.use((config) => {
  const token = useAuthStore.getState().token;
  if (token) config.headers.Authorization = `Bearer ${token}`;
  return config;
});

/** Endpoints that establish a session; a 401 from these is the answer, not a
 *  reason to go looking for a fresher token. */
function isAuthEndpoint(url?: string): boolean {
  return !!url && /\/auth\/(login|register|refresh|google|apple)|\/join\/[^/]+\/claim/.test(url);
}

function signOut() {
  useAuthStore.getState().clearAuth();
  window.location.replace(loginPathFor(window.location));
}

/**
 * One refresh at a time, shared by every request that hits a 401 while it runs
 * — see `lib/singleFlight.ts` for why that is a correctness requirement here.
 */
const refreshAccessToken = singleFlight(async (): Promise<string> => {
  const stored = useAuthStore.getState().refreshToken;
  if (!stored) throw new Error("no refresh token");

  // A bare axios call: the instance's own interceptor would recurse here.
  const { data } = await axios.post<{ token: string; refresh_token: string }>(
    `${apiBaseUrl}/auth/refresh`,
    { refresh_token: stored }
  );
  useAuthStore.getState().setTokens(data.token, data.refresh_token);
  return data.token;
});

type Retriable = InternalAxiosRequestConfig & { _retried?: boolean };

api.interceptors.response.use(
  (r) => r,
  async (error: AxiosError) => {
    const status = error.response?.status;
    const original = error.config as Retriable | undefined;

    if (status !== 401 || !original || original._retried || isAuthEndpoint(original.url)) {
      // A 401 from the refresh endpoint itself is terminal — never retry it,
      // clear and start over.
      if (status === 401 && original && original.url?.includes("/auth/refresh")) signOut();
      return Promise.reject(error);
    }

    if (!useAuthStore.getState().refreshToken) {
      // Nothing to refresh with (an older session predating this, say).
      signOut();
      return Promise.reject(error);
    }

    original._retried = true;
    try {
      const token = await refreshAccessToken();
      original.headers.Authorization = `Bearer ${token}`;
      return api.request(original);
    } catch {
      signOut();
      return Promise.reject(error);
    }
  }
);

export default api;
