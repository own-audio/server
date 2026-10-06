// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

const KEY = "own-audio-sidebar-collapsed";

function read(): boolean {
  try {
    return localStorage.getItem(KEY) === "1";
  } catch {
    return false;
  }
}

interface SidebarState {
  collapsed: boolean;
  toggle: () => void;
  set: (v: boolean) => void;
}

/** Whether the sidebar is an icon rail. A per-viewer preference, so it lives
 *  in localStorage; a blocked store just means it isn't remembered. */
export const useSidebar = create<SidebarState>()((set, get) => {
  const persist = (collapsed: boolean) => {
    try {
      localStorage.setItem(KEY, collapsed ? "1" : "0");
    } catch {
      // preference only
    }
    return { collapsed };
  };
  return {
    collapsed: read(),
    toggle: () => set(persist(!get().collapsed)),
    set: (v) => set(persist(v)),
  };
});
