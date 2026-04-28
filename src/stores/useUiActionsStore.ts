// ── UI action bus ──────────────────────────────────────────────
//
// A tiny zustand store the app uses to dispatch UI-level actions
// from anywhere in the tree without threading callback props
// through every intermediate component. Right now the only
// subscriber is `App.tsx`, which listens for password-recovery
// requests so panels (Server Monitor, Terminal placeholder) can
// open the saved-connection editor without each one needing a
// dedicated `onEditConnection` prop chain.
//
// New entries here should stay tiny — a counter that increments on
// each request, plus the payload of the latest request — so React
// only reacts when something actually fires. Prefer adding a new
// counter+payload pair over piggy-backing existing ones, so a
// listener that only cares about action X doesn't re-render when
// action Y fires.

import { create } from "zustand";

type Store = {
  /** Bumped every time a recovery request fires so subscribers can
   *  detect the event with a single stable selector. */
  recoveryRequestSeq: number;
  /** Saved-connection index the requester wants to edit. Undefined
   *  before the first request. */
  recoveryRequestIndex: number | undefined;
  /** Fire a "user wants to re-enter the password for saved
   *  connection N" event. App.tsx opens the edit dialog. */
  requestEditConnection: (savedIndex: number) => void;
  /** Bumped every time a "open the Webhooks dialog and jump to
   *  the Failures tab" request fires — used by the failure-toast
   *  CTA so the user goes straight to the replay UI without
   *  having to navigate manually. */
  openWebhookFailuresSeq: number;
  /** Fire the open-failures-tab request. SoftwarePanel listens
   *  and opens the dialog with `activeTab = "failures"`. */
  openWebhookFailures: () => void;
};

export const useUiActionsStore = create<Store>((set) => ({
  recoveryRequestSeq: 0,
  recoveryRequestIndex: undefined,
  requestEditConnection: (savedIndex: number) =>
    set((state) => ({
      recoveryRequestSeq: state.recoveryRequestSeq + 1,
      recoveryRequestIndex: savedIndex,
    })),
  openWebhookFailuresSeq: 0,
  openWebhookFailures: () =>
    set((state) => ({
      openWebhookFailuresSeq: state.openWebhookFailuresSeq + 1,
    })),
}));
