import type { I18nValue } from "./useI18n";

type Translator = I18nValue["t"];

function getMessage(error: unknown) {
  if (typeof error === "string") return error;
  if (error instanceof Error) return error.message;
  return String(error);
}

function translateExact(message: string, t: Translator) {
  const translated = t(message);
  return translated !== message ? translated : null;
}

function formatTriedMethods(raw: string) {
  const normalized = raw.trim().replace(/^\[/, "").replace(/\]$/, "").replace(/"/g, "");
  return normalized || raw;
}

function localizeRuntimeMessageInternal(message: string, t: Translator, depth: number): string {
  const value = String(message || "").trim();
  if (!value) return "";
  if (depth > 6) return value;

  const wrappers = [
    /^Error invoking tauri command ['"`]?[^'"`]+['"`]?:\s*(.+)$/i,
    /^Error invoking [^:]+:\s*(.+)$/i,
    /^failed to invoke command ['"`]?[^'"`]+['"`]?:\s*(.+)$/i,
  ];

  for (const wrapper of wrappers) {
    const match = value.match(wrapper);
    if (match) {
      return localizeRuntimeMessageInternal(match[1], t, depth + 1);
    }
  }

  const exact = translateExact(value, t);
  if (exact) return exact;

  const patterns: Array<{
    pattern: RegExp;
    resolve: (match: RegExpMatchArray) => string;
  }> = [
    {
      pattern: /^unknown saved SSH connection: (.+)$/i,
      resolve: ([, index]) => t("Unknown saved SSH connection #{index}.", { index }),
    },
    {
      pattern: /^unknown tunnel: (.+)$/i,
      resolve: ([, id]) => t("Unknown tunnel: {id}.", { id }),
    },
    {
      pattern: /^unknown terminal session: (.+)$/i,
      resolve: ([, id]) => t("Unknown terminal session: {id}.", { id }),
    },
    {
      pattern: /^unknown log stream: (.+)$/i,
      resolve: ([, id]) => t("Unknown log stream: {id}.", { id }),
    },
    {
      pattern: /^ssh connect failed: (.+)$/i,
      resolve: ([, detail]) => t("SSH connection failed: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^ssh protocol: (.+)$/i,
      resolve: ([, detail]) => t("SSH protocol error: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^ssh authentication rejected \(tried: (.+)\)$/i,
      resolve: ([, methods]) => t("SSH authentication was rejected (tried: {methods}).", {
        methods: formatTriedMethods(methods),
      }),
    },
    {
      pattern: /^ssh host key mismatch for (.+): got (.+)$/i,
      resolve: ([, host, fingerprint]) => t("SSH host key mismatch for {host}: {fingerprint}", {
        host,
        fingerprint,
      }),
    },
    {
      pattern: /^ssh connect timeout after (.+)$/i,
      resolve: ([, duration]) => t("SSH connection timed out after {duration}.", { duration }),
    },
    {
      pattern: /^invalid ssh config: (.+)$/i,
      resolve: ([, detail]) => t("Invalid SSH configuration: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^ssh i\/o: (.+)$/i,
      resolve: ([, detail]) => t("SSH I/O error: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^docker ps exited (\d+):\s*(.*)$/i,
      resolve: ([, exit, detail]) =>
        t("Docker command `ps` exited with code {exit}: {detail}", {
          exit,
          detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
        }),
    },
    {
      pattern: /^docker inspect exited (\d+):\s*(.*)$/i,
      resolve: ([, exit, detail]) =>
        t("Docker inspect exited with code {exit}: {detail}", {
          exit,
          detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
        }),
    },
    {
      pattern: /^docker images exited (\d+)$/i,
      resolve: ([, exit]) => t("Docker images exited with code {exit}.", { exit }),
    },
    {
      pattern: /^docker volume ls exited (\d+)$/i,
      resolve: ([, exit]) => t("Docker volume list exited with code {exit}.", { exit }),
    },
    {
      pattern: /^docker network ls exited (\d+)$/i,
      resolve: ([, exit]) => t("Docker network list exited with code {exit}.", { exit }),
    },
    {
      pattern: /^docker rmi exited (\d+):\s*(.*)$/i,
      resolve: ([, exit, detail]) =>
        t("Docker image removal exited with code {exit}: {detail}", {
          exit,
          detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
        }),
    },
    {
      pattern: /^docker ([^:]+) exited (\d+):\s*(.*)$/i,
      resolve: ([, verb, exit, detail]) =>
        t("Docker `{verb}` exited with code {exit}: {detail}", {
          verb,
          exit,
          detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
        }),
    },
    {
      pattern: /^docker ps failed: (.+)$/i,
      resolve: ([, detail]) => t("Docker `ps` failed: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^docker ([^:]+) failed: (.+)$/i,
      resolve: ([, verb, detail]) => t("Docker `{verb}` failed: {detail}", {
        verb,
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^unknown docker action: (.+)$/i,
      resolve: ([, action]) => t("Unknown Docker action: {action}.", { action }),
    },
    {
      pattern: /^refusing unsafe docker id (.+)$/i,
      resolve: ([, id]) => t("Refusing unsafe Docker identifier: {id}", { id }),
    },
    {
      pattern: /^unsafe image id (.+)$/i,
      resolve: ([, id]) => t("Unsafe image id: {id}", { id }),
    },
    {
      pattern: /^unsafe id: (.+)$/i,
      resolve: ([, id]) => t("Unsafe identifier: {id}", { id }),
    },
    {
      pattern: /^failed to run git: (.+)$/i,
      resolve: ([, detail]) => t("Failed to run git: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^failed to run git (.+): (.+)$/i,
      resolve: ([, args, detail]) => t("Failed to run git {args}: {detail}", {
        args,
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^git (.+) failed: (.+)$/i,
      resolve: ([, args, detail]) => t("Git {args} failed: {detail}", {
        args,
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^git (.+) failed$/i,
      resolve: ([, args]) => t("Git {args} failed", { args }),
    },
    {
      pattern: /^Failed to read (.+): (.+)$/i,
      resolve: ([, path, detail]) => t("Failed to read {path}: {detail}", {
        path,
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^Failed to write (.+): (.+)$/i,
      resolve: ([, path, detail]) => t("Failed to write {path}: {detail}", {
        path,
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^Updated remote '(.+)'\.$/i,
      resolve: ([, name]) => t("Updated remote '{name}'.", { name }),
    },
    {
      pattern: /^Fetched remote '(.+)'\.$/i,
      resolve: ([, name]) => t("Fetched remote '{name}'.", { name }),
    },
    {
      pattern: /^Accepted (.+) for (.+)$/i,
      resolve: ([, resolved, path]) => t("Accepted {resolved} for {path}.", { resolved, path }),
    },
    {
      pattern: /^Marked (.+) as resolved$/i,
      resolve: ([, path]) => t("Marked {path} as resolved.", { path }),
    },
    {
      pattern: /^invalid config: (.+)$/i,
      resolve: ([, detail]) => t("Invalid configuration: {detail}", {
        detail: localizeRuntimeMessageInternal(detail, t, depth + 1),
      }),
    },
    {
      pattern: /^refusing unsafe database identifier (.+)$/i,
      resolve: ([, name]) => t("Refusing unsafe database identifier: {name}", { name }),
    },
    {
      pattern: /^refusing unsafe table identifier (.+)$/i,
      resolve: ([, name]) => t("Refusing unsafe table identifier: {name}", { name }),
    },
    {
      pattern: /^refusing unsafe schema identifier (.+)$/i,
      resolve: ([, name]) => t("Refusing unsafe schema identifier: {name}", { name }),
    },
    {
      pattern: /^unexpected PING reply: (.+)$/i,
      resolve: ([, reply]) => t("Unexpected PING reply: {reply}", { reply }),
    },
    {
      pattern: /^file not found: (.+)$/i,
      resolve: ([, path]) => t("File not found: {path}", { path }),
    },
    {
      pattern: /^sqlite3 not found: (.+)$/i,
      resolve: ([, detail]) => t("`sqlite3` not found: {detail}", { detail }),
    },
    {
      pattern: /^not a valid database: (.+)$/i,
      resolve: ([, detail]) => t("Not a valid database: {detail}", { detail }),
    },
    {
      pattern: /^cannot read (.+): (.+)$/i,
      resolve: ([, path, detail]) => t("Cannot read {path}: {detail}", { path, detail }),
    },
    {
      pattern: /^SSH agent list failed: (.+)$/i,
      resolve: ([, detail]) => t("SSH agent list failed: {detail}", { detail }),
    },
    {
      pattern: /^request_subsystem\(sftp\): (.+)$/i,
      resolve: ([, detail]) => t("Failed to request the SFTP subsystem: {detail}", { detail }),
    },
    {
      pattern: /^SftpSession::new: (.+)$/i,
      resolve: ([, detail]) => t("Failed to create the SFTP session: {detail}", { detail }),
    },
    {
      pattern: /^known_hosts: (.+)$/i,
      resolve: ([, detail]) => t("Failed to read known_hosts: {detail}", { detail }),
    },
  ];

  for (const entry of patterns) {
    const match = value.match(entry.pattern);
    if (match) {
      return entry.resolve(match);
    }
  }

  return value;
}

export function localizeRuntimeMessage(message: string, t: Translator) {
  return localizeRuntimeMessageInternal(message, t, 0);
}

export function localizeError(error: unknown, t: Translator) {
  return localizeRuntimeMessageInternal(getMessage(error), t, 0);
}
