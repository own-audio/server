// SPDX-License-Identifier: AGPL-3.0-or-later
import { useState } from "react";

/** A small per-viewer preference (view mode, sort order) remembered in this
 *  browser only. A blocked or full store just means it isn't remembered. */
export function usePersistedState<T>(key: string, initial: T): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const raw = localStorage.getItem(key);
      return raw ? (JSON.parse(raw) as T) : initial;
    } catch {
      return initial;
    }
  });
  return [
    value,
    (v: T) => {
      setValue(v);
      try {
        localStorage.setItem(key, JSON.stringify(v));
      } catch {
        // preference only — nothing depends on it persisting
      }
    },
  ];
}
