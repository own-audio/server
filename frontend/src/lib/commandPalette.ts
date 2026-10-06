// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

interface PaletteState {
  open: boolean;
  setOpen: (v: boolean) => void;
}

/** Whether the global search palette is showing. Opened by Cmd/Ctrl+K and
 *  the top bar's search field. */
export const useCommandPalette = create<PaletteState>()((set) => ({
  open: false,
  setOpen: (open) => set({ open }),
}));
