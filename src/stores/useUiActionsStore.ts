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
  /** Bumped every time a panel asks to open the Settings dialog
   *  on a specific page (AI panel's unconfigured guide → "Ai"). */
  openSettingsSeq: number;
  /** Settings page key requested by the latest open-settings call. */
  openSettingsPage: string | undefined;
  /** Fire an "open Settings on page X" request. App.tsx listens. */
  requestOpenSettings: (page?: string) => void;
  /** Bumped every time something (e.g. the AI panel's insert-into-
   *  terminal button) wants keyboard focus returned to a terminal. */
  focusTerminalSeq: number;
  /** Session id of the terminal that should grab focus. The matching
   *  TerminalPanel focuses its viewport; others ignore the request. */
  focusTerminalSessionId: string | undefined;
  /** Fire a "pull focus back onto terminal <sessionId>" request. The
   *  AI panel calls this after inserting a command so the user can
   *  review and press Enter without first clicking the terminal. */
  requestFocusTerminal: (sessionId: string) => void;
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
  openSettingsSeq: 0,
  openSettingsPage: undefined,
  requestOpenSettings: (page?: string) =>
    set((state) => ({
      openSettingsSeq: state.openSettingsSeq + 1,
      openSettingsPage: page,
    })),
  focusTerminalSeq: 0,
  focusTerminalSessionId: undefined,
  requestFocusTerminal: (sessionId: string) =>
    set((state) => ({
      focusTerminalSeq: state.focusTerminalSeq + 1,
      focusTerminalSessionId: sessionId,
    })),
}));
