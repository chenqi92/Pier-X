import { create } from "zustand";

export type ToastKind = "info" | "success" | "warning" | "error";

/// Optional CTA rendered inline on a toast — e.g. "Open Failures
/// tab" on a webhook-failed alert. Clicking dismisses the toast
/// AND fires the callback. Stored as a function reference so the
/// renderer doesn't need to dispatch through the store.
export type ToastAction = {
  /** Visible button label (already localised). */
  label: string;
  onClick: () => void;
};

export type ToastItem = {
  id: number;
  kind: ToastKind;
  message: string;
  /** Milliseconds before auto-dismiss. 0 = sticky (user must close). */
  duration: number;
  /** Optional inline action — rendered as a small button after the
   *  message text. Click dismisses the toast as a side-effect. */
  action?: ToastAction;
};

type ToastStore = {
  toasts: ToastItem[];
  /** Push a toast; returns its id. */
  push: (input: {
    kind?: ToastKind;
    message: string;
    duration?: number;
    action?: ToastAction;
  }) => number;
  dismiss: (id: number) => void;
  clear: () => void;
};

// Monotonic counter — module-scope so id assignment is predictable
// regardless of mount/unmount churn.
let nextId = 1;

export const useToastStore = create<ToastStore>((set, get) => ({
  toasts: [],

  push: ({ kind = "info", message, duration = 4000, action }) => {
    const id = nextId++;
    set((s) => ({
      toasts: [...s.toasts, { id, kind, message, duration, action }],
    }));
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
  /** Action-aware variant — renders a button next to the message
   *  that fires `action.onClick` AND dismisses the toast. Used for
   *  e.g. "Open Failures tab" on a webhook-failed alert. */
  withAction: (
    kind: ToastKind,
    message: string,
    action: ToastAction,
    duration?: number,
  ) =>
    useToastStore.getState().push({
      kind,
      message,
      duration: duration ?? (kind === "error" ? 8000 : 6000),
      action,
    }),
};
