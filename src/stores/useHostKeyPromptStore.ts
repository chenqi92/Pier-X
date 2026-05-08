import { create } from "zustand";

/**
 * Tracks the set of in-flight host-key TOFU/mismatch prompts.
 *
 * pier-core's verifier holds a per-target singleflight `_gate` mutex
 * across `block_on(ssh_connect)` while it awaits the user's decision
 * on the prompt. Any keep-alive panel that fires an SSH-bound invoke
 * during that window blocks on the same gate; when the user finally
 * clicks "Trust", every queued invoke resolves in the same React
 * batch and the resulting render burst can starve WebView2 and paint
 * the window white.
 *
 * Polling panels read `hasPendingHostKeyPrompts()` synchronously in
 * their tick callbacks and skip the probe while a prompt is open, so
 * the queue never builds up in the first place.
 */
type HostKeyPromptState = {
  pending: Set<string>;
  add: (id: string) => void;
  remove: (id: string) => void;
};

export const useHostKeyPromptStore = create<HostKeyPromptState>((set) => ({
  pending: new Set<string>(),
  add: (id) =>
    set((s) => {
      if (s.pending.has(id)) return s;
      const next = new Set(s.pending);
      next.add(id);
      return { pending: next };
    }),
  remove: (id) =>
    set((s) => {
      if (!s.pending.has(id)) return s;
      const next = new Set(s.pending);
      next.delete(id);
      return { pending: next };
    }),
}));

/** Synchronous getter for polling ticks — does not subscribe. */
export function hasPendingHostKeyPrompts(): boolean {
  return useHostKeyPromptStore.getState().pending.size > 0;
}
