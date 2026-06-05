import { create } from "zustand";

/**
 * Imperative confirmation API — a themed replacement for the
 * `window.confirm` calls scattered across panels. `window.confirm`
 * is unreliable inside Tauri's webview (it can return `undefined`,
 * letting an `if (!window.confirm(...)) return` guard fall through
 * so a destructive action fires with no prompt — see
 * `SftpPanel`/`DbInstancePicker`). This store + the single
 * `<ConfirmHost/>` mounted at the app root render one themed
 * `ConfirmDialog` and resolve a promise, so call sites stay almost
 * identical in shape:
 *
 *   if (!(await confirm({ message, tone: "destructive" }))) return;
 */
export type ConfirmOptions = {
  /** Dialog title. Defaults to a generic "Confirm" in the host. */
  title?: string;
  /** Body text. Pre-interpolated by the caller (no `t()` vars here). */
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  /** `destructive` paints the confirm button red. Default neutral. */
  tone?: "neutral" | "destructive";
};

type ConfirmRequest = ConfirmOptions & { id: number };

type ConfirmState = {
  request: ConfirmRequest | null;
  resolve: ((ok: boolean) => void) | null;
  confirm: (opts: ConfirmOptions) => Promise<boolean>;
  /** Internal — called by the host when the user confirms/cancels. */
  settle: (ok: boolean) => void;
};

let nextId = 1;

export const useConfirmStore = create<ConfirmState>((set, get) => ({
  request: null,
  resolve: null,
  confirm: (opts) =>
    new Promise<boolean>((resolve) => {
      // If a prior request is still open (rapid double-trigger),
      // resolve it as cancelled before replacing it so its awaiter
      // doesn't hang forever.
      const prev = get().resolve;
      if (prev) prev(false);
      set({ request: { ...opts, id: nextId++ }, resolve });
    }),
  settle: (ok) => {
    const r = get().resolve;
    set({ request: null, resolve: null });
    if (r) r(ok);
  },
}));

/** Convenience wrapper so call sites can `import { confirm }`. */
export function confirm(opts: ConfirmOptions): Promise<boolean> {
  return useConfirmStore.getState().confirm(opts);
}
