// Desktop + in-app notification helper.
//
// We deliberately avoid pulling in `tauri-plugin-notification` and
// its sister JS package — the browser's `Notification` API works
// inside Tauri's webview on every supported platform, and the
// in-app toast is the only fallback we need when permission is
// denied or the OS has DND on.
//
// Calling `desktopNotify(kind, title, body)` always posts a toast
// (so the user sees something even when the OS is suppressing
// notifications) and additionally tries to emit a system-level
// notification when permission has been granted.

import { toast } from "../stores/useToastStore";

type NotifyKind = "info" | "warning" | "error";

/** Optional inline action rendered as a button on the toast.
 *  Has no effect on the desktop-level Notification (the OS UI
 *  doesn't carry actions consistently across platforms). */
export type NotifyAction = {
  label: string;
  onClick: () => void;
};

let permissionState: NotificationPermission | "unsupported" = "default";

/**
 * Initialise notification permission once during app startup.
 * Called from `App.tsx` so the user sees the prompt at most once
 * per Pier-X session, not at the moment a webhook fires.
 *
 * No-op on platforms where the webview doesn't expose the
 * `Notification` constructor (rare — older Linux WebKit, some
 * embedded webviews); we just fall back to in-app toasts.
 */
export async function initDesktopNotifications(): Promise<void> {
  if (typeof Notification === "undefined") {
    permissionState = "unsupported";
    return;
  }
  if (Notification.permission === "default") {
    try {
      permissionState = await Notification.requestPermission();
    } catch {
      // Some environments throw if the call shape is wrong; treat
      // as denied and fall through to toasts.
      permissionState = "denied";
    }
  } else {
    permissionState = Notification.permission;
  }
}

/**
 * Push an in-app toast + (when permission allows) a system-level
 * desktop notification. `kind` controls the toast's severity
 * styling; the desktop notification shape is identical across
 * kinds — OS UI doesn't carry "warning" vs "error" semantics.
 *
 * The toast always fires so the user sees the event when the
 * app is in the foreground; the OS notification is the
 * background-attention escalation.
 */
export function desktopNotify(
  kind: NotifyKind,
  title: string,
  body?: string,
  action?: NotifyAction,
): void {
  const message = body ? `${title} — ${body}` : title;
  if (action) {
    const toastKind =
      kind === "warning" ? "warning" : kind === "error" ? "error" : "info";
    toast.withAction(toastKind, message, action);
  } else if (kind === "warning") {
    toast.warn(message);
  } else if (kind === "error") {
    toast.error(message);
  } else {
    toast.info(message);
  }

  if (permissionState !== "granted") return;
  try {
    // `tag` lets the OS coalesce repeated notifications about the
    // same event class — e.g. ten webhook failures in a row
    // collapse into one badge instead of stacking ten toasts on
    // macOS Notification Center.
    const tag = `pier-x-${kind}-${title}`;
    new Notification(title, {
      body: body ?? "",
      tag,
      // `silent` is honoured on Windows + some Linux DEs; macOS
      // ignores it. Either way, no harm.
      silent: false,
    });
  } catch {
    // Construction can throw under heavy throttling or when the
    // user revoked permission mid-session. Toast already
    // surfaced; nothing more to do.
  }
}
