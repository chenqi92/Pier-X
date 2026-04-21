import type { LogSource, LogSourceMode } from "./types";

// Shell-escape a path / argument for bash. Wraps in single quotes
// and doubles up any embedded single quotes. Good enough for the
// tail/journalctl/docker-logs args we emit here.
export function shellEscape(s: string): string {
  return `'${s.split("'").join("'\\''")}'`;
}

export type LogSystemPreset = {
  id: string;
  /** Short human label shown in the dropdown. */
  label: string;
  /** One-line description for the picker summary. */
  description: string;
  /** Name of the argument input (UNIT / CONTAINER / …). Omit when no arg is needed. */
  argLabel?: string;
  /** Placeholder for the argument input when one is needed. */
  argPlaceholder?: string;
  /** Produce the shell command to run. Return "" when the preset is
   *  incomplete (e.g. journald unit missing its unit name) so callers
   *  can gate the Start button. */
  compile: (arg: string) => string;
};

export const LOG_SYSTEM_PRESETS: LogSystemPreset[] = [
  {
    id: "syslog",
    label: "syslog",
    description: "/var/log/syslog",
    compile: () => "tail -F /var/log/syslog",
  },
  {
    id: "auth",
    label: "auth.log",
    description: "/var/log/auth.log",
    compile: () => "tail -F /var/log/auth.log",
  },
  {
    id: "nginx-access",
    label: "nginx access",
    description: "/var/log/nginx/access.log",
    compile: () => "tail -F /var/log/nginx/access.log",
  },
  {
    id: "nginx-error",
    label: "nginx error",
    description: "/var/log/nginx/error.log",
    compile: () => "tail -F /var/log/nginx/error.log",
  },
  {
    id: "dmesg",
    label: "dmesg",
    description: "kernel ring buffer",
    compile: () => "dmesg -w",
  },
  {
    id: "journald",
    label: "journald (all)",
    description: "journalctl -f",
    compile: () => "journalctl -f",
  },
  {
    id: "journald-unit",
    label: "journald unit",
    description: "journalctl -u <unit> -f",
    argLabel: "UNIT",
    argPlaceholder: "e.g. nginx",
    compile: (arg) => {
      const u = arg.trim();
      return u ? `journalctl -u ${shellEscape(u)} -f` : "";
    },
  },
  {
    id: "docker-container",
    label: "docker container",
    description: "docker logs -f <container>",
    argLabel: "CONTAINER",
    argPlaceholder: "id or name",
    compile: (arg) => {
      const c = arg.trim();
      return c ? `docker logs -f ${shellEscape(c)}` : "";
    },
  },
];

export function findPreset(id: string): LogSystemPreset | undefined {
  return LOG_SYSTEM_PRESETS.find((p) => p.id === id);
}

/** Extensions we treat as "log-like" when filtering a directory for the File-mode dropdown. */
export const LOG_FILE_EXTS = new Set([".log", ".out", ".err", ".txt"]);

export function isLogLikeFilename(name: string): boolean {
  const lower = name.toLowerCase();
  for (const ext of LOG_FILE_EXTS) {
    if (lower.endsWith(ext)) return true;
  }
  // numbered rotations: nginx.access.log.1, auth.log.2
  return /\.log\.\d+$/.test(lower);
}

/** Compile a LogSource into the shell command for `log_stream_start`.
 *  Returns "" when the selection is incomplete — callers should gate
 *  the Start button accordingly. */
export function compileLogSource(src: LogSource): string {
  switch (src.mode) {
    case "file": {
      const p = src.filePath.trim();
      return p ? `tail -F ${shellEscape(p)}` : "";
    }
    case "system": {
      const preset = findPreset(src.systemPresetId);
      return preset ? preset.compile(src.systemArg) : "";
    }
    case "custom":
      return src.customCommand.trim();
  }
}

/** Short human summary shown in the lg-head row. */
export function describeLogSource(src: LogSource): string {
  switch (src.mode) {
    case "file":
      return src.filePath.trim() || "(no file selected)";
    case "system": {
      const preset = findPreset(src.systemPresetId);
      if (!preset) return "(no preset)";
      const arg = src.systemArg.trim();
      if (preset.argLabel && !arg) return `${preset.label} (missing ${preset.argLabel.toLowerCase()})`;
      return preset.argLabel ? `${preset.label} · ${arg}` : preset.label;
    }
    case "custom":
      return src.customCommand.trim() || "(no command)";
  }
}

export const MODES: { id: LogSourceMode; label: string }[] = [
  { id: "file", label: "File" },
  { id: "system", label: "System" },
  { id: "custom", label: "Custom" },
];
