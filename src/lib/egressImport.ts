import type { SavedSshConnection } from "./types";

/** Outcome of parsing a clipboard blob into an importable egress
 *  profile shape. The import dialog consumes `patch` to overlay the
 *  parsed fields onto its draft. `wgConf` carries a wireguard `.conf`
 *  body the caller must write to the managed slot via
 *  `egressWgConfSave` before saving the profile. */
export type EgressImportResult =
  | {
      kind: "socks5" | "http";
      patch: {
        host: string;
        port: number;
        useAuth: boolean;
        authUser: string;
        authPassword: string;
      };
      hint: string;
    }
  | {
      kind: "ssh_jump";
      patch: { viaConnection: string };
      /** Free-form `user@host[:port]` we extracted from a `ProxyJump`
       *  string. The dialog matches it against saved SSH connections;
       *  the user picks one if no exact match. */
      jumpHint: { user: string; host: string; port: number };
      hint: string;
    }
  | {
      kind: "wireguard";
      /** Full wg-quick conf body — caller writes via egressWgConfSave. */
      wgConf: string;
      hint: string;
    };

const PROXY_URL_RE =
  /^(?<scheme>socks5h?|socks4|http|https):\/\/(?:(?<user>[^:@\s]+)(?::(?<pwd>[^@\s]*))?@)?(?<host>[^:/\s]+):(?<port>\d{1,5})(?:\/.*)?$/i;

const SSH_PROXYJUMP_RE =
  /(?:ssh\s+)?(?:-J|ProxyJump)[\s=]+(?<chain>[^\s]+)/i;

/** Bare `[user@]host[:port]` string the user may paste alone. We
 *  treat it as a ssh_jump hint when it looks plausibly like an SSH
 *  endpoint and the rest of the blob doesn't match a richer format. */
const BARE_USER_HOST_RE =
  /^(?:(?<user>[a-z_][\w.-]*)@)?(?<host>[a-z0-9.-]+)(?::(?<port>\d{1,5}))?$/i;

function parseProxyUrl(line: string): EgressImportResult | null {
  const m = line.match(PROXY_URL_RE);
  if (!m?.groups) return null;
  const port = Number.parseInt(m.groups.port, 10);
  if (!Number.isFinite(port) || port <= 0 || port > 65535) return null;
  const scheme = m.groups.scheme.toLowerCase();
  const kind: "socks5" | "http" = scheme.startsWith("http") ? "http" : "socks5";
  const user = m.groups.user ? decodeURIComponent(m.groups.user) : "";
  const pwd = m.groups.pwd ? decodeURIComponent(m.groups.pwd) : "";
  return {
    kind,
    patch: {
      host: m.groups.host,
      port,
      useAuth: user.length > 0,
      authUser: user,
      authPassword: pwd,
    },
    hint: `${kind.toUpperCase()} ${m.groups.host}:${port}`,
  };
}

function parseProxyJump(line: string): EgressImportResult | null {
  const m = line.match(SSH_PROXYJUMP_RE);
  const chain = m?.groups?.chain;
  if (!chain) return null;
  // `ProxyJump` accepts a comma-separated chain; first hop is the
  // immediate jump host, deeper hops would need to live in their own
  // ssh_jump profiles. Import the first hop and hint about the rest.
  const first = chain.split(",")[0]?.trim();
  if (!first) return null;
  const bare = first.match(BARE_USER_HOST_RE);
  if (!bare?.groups) return null;
  const port = bare.groups.port ? Number.parseInt(bare.groups.port, 10) : 22;
  return {
    kind: "ssh_jump",
    patch: { viaConnection: "" },
    jumpHint: {
      user: bare.groups.user ?? "",
      host: bare.groups.host,
      port,
    },
    hint:
      chain.includes(",")
        ? `ProxyJump → ${first} (+ deeper hops; create separate profiles)`
        : `ProxyJump → ${first}`,
  };
}

function parseBareUserHost(line: string): EgressImportResult | null {
  const m = line.match(BARE_USER_HOST_RE);
  if (!m?.groups) return null;
  // Reject tokens that are obviously not hostnames — the regex is
  // permissive enough to also match plain words otherwise.
  if (!m.groups.host.includes(".") && !m.groups.host.includes(":")) return null;
  const port = m.groups.port ? Number.parseInt(m.groups.port, 10) : 22;
  return {
    kind: "ssh_jump",
    patch: { viaConnection: "" },
    jumpHint: { user: m.groups.user ?? "", host: m.groups.host, port },
    hint: `SSH endpoint ${m.groups.user ? m.groups.user + "@" : ""}${m.groups.host}:${port}`,
  };
}

function parseWireguard(blob: string): EgressImportResult | null {
  // wg-quick conf shape: an [Interface] section with a PrivateKey,
  // followed by one or more [Peer] sections. We don't try to parse
  // the contents — the backend hands the file straight to wg-quick.
  if (!/\[Interface\][\s\S]*PrivateKey\s*=/i.test(blob)) return null;
  if (!/\[Peer\][\s\S]*PublicKey\s*=/i.test(blob)) return null;
  return {
    kind: "wireguard",
    wgConf: blob.trim() + "\n",
    hint: "WireGuard wg-quick conf",
  };
}

/** Best-effort detection. Order matters: structured formats (full URL,
 *  WireGuard conf, explicit ProxyJump) win over the bare `user@host`
 *  fallback so a SOCKS URL doesn't accidentally get classified as an
 *  SSH endpoint by stripping the scheme. */
export function parseEgressClipboard(raw: string): EgressImportResult | null {
  const blob = raw.trim();
  if (!blob) return null;
  // Multi-line: try wireguard first, then fall back to per-line
  // matchers (e.g. an `~/.ssh/config` snippet pasted whole).
  if (blob.includes("\n")) {
    const wg = parseWireguard(blob);
    if (wg) return wg;
    for (const line of blob.split(/\r?\n/)) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      const r = parseProxyUrl(trimmed) ?? parseProxyJump(trimmed);
      if (r) return r;
    }
    return null;
  }
  return (
    parseProxyUrl(blob) ??
    parseProxyJump(blob) ??
    parseBareUserHost(blob)
  );
}

/** Find the saved SSH connection that best matches a `ProxyJump`
 *  hint. Match precedence: exact `(user, host, port)`, then `(host,
 *  port)`, then `host` alone. Returns the connection name (the field
 *  `EgressProfile.ssh_jump.viaConnection` expects). */
export function matchJumpConnection(
  hint: { user: string; host: string; port: number },
  connections: SavedSshConnection[],
): string | null {
  const userMatch = connections.find(
    (c) =>
      c.host === hint.host &&
      c.port === hint.port &&
      (hint.user === "" || c.user === hint.user),
  );
  if (userMatch) return userMatch.name;
  const hostPortMatch = connections.find(
    (c) => c.host === hint.host && c.port === hint.port,
  );
  if (hostPortMatch) return hostPortMatch.name;
  const hostMatch = connections.find((c) => c.host === hint.host);
  return hostMatch?.name ?? null;
}
