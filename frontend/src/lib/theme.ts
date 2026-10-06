// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

export type ThemePreference = "system" | "light" | "dark";

const STORAGE_KEY = "own-audio-theme";

function readStored(): ThemePreference {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "light" || v === "dark") return v;
  } catch {
    // storage unavailable — fall through to system
  }
  return "system";
}

function apply(pref: ThemePreference) {
  const root = document.documentElement;
  if (pref === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", pref);
}

interface ThemeState {
  preference: ThemePreference;
  setPreference: (p: ThemePreference) => void;
}

export const useTheme = create<ThemeState>()((set) => ({
  preference: readStored(),
  setPreference: (preference) => {
    try {
      if (preference === "system") localStorage.removeItem(STORAGE_KEY);
      else localStorage.setItem(STORAGE_KEY, preference);
    } catch {
      // ignore
    }
    apply(preference);
    set({ preference });
  },
}));

/** Call once before first render so there is no flash of the wrong theme. */
export function initTheme() {
  apply(readStored());
}

export function resolvedTheme(pref: ThemePreference): "light" | "dark" {
  if (pref !== "system") return pref;
  return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
}
