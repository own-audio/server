// SPDX-License-Identifier: AGPL-3.0-or-later
import { create } from "zustand";

export type ToastTone = "neutral" | "success" | "error";
export interface ToastAction { label: string; onClick: () => void }
export interface ToastItem { id: number; title: string; description?: string; tone: ToastTone; action?: ToastAction }

interface ToastState {
  items: ToastItem[];
  push: (t: Omit<ToastItem, "id">) => void;
  dismiss: (id: number) => void;
}

let nextId = 1;
export const useToasts = create<ToastState>()((set) => ({
  items: [],
  push: (t) => set((s) => ({ items: [...s.items, { ...t, id: nextId++ }] })),
  dismiss: (id) => set((s) => ({ items: s.items.filter((i) => i.id !== id) })),
}));

export const toast = {
  show: (title: string, description?: string) => useToasts.getState().push({ title, description, tone: "neutral" }),
  success: (title: string, description?: string) => useToasts.getState().push({ title, description, tone: "success" }),
  error: (title: string, description?: string) => useToasts.getState().push({ title, description, tone: "error" }),
  /** A toast with one action button — "Moved to Trash · Undo". */
  withAction: (title: string, action: ToastAction, description?: string) =>
    useToasts.getState().push({ title, description, tone: "neutral", action }),
};
