// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

interface ShortcutsState {
  open: boolean;
  setOpen: (v: boolean) => void;
}

/** Whether the keyboard-shortcuts dialog is showing. Opened with `?`. */
export const useShortcuts = create<ShortcutsState>()((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));
