import { create } from "zustand";

export type ToastKind = "info" | "success" | "warning" | "error";

export type ToastItem = {
  id: number;
  kind: ToastKind;
  message: string;
  /** Milliseconds before auto-dismiss. 0 = sticky (user must close). */
  duration: number;
};

type ToastStore = {
  toasts: ToastItem[];
  /** Push a toast; returns its id. */
  push: (input: { kind?: ToastKind; message: string; duration?: number }) => number;
  dismiss: (id: number) => void;
  clear: () => void;
};

// Monotonic counter — module-scope so id assignment is predictable
// regardless of mount/unmount churn.
let nextId = 1;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],

  push: ({ kind = "info", message, duration = 4000 }) => {
    const id = nextId++;
    set((s) => ({ toasts: [...s.toasts, { id, kind, message, duration }] }));
    if (duration > 0 && typeof window !== "undefined") {
      window.setTimeout(() => get().dismiss(id), duration);
    }
    return id;
  },

  dismiss: (id) => {
    set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) }));
  },

  clear: () => set({ toasts: [] }),
}));

// Convenience helpers so callers don't have to reach into the store.
export const toast = {
  info: (message: string, duration?: number) =>
    useToastStore.getState().push({ kind: "info", message, duration }),
  success: (message: string, duration?: number) =>
    useToastStore.getState().push({ kind: "success", message, duration }),
  warn: (message: string, duration?: number) =>
    useToastStore.getState().push({ kind: "warning", message, duration }),
  error: (message: string, duration?: number) =>
    useToastStore.getState().push({ kind: "error", message, duration: duration ?? 6000 }),
};
