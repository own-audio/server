// SPDX-License-Identifier: AGPL-3.0-or-later
import api from "./client";
import type { ContinueItem, PrivateItem, SearchResult } from "./types";

export async function getContinueListening(): Promise<ContinueItem[]> {
  const { data } = await api.get<ContinueItem[]>("/library/continue");
  return data;
}

export async function search(q: string, limit = 20): Promise<SearchResult[]> {
  const { data } = await api.get<SearchResult[]>("/library/search", {
    params: { q, limit },
  });
  return data;
}

/** Always the caller's own — the server scopes it to you, family admin or not. */
export async function listPrivateItems(): Promise<PrivateItem[]> {
  const { data } = await api.get<PrivateItem[]>("/library/private");
  return data;
}
