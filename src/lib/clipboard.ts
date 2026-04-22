// ── Clipboard helpers ──────────────────────────────────────────
//
// `navigator.clipboard.readText` / `writeText` work in browsers but
// the Chromium webview Tauri bundles treats the page like a remote
// origin and shows a "this site wants to read the clipboard"
// permission prompt the first time `readText` runs. That prompt is
// jarring in a desktop app — the OS already trusts the application
// shell with the clipboard.
//
// `tauri-plugin-clipboard-manager` exposes the same operations
// through Tauri's IPC, which never raises that prompt. We try the
// plugin first (succeeds in any Tauri build with the plugin
// registered), then fall back to the Web API for non-Tauri runtimes
// like vanilla `vite preview` or unit tests.

import {
  readText as tauriReadText,
  writeText as tauriWriteText,
} from "@tauri-apps/plugin-clipboard-manager";

function isTauriRuntime(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

/**
 * Read the OS clipboard as plain text. Returns the empty string if
 * the clipboard is empty or the read failed (silent — callers
 * should handle empty input as "nothing to paste"). Never throws.
 */
export async function readClipboardText(): Promise<string> {
  if (isTauriRuntime()) {
    try {
      const text = await tauriReadText();
      return typeof text === "string" ? text : "";
    } catch {
      return "";
    }
  }
  if (typeof navigator !== "undefined" && navigator.clipboard?.readText) {
    try {
      return await navigator.clipboard.readText();
    } catch {
      return "";
    }
  }
  return "";
}

/**
 * Write `text` to the OS clipboard. Resolves once the write
 * completes; swallows errors so a clipboard write blocked by the
 * environment does not propagate to UI handlers (the user can just
 * try again).
 */
export async function writeClipboardText(text: string): Promise<void> {
  if (isTauriRuntime()) {
    try {
      await tauriWriteText(text);
      return;
    } catch {
      // Fall through to the Web API in case the plugin isn't
      // registered for some reason; the Web API may still succeed.
    }
  }
  if (typeof navigator !== "undefined" && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* swallow — clipboard writes are best-effort */
    }
  }
}
