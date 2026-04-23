/**
 * Thin frontend-side wrapper around the backend file logger.
 *
 * Every warning / error printed to the browser console — plus every
 * uncaught exception and unhandled promise rejection — is mirrored into
 * the same `pier-x.log` file the Rust side writes to. The file is
 * truncated on every app startup (see `pier_core::logging::init`), so
 * it never grows without bound but gives us a complete timeline of the
 * current run.
 *
 * Writes are batched: we coalesce bursts into a single IPC call every
 * 250ms so a tight `console.log` loop in a panel doesn't turn each
 * character into a Tauri command. On page unload we flush synchronously
 * (best-effort) so the last few events land before the webview tears
 * down.
 */
import { invoke, isTauri } from "@tauri-apps/api/core";

type Level = "DEBUG" | "INFO" | "WARN" | "ERROR";

type PendingRecord = {
  level: Level;
  source: string;
  message: string;
};

const FLUSH_INTERVAL_MS = 250;
const MAX_QUEUE = 512;

let queue: PendingRecord[] = [];
let flushScheduled = false;
let initialized = false;

function isRunningInTauri(): boolean {
  try {
    return isTauri();
  } catch {
    return false;
  }
}

function scheduleFlush() {
  if (flushScheduled) return;
  flushScheduled = true;
  window.setTimeout(() => {
    flushScheduled = false;
    void flush();
  }, FLUSH_INTERVAL_MS);
}

async function flush() {
  if (queue.length === 0) return;
  const batch = queue;
  queue = [];
  if (!isRunningInTauri()) return;
  for (const rec of batch) {
    try {
      await invoke("log_write", rec);
    } catch {
      // Swallow — a logger that errors on write would recurse via the
      // error handler we install below. Better to lose the line.
    }
  }
}

function enqueue(level: Level, source: string, parts: unknown[]) {
  const message = parts.map(stringifyArg).join(" ");
  queue.push({ level, source, message });
  if (queue.length > MAX_QUEUE) {
    // Drop the oldest half so a runaway loop doesn't chew unbounded memory.
    queue.splice(0, queue.length - MAX_QUEUE / 2);
  }
  scheduleFlush();
}

/** Best-effort stringify that handles Errors, objects, and primitives
 *  without throwing (JSON.stringify can explode on cyclic graphs). */
function stringifyArg(v: unknown): string {
  if (v instanceof Error) {
    const stack = v.stack ? `\n${v.stack}` : "";
    return `${v.name}: ${v.message}${stack}`;
  }
  if (v === null) return "null";
  if (v === undefined) return "undefined";
  if (typeof v === "string") return v;
  if (typeof v === "number" || typeof v === "boolean" || typeof v === "bigint") {
    return String(v);
  }
  try {
    return JSON.stringify(v);
  } catch {
    try {
      return String(v);
    } catch {
      return "[unserializable]";
    }
  }
}

/** Install the console + error hooks. Idempotent — callers can re-run
 *  safely during HMR without double-wrapping. */
export function initLogger() {
  if (initialized) return;
  initialized = true;

  const origLog = console.log.bind(console);
  const origInfo = console.info.bind(console);
  const origWarn = console.warn.bind(console);
  const origError = console.error.bind(console);
  const origDebug = console.debug.bind(console);

  console.log = (...args: unknown[]) => {
    origLog(...args);
    enqueue("INFO", "console.log", args);
  };
  console.info = (...args: unknown[]) => {
    origInfo(...args);
    enqueue("INFO", "console.info", args);
  };
  console.warn = (...args: unknown[]) => {
    origWarn(...args);
    enqueue("WARN", "console.warn", args);
  };
  console.error = (...args: unknown[]) => {
    origError(...args);
    enqueue("ERROR", "console.error", args);
  };
  console.debug = (...args: unknown[]) => {
    origDebug(...args);
    enqueue("DEBUG", "console.debug", args);
  };

  window.addEventListener("error", (event) => {
    const src = event.filename ? `${event.filename}:${event.lineno}:${event.colno}` : "window";
    enqueue("ERROR", src, [event.message, event.error]);
  });

  window.addEventListener("unhandledrejection", (event) => {
    enqueue("ERROR", "unhandledrejection", [event.reason]);
  });

  // Best-effort flush on page hide / unload so the last few events
  // make it to disk before the webview detaches.
  const flushOnExit = () => {
    void flush();
  };
  window.addEventListener("beforeunload", flushOnExit);
  window.addEventListener("pagehide", flushOnExit);

  // Advertise our identity so the log starts with something useful.
  enqueue("INFO", "frontend", [
    `Pier-X frontend online; ua=${navigator.userAgent}; lang=${navigator.language}`,
  ]);
  scheduleFlush();
}

/** Manual log entry, used by panels that want to annotate the timeline
 *  without going through `console.*`. */
export function logEvent(level: Level, source: string, message: string) {
  enqueue(level, source, [message]);
}

/** Resolve the absolute path of the active log file. Empty string if
 *  the backend hasn't initialised the logger yet (shouldn't happen in
 *  practice). */
export async function getLogFilePath(): Promise<string> {
  if (!isRunningInTauri()) return "";
  try {
    return await invoke<string>("log_file_path");
  } catch {
    return "";
  }
}

/** Read the tail of the log file. `maxBytes` defaults to 2 MiB on the
 *  backend; leave it undefined unless you specifically want a smaller
 *  slice. */
export async function readLogTail(maxBytes?: number): Promise<string> {
  if (!isRunningInTauri()) return "";
  try {
    return await invoke<string>("log_read_tail", { maxBytes: maxBytes ?? null });
  } catch (e) {
    return String(e);
  }
}

/** Truncate the log file to 0 bytes. No-op outside Tauri. */
export async function clearLogFile(): Promise<void> {
  if (!isRunningInTauri()) return;
  await invoke<void>("log_clear");
}

/** Read/set the verbose logging flag. */
export async function getLogVerbose(): Promise<boolean> {
  if (!isRunningInTauri()) return false;
  try {
    return await invoke<boolean>("log_get_verbose");
  } catch {
    return false;
  }
}

export async function setLogVerbose(enabled: boolean): Promise<void> {
  if (!isRunningInTauri()) return;
  await invoke<void>("log_set_verbose", { enabled });
}
