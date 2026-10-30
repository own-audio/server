// SPDX-License-Identifier: AGPL-3.0-or-later
import { useCallback } from "react";
import { usePersistedState } from "./persistedState";

/** A column width the user can drag, kept per `key` across visits. */
export function useColumnWidth(key: string, initial: number, min: number, max: number) {
  const [stored, setStored] = usePersistedState<number>(key, initial);
  const clamp = useCallback((n: number) => Math.min(max, Math.max(min, Math.round(n))), [min, max]);
  return { width: clamp(stored), setWidth: (n: number) => setStored(clamp(n)), initial, min, max };
}

export type ColumnWidthState = ReturnType<typeof useColumnWidth>;
