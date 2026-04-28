import type { NginxNode } from "../lib/commands";

// ── Types ────────────────────────────────────────────────────────

export type Directive = Extract<NginxNode, { kind: "directive" }>;
export type FieldValue = string | boolean;
export type FieldValues = Record<string, FieldValue>;

export type FieldDef =
  | { key: string; type: "text"; label: string; placeholder?: string }
  | { key: string; type: "textarea"; label: string; placeholder?: string; rows?: number }
  | { key: string; type: "switch"; label: string }
  | {
      key: string;
      type: "select";
      label: string;
      options: { value: string; label: string }[];
    };

export type ScopeKind = "main" | "http" | "server";

export type FeatureGroup =
  | "tls"
  | "security"
  | "performance"
  | "logging"
  | "proxy"
  | "real-ip"
  | "rate-limit"
  | "worker";

export type Feature = {
  id: string;
  group: FeatureGroup;
  contexts: ScopeKind[];
  title: string;
  description: string;
  fields: FieldDef[];
  defaults: FieldValues;
  detect: (block: NginxNode[]) => boolean;
  read: (block: NginxNode[]) => FieldValues;
  enable: (block: NginxNode[], values: FieldValues) => NginxNode[];
  disable: (block: NginxNode[]) => NginxNode[];
  requires?: (block: NginxNode[], scope: ScopeKind) => string | null;
};

// ── AST helpers ──────────────────────────────────────────────────

export const isDirective = (n: NginxNode): n is Directive =>
  n.kind === "directive";

export function newDirective(name: string, args: string[]): Directive {
  return {
    kind: "directive",
    name,
    args,
    leadingComments: [],
    leadingBlanks: 0,
    inlineComment: null,
    block: null,
    opaqueBody: null,
  };
}

export function newBlockDirective(name: string, args: string[]): Directive {
  return {
    kind: "directive",
    name,
    args,
    leadingComments: [],
    leadingBlanks: 0,
    inlineComment: null,
    block: [],
    opaqueBody: null,
  };
}

export function findFirst(
  block: NginxNode[],
  name: string,
): { idx: number; d: Directive } | null {
  for (let i = 0; i < block.length; i++) {
    const n = block[i];
    if (isDirective(n) && n.name === name) return { idx: i, d: n };
  }
  return null;
}

export function findAll(
  block: NginxNode[],
  name: string,
): { idx: number; d: Directive }[] {
  const out: { idx: number; d: Directive }[] = [];
  for (let i = 0; i < block.length; i++) {
    const n = block[i];
    if (isDirective(n) && n.name === name) out.push({ idx: i, d: n });
  }
  return out;
}

export function removeAll(
  block: NginxNode[],
  pred: (d: Directive) => boolean,
): NginxNode[] {
  return block.filter((n) => !(isDirective(n) && pred(n)));
}

// Place new directives before the first nested block (location/if/server)
// so toggled-on directives sit with their peers, not below all the blocks.
export function insertSimple(block: NginxNode[], d: Directive): NginxNode[] {
  for (let i = 0; i < block.length; i++) {
    const n = block[i];
    if (isDirective(n) && (n.block !== null || n.opaqueBody !== null)) {
      return [...block.slice(0, i), d, ...block.slice(i)];
    }
  }
  return [...block, d];
}

export function upsert(
  block: NginxNode[],
  name: string,
  args: string[],
): NginxNode[] {
  const found = findFirst(block, name);
  if (found) {
    const copy = block.slice();
    copy[found.idx] = { ...found.d, args };
    return copy;
  }
  return insertSimple(block, newDirective(name, args));
}

export function findAddHeader(
  block: NginxNode[],
  headerName: string,
): { idx: number; d: Directive } | null {
  const lower = headerName.toLowerCase();
  for (let i = 0; i < block.length; i++) {
    const n = block[i];
    if (
      isDirective(n) &&
      n.name === "add_header" &&
      (n.args[0] ?? "").toLowerCase() === lower
    ) {
      return { idx: i, d: n };
    }
  }
  return null;
}

export function upsertAddHeader(
  block: NginxNode[],
  headerName: string,
  value: string,
): NginxNode[] {
  const args = [headerName, value, "always"];
  const found = findAddHeader(block, headerName);
  if (found) {
    const copy = block.slice();
    copy[found.idx] = { ...found.d, args };
    return copy;
  }
  return insertSimple(block, newDirective("add_header", args));
}

export function removeAddHeader(
  block: NginxNode[],
  headerName: string,
): NginxNode[] {
  const lower = headerName.toLowerCase();
  return removeAll(
    block,
    (d) =>
      d.name === "add_header" && (d.args[0] ?? "").toLowerCase() === lower,
  );
}

export function unquote(s: string): string {
  if (s.length >= 2) {
    const f = s[0];
    const l = s[s.length - 1];
    if ((f === '"' && l === '"') || (f === "'" && l === "'")) {
      return s.slice(1, -1).replace(/\\(["\\])/g, "$1");
    }
  }
  return s;
}

type ListenInfo = { bind: string; flags: Set<string> };

export function parseListen(d: Directive): ListenInfo {
  const args = d.args.slice();
  const bind = args.shift() ?? "80";
  return { bind, flags: new Set(args.map((a) => a.toLowerCase())) };
}

export function hasListenFlag(block: NginxNode[], flag: string): boolean {
  for (const n of block) {
    if (
      isDirective(n) &&
      n.name === "listen" &&
      parseListen(n).flags.has(flag)
    ) {
      return true;
    }
  }
  return false;
}

export function setListenFlag(
  block: NginxNode[],
  flag: string,
  on: boolean,
  pred: (info: ListenInfo) => boolean,
): NginxNode[] {
  return block.map((n) => {
    if (!isDirective(n) || n.name !== "listen") return n;
    const info = parseListen(n);
    if (!pred(info)) return n;
    const isOn = info.flags.has(flag);
    if (on === isOn) return n;
    if (on) return { ...n, args: [...n.args, flag] };
    return {
      ...n,
      args: n.args.filter((a, i) => i === 0 || a.toLowerCase() !== flag),
    };
  });
}

const isHttps = (b: NginxNode[]) =>
  hasListenFlag(b, "ssl") || !!findFirst(b, "ssl_certificate");

// ── Scope discovery ──────────────────────────────────────────────

export type Scope = {
  kind: ScopeKind;
  /** Path into nodes to the directive whose `.block` is this scope.
   *  Empty array means the top-level array itself. */
  path: number[];
  label: string;
};

function serverLabel(d: Directive): string {
  if (!d.block) return "server";
  const sn = findFirst(d.block, "server_name");
  if (sn && sn.d.args.length > 0) {
    return `server { server_name ${sn.d.args.slice(0, 3).join(" ")} }`;
  }
  const ls = findFirst(d.block, "listen");
  if (ls) return `server { listen ${ls.d.args.join(" ")} }`;
  return "server { … }";
}

export function collectScopes(nodes: NginxNode[]): Scope[] {
  const out: Scope[] = [{ kind: "main", path: [], label: "Top-level" }];
  function walk(block: NginxNode[], path: number[]) {
    for (let i = 0; i < block.length; i++) {
      const n = block[i];
      if (!isDirective(n) || !n.block) continue;
      const next = [...path, i];
      if (n.name === "http") {
        out.push({ kind: "http", path: next, label: "http { … }" });
      } else if (n.name === "server") {
        out.push({ kind: "server", path: next, label: serverLabel(n) });
      }
      walk(n.block, next);
    }
  }
  walk(nodes, []);
  return out;
}

export function getBlockAt(
  nodes: NginxNode[],
  path: number[],
): NginxNode[] | null {
  let cur: NginxNode[] = nodes;
  for (const i of path) {
    const n = cur[i];
    if (!isDirective(n) || !n.block) return null;
    cur = n.block;
  }
  return cur;
}

export function replaceBlockAt(
  nodes: NginxNode[],
  path: number[],
  newBlock: NginxNode[],
): NginxNode[] {
  if (path.length === 0) return newBlock;
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!isDirective(target) || target.block === null) return nodes;
  const copy = nodes.slice();
  copy[head] =
    rest.length === 0
      ? { ...target, block: newBlock }
      : { ...target, block: replaceBlockAt(target.block, rest, newBlock) };
  return copy;
}

// ── Feature catalog ──────────────────────────────────────────────

export const FEATURES: Feature[] = [
  // ── TLS ──────────────────────────────────────────────────────
  {
    id: "https",
    group: "tls",
    contexts: ["server"],
    title: "HTTPS / TLS",
    description: "Serve over TLS using a certificate + private key.",
    fields: [
      {
        key: "cert",
        type: "text",
        label: "Certificate path",
        placeholder: "/etc/letsencrypt/live/example.com/fullchain.pem",
      },
      {
        key: "key",
        type: "text",
        label: "Private key path",
        placeholder: "/etc/letsencrypt/live/example.com/privkey.pem",
      },
      { key: "port", type: "text", label: "HTTPS port", placeholder: "443" },
    ],
    defaults: {
      cert: "/etc/letsencrypt/live/example.com/fullchain.pem",
      key: "/etc/letsencrypt/live/example.com/privkey.pem",
      port: "443",
    },
    detect: (b) => isHttps(b),
    read: (b) => {
      const cert = unquote(findFirst(b, "ssl_certificate")?.d.args[0] ?? "");
      const key = unquote(findFirst(b, "ssl_certificate_key")?.d.args[0] ?? "");
      let port = "443";
      for (const n of b) {
        if (isDirective(n) && n.name === "listen") {
          const info = parseListen(n);
          if (info.flags.has("ssl")) {
            const m = /:(\d+)$/.exec(info.bind) ?? /^(\d+)$/.exec(info.bind);
            if (m) port = m[1];
          }
        }
      }
      return { cert, key, port };
    },
    enable: (b, v) => {
      const cert = String(v.cert || "");
      const key = String(v.key || "");
      const port = String(v.port || "443");
      let next = b;
      if (hasListenFlag(next, "ssl")) {
        next = next.map((n) => {
          if (!isDirective(n) || n.name !== "listen") return n;
          const info = parseListen(n);
          if (!info.flags.has("ssl")) return n;
          const newBind = info.bind.includes(":")
            ? info.bind.replace(/(\d+)$/, port)
            : port;
          return { ...n, args: [newBind, ...n.args.slice(1)] };
        });
      } else {
        next = insertSimple(next, newDirective("listen", [port, "ssl"]));
      }
      next = upsert(next, "ssl_certificate", [cert]);
      next = upsert(next, "ssl_certificate_key", [key]);
      return next;
    },
    disable: (b) => {
      let next = setListenFlag(b, "ssl", false, () => true);
      next = setListenFlag(next, "http2", false, () => true);
      next = removeAll(
        next,
        (d) =>
          d.name === "ssl_certificate" || d.name === "ssl_certificate_key",
      );
      return next;
    },
  },
  {
    id: "http2",
    group: "tls",
    contexts: ["server"],
    title: "HTTP/2",
    description: "Add the http2 flag to the SSL listen line.",
    fields: [],
    defaults: {},
    detect: (b) => hasListenFlag(b, "http2"),
    read: () => ({}),
    enable: (b) =>
      setListenFlag(b, "http2", true, (info) => info.flags.has("ssl")),
    disable: (b) => setListenFlag(b, "http2", false, () => true),
    requires: (b) => (hasListenFlag(b, "ssl") ? null : "Enable HTTPS first."),
  },
  {
    id: "ssl-protocols",
    group: "tls",
    contexts: ["http", "server"],
    title: "TLS protocols & ciphers",
    description:
      "Pick which TLS versions and cipher suites the server will negotiate.",
    fields: [
      {
        key: "protocols",
        type: "text",
        label: "ssl_protocols",
        placeholder: "TLSv1.2 TLSv1.3",
      },
      {
        key: "ciphers",
        type: "text",
        label: "ssl_ciphers",
        placeholder: "HIGH:!aNULL:!MD5",
      },
      { key: "preferServer", type: "switch", label: "Prefer server ciphers" },
    ],
    defaults: {
      protocols: "TLSv1.2 TLSv1.3",
      ciphers: "HIGH:!aNULL:!MD5",
      preferServer: true,
    },
    detect: (b) => !!findFirst(b, "ssl_protocols"),
    read: (b) => {
      const p = findFirst(b, "ssl_protocols");
      const c = findFirst(b, "ssl_ciphers");
      const pref = findFirst(b, "ssl_prefer_server_ciphers");
      return {
        protocols: p ? p.d.args.join(" ") : "TLSv1.2 TLSv1.3",
        ciphers: c ? unquote(c.d.args[0] ?? "") : "HIGH:!aNULL:!MD5",
        preferServer:
          !!pref && (pref.d.args[0] ?? "").toLowerCase() === "on",
      };
    },
    enable: (b, v) => {
      let next = upsert(
        b,
        "ssl_protocols",
        String(v.protocols || "TLSv1.2 TLSv1.3").trim().split(/\s+/),
      );
      next = upsert(next, "ssl_ciphers", [
        String(v.ciphers || "HIGH:!aNULL:!MD5"),
      ]);
      next = upsert(next, "ssl_prefer_server_ciphers", [
        v.preferServer ? "on" : "off",
      ]);
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "ssl_protocols" ||
          d.name === "ssl_ciphers" ||
          d.name === "ssl_prefer_server_ciphers",
      ),
    requires: (b) => (isHttps(b) ? null : "Enable HTTPS first."),
  },
  {
    id: "ssl-session",
    group: "tls",
    contexts: ["http", "server"],
    title: "TLS session cache",
    description:
      "Reuse TLS handshakes across connections to cut per-connect cost.",
    fields: [
      {
        key: "cache",
        type: "text",
        label: "ssl_session_cache",
        placeholder: "shared:SSL:10m",
      },
      {
        key: "timeout",
        type: "text",
        label: "ssl_session_timeout",
        placeholder: "1d",
      },
    ],
    defaults: { cache: "shared:SSL:10m", timeout: "1d" },
    detect: (b) => !!findFirst(b, "ssl_session_cache"),
    read: (b) => ({
      cache:
        findFirst(b, "ssl_session_cache")?.d.args.join(" ") ?? "shared:SSL:10m",
      timeout: findFirst(b, "ssl_session_timeout")?.d.args[0] ?? "1d",
    }),
    enable: (b, v) => {
      let next = upsert(
        b,
        "ssl_session_cache",
        String(v.cache || "shared:SSL:10m").split(/\s+/),
      );
      next = upsert(next, "ssl_session_timeout", [String(v.timeout || "1d")]);
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "ssl_session_cache" || d.name === "ssl_session_timeout",
      ),
    requires: (b) => (isHttps(b) ? null : "Enable HTTPS first."),
  },
  {
    id: "ocsp-stapling",
    group: "tls",
    contexts: ["http", "server"],
    title: "OCSP stapling",
    description:
      "Server fetches certificate revocation status so clients don't have to.",
    fields: [],
    defaults: {},
    detect: (b) => {
      const f = findFirst(b, "ssl_stapling");
      return !!f && (f.d.args[0] ?? "").toLowerCase() === "on";
    },
    read: () => ({}),
    enable: (b) => {
      let next = upsert(b, "ssl_stapling", ["on"]);
      next = upsert(next, "ssl_stapling_verify", ["on"]);
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "ssl_stapling" || d.name === "ssl_stapling_verify",
      ),
    requires: (b) => (isHttps(b) ? null : "Enable HTTPS first."),
  },

  // ── Security headers ────────────────────────────────────────
  {
    id: "hsts",
    group: "security",
    contexts: ["http", "server"],
    title: "HSTS (Strict-Transport-Security)",
    description: "Force browsers to use HTTPS for the configured duration.",
    fields: [
      {
        key: "maxAge",
        type: "text",
        label: "max-age (seconds)",
        placeholder: "63072000",
      },
      { key: "subdomains", type: "switch", label: "includeSubDomains" },
      { key: "preload", type: "switch", label: "preload" },
    ],
    defaults: { maxAge: "63072000", subdomains: true, preload: false },
    detect: (b) => !!findAddHeader(b, "Strict-Transport-Security"),
    read: (b) => {
      const found = findAddHeader(b, "Strict-Transport-Security");
      if (!found)
        return { maxAge: "63072000", subdomains: true, preload: false };
      const v = unquote(found.d.args[1] ?? "");
      const m = /max-age=(\d+)/.exec(v);
      return {
        maxAge: m ? m[1] : "63072000",
        subdomains: /includeSubDomains/i.test(v),
        preload: /preload/i.test(v),
      };
    },
    enable: (b, v) => {
      const parts = [`max-age=${String(v.maxAge || "63072000")}`];
      if (v.subdomains) parts.push("includeSubDomains");
      if (v.preload) parts.push("preload");
      return upsertAddHeader(b, "Strict-Transport-Security", parts.join("; "));
    },
    disable: (b) => removeAddHeader(b, "Strict-Transport-Security"),
    requires: (b) => (isHttps(b) ? null : "Enable HTTPS first."),
  },
  {
    id: "x-frame-options",
    group: "security",
    contexts: ["http", "server"],
    title: "X-Frame-Options: SAMEORIGIN",
    description:
      "Disallow rendering the site inside an iframe on a different origin.",
    fields: [],
    defaults: {},
    detect: (b) => !!findAddHeader(b, "X-Frame-Options"),
    read: () => ({}),
    enable: (b) => upsertAddHeader(b, "X-Frame-Options", "SAMEORIGIN"),
    disable: (b) => removeAddHeader(b, "X-Frame-Options"),
  },
  {
    id: "x-content-type-options",
    group: "security",
    contexts: ["http", "server"],
    title: "X-Content-Type-Options: nosniff",
    description:
      "Tell browsers not to MIME-sniff away from the declared Content-Type.",
    fields: [],
    defaults: {},
    detect: (b) => !!findAddHeader(b, "X-Content-Type-Options"),
    read: () => ({}),
    enable: (b) => upsertAddHeader(b, "X-Content-Type-Options", "nosniff"),
    disable: (b) => removeAddHeader(b, "X-Content-Type-Options"),
  },
  {
    id: "referrer-policy",
    group: "security",
    contexts: ["http", "server"],
    title: "Referrer-Policy",
    description: "Control how much of the URL is leaked in the Referer header.",
    fields: [
      {
        key: "value",
        type: "select",
        label: "Policy",
        options: [
          { value: "no-referrer", label: "no-referrer" },
          { value: "no-referrer-when-downgrade", label: "no-referrer-when-downgrade" },
          { value: "same-origin", label: "same-origin" },
          { value: "origin", label: "origin" },
          { value: "strict-origin", label: "strict-origin" },
          {
            value: "strict-origin-when-cross-origin",
            label: "strict-origin-when-cross-origin",
          },
          { value: "unsafe-url", label: "unsafe-url" },
        ],
      },
    ],
    defaults: { value: "strict-origin-when-cross-origin" },
    detect: (b) => !!findAddHeader(b, "Referrer-Policy"),
    read: (b) => {
      const found = findAddHeader(b, "Referrer-Policy");
      return {
        value: found
          ? unquote(found.d.args[1] ?? "")
          : "strict-origin-when-cross-origin",
      };
    },
    enable: (b, v) =>
      upsertAddHeader(
        b,
        "Referrer-Policy",
        String(v.value || "strict-origin-when-cross-origin"),
      ),
    disable: (b) => removeAddHeader(b, "Referrer-Policy"),
  },
  {
    id: "csp",
    group: "security",
    contexts: ["http", "server"],
    title: "Content-Security-Policy",
    description:
      "Whitelist origins for scripts, styles, images, etc. — strong XSS mitigation.",
    fields: [
      {
        key: "policy",
        type: "textarea",
        label: "Policy",
        placeholder: "default-src 'self'; img-src 'self' data:; …",
        rows: 3,
      },
    ],
    defaults: { policy: "default-src 'self'" },
    detect: (b) => !!findAddHeader(b, "Content-Security-Policy"),
    read: (b) => {
      const found = findAddHeader(b, "Content-Security-Policy");
      return {
        policy: found ? unquote(found.d.args[1] ?? "") : "default-src 'self'",
      };
    },
    enable: (b, v) =>
      upsertAddHeader(
        b,
        "Content-Security-Policy",
        String(v.policy || "default-src 'self'"),
      ),
    disable: (b) => removeAddHeader(b, "Content-Security-Policy"),
  },
  {
    id: "permissions-policy",
    group: "security",
    contexts: ["http", "server"],
    title: "Permissions-Policy",
    description:
      "Restrict which browser features (camera, geolocation, …) the page can use.",
    fields: [
      {
        key: "policy",
        type: "text",
        label: "Policy",
        placeholder: "camera=(), microphone=(), geolocation=()",
      },
    ],
    defaults: { policy: "camera=(), microphone=(), geolocation=()" },
    detect: (b) => !!findAddHeader(b, "Permissions-Policy"),
    read: (b) => {
      const found = findAddHeader(b, "Permissions-Policy");
      return {
        policy: found
          ? unquote(found.d.args[1] ?? "")
          : "camera=(), microphone=(), geolocation=()",
      };
    },
    enable: (b, v) =>
      upsertAddHeader(b, "Permissions-Policy", String(v.policy || "")),
    disable: (b) => removeAddHeader(b, "Permissions-Policy"),
  },
  {
    id: "server-tokens-off",
    group: "security",
    contexts: ["main", "http", "server"],
    title: "Hide nginx version",
    description:
      "Set server_tokens off so error pages and headers don't leak the version string.",
    fields: [],
    defaults: {},
    detect: (b) => {
      const f = findFirst(b, "server_tokens");
      return !!f && (f.d.args[0] ?? "").toLowerCase() === "off";
    },
    read: () => ({}),
    enable: (b) => upsert(b, "server_tokens", ["off"]),
    disable: (b) => removeAll(b, (d) => d.name === "server_tokens"),
  },

  // ── Performance ────────────────────────────────────────────
  {
    id: "client-max-body-size",
    group: "performance",
    contexts: ["http", "server"],
    title: "Upload size limit",
    description:
      "Maximum size of the client request body (file uploads, JSON POST, …).",
    fields: [
      {
        key: "size",
        type: "text",
        label: "client_max_body_size",
        placeholder: "10m",
      },
    ],
    defaults: { size: "10m" },
    detect: (b) => !!findFirst(b, "client_max_body_size"),
    read: (b) => ({
      size: findFirst(b, "client_max_body_size")?.d.args[0] ?? "10m",
    }),
    enable: (b, v) =>
      upsert(b, "client_max_body_size", [String(v.size || "10m")]),
    disable: (b) =>
      removeAll(b, (d) => d.name === "client_max_body_size"),
  },
  {
    id: "keepalive",
    group: "performance",
    contexts: ["http", "server"],
    title: "Keep-alive",
    description:
      "How long and how many requests an idle TCP connection is reused.",
    fields: [
      {
        key: "timeout",
        type: "text",
        label: "keepalive_timeout",
        placeholder: "65",
      },
      {
        key: "requests",
        type: "text",
        label: "keepalive_requests",
        placeholder: "100",
      },
    ],
    defaults: { timeout: "65", requests: "100" },
    detect: (b) =>
      !!findFirst(b, "keepalive_timeout") ||
      !!findFirst(b, "keepalive_requests"),
    read: (b) => ({
      timeout: findFirst(b, "keepalive_timeout")?.d.args[0] ?? "65",
      requests: findFirst(b, "keepalive_requests")?.d.args[0] ?? "100",
    }),
    enable: (b, v) => {
      let next = upsert(b, "keepalive_timeout", [String(v.timeout || "65")]);
      next = upsert(next, "keepalive_requests", [
        String(v.requests || "100"),
      ]);
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "keepalive_timeout" || d.name === "keepalive_requests",
      ),
  },
  {
    id: "sendfile-tcp",
    group: "performance",
    contexts: ["http"],
    title: "sendfile / TCP tuning",
    description:
      "Zero-copy file send and TCP-level batching tweaks for high-throughput setups.",
    fields: [
      { key: "sendfile", type: "switch", label: "sendfile on" },
      { key: "tcp_nopush", type: "switch", label: "tcp_nopush on" },
      { key: "tcp_nodelay", type: "switch", label: "tcp_nodelay on" },
    ],
    defaults: { sendfile: true, tcp_nopush: true, tcp_nodelay: true },
    detect: (b) => {
      const sf = findFirst(b, "sendfile");
      return !!sf && (sf.d.args[0] ?? "").toLowerCase() === "on";
    },
    read: (b) => {
      const on = (name: string) => {
        const f = findFirst(b, name);
        return !!f && (f.d.args[0] ?? "").toLowerCase() === "on";
      };
      return {
        sendfile: on("sendfile"),
        tcp_nopush: on("tcp_nopush"),
        tcp_nodelay: on("tcp_nodelay"),
      };
    },
    enable: (b, v) => {
      let next = b;
      next = v.sendfile
        ? upsert(next, "sendfile", ["on"])
        : removeAll(next, (d) => d.name === "sendfile");
      next = v.tcp_nopush
        ? upsert(next, "tcp_nopush", ["on"])
        : removeAll(next, (d) => d.name === "tcp_nopush");
      next = v.tcp_nodelay
        ? upsert(next, "tcp_nodelay", ["on"])
        : removeAll(next, (d) => d.name === "tcp_nodelay");
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "sendfile" ||
          d.name === "tcp_nopush" ||
          d.name === "tcp_nodelay",
      ),
  },
  {
    id: "gzip",
    group: "performance",
    contexts: ["http", "server"],
    title: "Gzip compression",
    description: "Compress text responses to reduce bandwidth.",
    fields: [
      {
        key: "types",
        type: "text",
        label: "MIME types",
        placeholder: "text/plain text/css application/json …",
      },
      {
        key: "minLength",
        type: "text",
        label: "Min length (bytes)",
        placeholder: "1024",
      },
      {
        key: "compLevel",
        type: "text",
        label: "Compression level (1-9)",
        placeholder: "5",
      },
    ],
    defaults: {
      types:
        "text/plain text/css application/json application/javascript text/xml application/xml",
      minLength: "1024",
      compLevel: "5",
    },
    detect: (b) => {
      const f = findFirst(b, "gzip");
      return !!f && (f.d.args[0] ?? "").toLowerCase() === "on";
    },
    read: (b) => ({
      types:
        findFirst(b, "gzip_types")?.d.args.join(" ") ??
        "text/plain text/css application/json application/javascript text/xml application/xml",
      minLength: findFirst(b, "gzip_min_length")?.d.args[0] ?? "1024",
      compLevel: findFirst(b, "gzip_comp_level")?.d.args[0] ?? "5",
    }),
    enable: (b, v) => {
      let next = upsert(b, "gzip", ["on"]);
      const types = String(v.types || "")
        .trim()
        .split(/\s+/)
        .filter(Boolean);
      if (types.length > 0) next = upsert(next, "gzip_types", types);
      next = upsert(next, "gzip_min_length", [String(v.minLength || "1024")]);
      next = upsert(next, "gzip_comp_level", [String(v.compLevel || "5")]);
      next = upsert(next, "gzip_vary", ["on"]);
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "gzip" ||
          d.name === "gzip_types" ||
          d.name === "gzip_vary" ||
          d.name === "gzip_min_length" ||
          d.name === "gzip_comp_level",
      ),
  },
  {
    id: "open-file-cache",
    group: "performance",
    contexts: ["http", "server"],
    title: "Open file cache",
    description:
      "Cache file descriptors / metadata to skip repeated stat() calls under load.",
    fields: [
      {
        key: "max",
        type: "text",
        label: "max",
        placeholder: "1000",
      },
      {
        key: "inactive",
        type: "text",
        label: "inactive",
        placeholder: "20s",
      },
    ],
    defaults: { max: "1000", inactive: "20s" },
    detect: (b) => {
      const f = findFirst(b, "open_file_cache");
      return !!f && (f.d.args[0] ?? "").toLowerCase() !== "off";
    },
    read: (b) => {
      const f = findFirst(b, "open_file_cache");
      const m = f
        ? f.d.args.find((a) => a.startsWith("max="))?.slice(4) ?? "1000"
        : "1000";
      const inactive = f
        ? f.d.args.find((a) => a.startsWith("inactive="))?.slice(9) ?? "20s"
        : "20s";
      return { max: m, inactive };
    },
    enable: (b, v) =>
      upsert(b, "open_file_cache", [
        `max=${String(v.max || "1000")}`,
        `inactive=${String(v.inactive || "20s")}`,
      ]),
    disable: (b) => removeAll(b, (d) => d.name === "open_file_cache"),
  },
  {
    id: "static-cache",
    group: "performance",
    contexts: ["server"],
    title: "Static asset cache (expires)",
    description:
      "Tell clients to cache responses for the chosen duration. Apply inside a location for finer control.",
    fields: [
      {
        key: "duration",
        type: "text",
        label: "expires",
        placeholder: "30d",
      },
    ],
    defaults: { duration: "30d" },
    detect: (b) => !!findFirst(b, "expires"),
    read: (b) => ({
      duration: findFirst(b, "expires")?.d.args[0] ?? "30d",
    }),
    enable: (b, v) => upsert(b, "expires", [String(v.duration || "30d")]),
    disable: (b) => removeAll(b, (d) => d.name === "expires"),
  },

  // ── Logging ───────────────────────────────────────────────
  {
    id: "access-log",
    group: "logging",
    contexts: ["http", "server"],
    title: "Access log",
    description:
      "Where access requests are written. Toggle off to skip logging entirely.",
    fields: [
      {
        key: "off",
        type: "switch",
        label: "Disable (access_log off)",
      },
      {
        key: "path",
        type: "text",
        label: "Log path",
        placeholder: "/var/log/nginx/access.log",
      },
    ],
    defaults: { off: false, path: "/var/log/nginx/access.log" },
    detect: (b) => !!findFirst(b, "access_log"),
    read: (b) => {
      const f = findFirst(b, "access_log");
      if (!f) return { off: false, path: "/var/log/nginx/access.log" };
      const first = f.d.args[0] ?? "";
      if (first.toLowerCase() === "off")
        return { off: true, path: "/var/log/nginx/access.log" };
      return { off: false, path: first };
    },
    enable: (b, v) =>
      upsert(b, "access_log", [
        v.off ? "off" : String(v.path || "/var/log/nginx/access.log"),
      ]),
    disable: (b) => removeAll(b, (d) => d.name === "access_log"),
  },
  {
    id: "error-log",
    group: "logging",
    contexts: ["main", "http", "server"],
    title: "Error log",
    description: "Where nginx writes its own diagnostics, and at what level.",
    fields: [
      {
        key: "path",
        type: "text",
        label: "Log path",
        placeholder: "/var/log/nginx/error.log",
      },
      {
        key: "level",
        type: "select",
        label: "Level",
        options: [
          { value: "debug", label: "debug" },
          { value: "info", label: "info" },
          { value: "notice", label: "notice" },
          { value: "warn", label: "warn" },
          { value: "error", label: "error" },
          { value: "crit", label: "crit" },
        ],
      },
    ],
    defaults: { path: "/var/log/nginx/error.log", level: "warn" },
    detect: (b) => !!findFirst(b, "error_log"),
    read: (b) => {
      const f = findFirst(b, "error_log");
      if (!f) return { path: "/var/log/nginx/error.log", level: "warn" };
      return {
        path: f.d.args[0] ?? "/var/log/nginx/error.log",
        level: f.d.args[1] ?? "warn",
      };
    },
    enable: (b, v) =>
      upsert(b, "error_log", [
        String(v.path || "/var/log/nginx/error.log"),
        String(v.level || "warn"),
      ]),
    disable: (b) => removeAll(b, (d) => d.name === "error_log"),
  },

  // ── Proxy ────────────────────────────────────────────────
  {
    id: "proxy-timeouts",
    group: "proxy",
    contexts: ["http", "server"],
    title: "Proxy timeouts",
    description:
      "How long to wait when nginx is talking to an upstream (proxy_pass target).",
    fields: [
      {
        key: "connect",
        type: "text",
        label: "proxy_connect_timeout",
        placeholder: "60s",
      },
      {
        key: "read",
        type: "text",
        label: "proxy_read_timeout",
        placeholder: "60s",
      },
      {
        key: "send",
        type: "text",
        label: "proxy_send_timeout",
        placeholder: "60s",
      },
    ],
    defaults: { connect: "60s", read: "60s", send: "60s" },
    detect: (b) =>
      !!findFirst(b, "proxy_connect_timeout") ||
      !!findFirst(b, "proxy_read_timeout") ||
      !!findFirst(b, "proxy_send_timeout"),
    read: (b) => ({
      connect: findFirst(b, "proxy_connect_timeout")?.d.args[0] ?? "60s",
      read: findFirst(b, "proxy_read_timeout")?.d.args[0] ?? "60s",
      send: findFirst(b, "proxy_send_timeout")?.d.args[0] ?? "60s",
    }),
    enable: (b, v) => {
      let next = upsert(b, "proxy_connect_timeout", [
        String(v.connect || "60s"),
      ]);
      next = upsert(next, "proxy_read_timeout", [String(v.read || "60s")]);
      next = upsert(next, "proxy_send_timeout", [String(v.send || "60s")]);
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "proxy_connect_timeout" ||
          d.name === "proxy_read_timeout" ||
          d.name === "proxy_send_timeout",
      ),
  },
  {
    id: "proxy-buffering",
    group: "proxy",
    contexts: ["http", "server"],
    title: "Proxy buffering",
    description:
      "Buffer upstream responses before forwarding to the client. Disable for streaming / SSE.",
    fields: [
      { key: "enabled", type: "switch", label: "proxy_buffering on" },
    ],
    defaults: { enabled: true },
    detect: (b) => !!findFirst(b, "proxy_buffering"),
    read: (b) => {
      const f = findFirst(b, "proxy_buffering");
      return {
        enabled: !f || (f.d.args[0] ?? "").toLowerCase() !== "off",
      };
    },
    enable: (b, v) =>
      upsert(b, "proxy_buffering", [v.enabled ? "on" : "off"]),
    disable: (b) => removeAll(b, (d) => d.name === "proxy_buffering"),
  },
  {
    id: "forward-headers",
    group: "proxy",
    contexts: ["http", "server"],
    title: "Forward client headers",
    description:
      "Pass client info to the upstream via X-Real-IP / X-Forwarded-* headers.",
    fields: [
      { key: "host", type: "switch", label: "Host" },
      { key: "realIp", type: "switch", label: "X-Real-IP" },
      { key: "forwardedFor", type: "switch", label: "X-Forwarded-For" },
      { key: "forwardedProto", type: "switch", label: "X-Forwarded-Proto" },
    ],
    defaults: { host: true, realIp: true, forwardedFor: true, forwardedProto: true },
    detect: (b) => !!findFirst(b, "proxy_set_header"),
    read: (b) => {
      const all = findAll(b, "proxy_set_header").map((x) =>
        (x.d.args[0] ?? "").toLowerCase(),
      );
      return {
        host: all.includes("host"),
        realIp: all.includes("x-real-ip"),
        forwardedFor: all.includes("x-forwarded-for"),
        forwardedProto: all.includes("x-forwarded-proto"),
      };
    },
    enable: (b, v) => {
      const KNOWN = new Set([
        "host",
        "x-real-ip",
        "x-forwarded-for",
        "x-forwarded-proto",
      ]);
      let next = removeAll(
        b,
        (d) =>
          d.name === "proxy_set_header" &&
          KNOWN.has((d.args[0] ?? "").toLowerCase()),
      );
      const add = (h: string, val: string) => {
        next = insertSimple(next, newDirective("proxy_set_header", [h, val]));
      };
      if (v.host) add("Host", "$host");
      if (v.realIp) add("X-Real-IP", "$remote_addr");
      if (v.forwardedFor) add("X-Forwarded-For", "$proxy_add_x_forwarded_for");
      if (v.forwardedProto) add("X-Forwarded-Proto", "$scheme");
      return next;
    },
    disable: (b) => {
      const KNOWN = new Set([
        "host",
        "x-real-ip",
        "x-forwarded-for",
        "x-forwarded-proto",
      ]);
      return removeAll(
        b,
        (d) =>
          d.name === "proxy_set_header" &&
          KNOWN.has((d.args[0] ?? "").toLowerCase()),
      );
    },
  },

  // ── Real IP ─────────────────────────────────────────────
  {
    id: "real-ip",
    group: "real-ip",
    contexts: ["http", "server"],
    title: "Trust proxy chain (real IP)",
    description:
      "If nginx sits behind a CDN/LB, recover the client's real IP from a forwarded header.",
    fields: [
      {
        key: "trustedFrom",
        type: "textarea",
        label: "set_real_ip_from (one CIDR per line)",
        placeholder: "10.0.0.0/8\n172.16.0.0/12",
        rows: 3,
      },
      {
        key: "header",
        type: "text",
        label: "real_ip_header",
        placeholder: "X-Forwarded-For",
      },
      { key: "recursive", type: "switch", label: "real_ip_recursive on" },
    ],
    defaults: {
      trustedFrom: "10.0.0.0/8",
      header: "X-Forwarded-For",
      recursive: true,
    },
    detect: (b) => !!findFirst(b, "set_real_ip_from"),
    read: (b) => {
      const cidrs = findAll(b, "set_real_ip_from")
        .map((x) => x.d.args[0] ?? "")
        .filter(Boolean)
        .join("\n");
      const header = findFirst(b, "real_ip_header")?.d.args[0] ?? "X-Forwarded-For";
      const rec = findFirst(b, "real_ip_recursive");
      return {
        trustedFrom: cidrs || "10.0.0.0/8",
        header,
        recursive: !!rec && (rec.d.args[0] ?? "").toLowerCase() === "on",
      };
    },
    enable: (b, v) => {
      let next = removeAll(
        b,
        (d) =>
          d.name === "set_real_ip_from" ||
          d.name === "real_ip_header" ||
          d.name === "real_ip_recursive",
      );
      const cidrs = String(v.trustedFrom || "")
        .split(/\r?\n|\s+/)
        .map((s) => s.trim())
        .filter(Boolean);
      for (const c of cidrs) {
        next = insertSimple(next, newDirective("set_real_ip_from", [c]));
      }
      next = insertSimple(
        next,
        newDirective("real_ip_header", [String(v.header || "X-Forwarded-For")]),
      );
      next = insertSimple(
        next,
        newDirective("real_ip_recursive", [v.recursive ? "on" : "off"]),
      );
      return next;
    },
    disable: (b) =>
      removeAll(
        b,
        (d) =>
          d.name === "set_real_ip_from" ||
          d.name === "real_ip_header" ||
          d.name === "real_ip_recursive",
      ),
  },

  // ── Rate limiting ──────────────────────────────────────
  {
    id: "rate-limit-zone",
    group: "rate-limit",
    contexts: ["http"],
    title: "Rate limit zone",
    description:
      "Define a shared memory zone for rate limiting. Apply per location with `limit_req`.",
    fields: [
      { key: "name", type: "text", label: "Zone name", placeholder: "one" },
      {
        key: "key",
        type: "text",
        label: "Key",
        placeholder: "$binary_remote_addr",
      },
      { key: "size", type: "text", label: "Size", placeholder: "10m" },
      { key: "rate", type: "text", label: "Rate", placeholder: "10r/s" },
    ],
    defaults: {
      name: "one",
      key: "$binary_remote_addr",
      size: "10m",
      rate: "10r/s",
    },
    detect: (b) => !!findFirst(b, "limit_req_zone"),
    read: (b) => {
      const f = findFirst(b, "limit_req_zone");
      if (!f)
        return {
          name: "one",
          key: "$binary_remote_addr",
          size: "10m",
          rate: "10r/s",
        };
      const args = f.d.args;
      const key = args[0] ?? "$binary_remote_addr";
      const zoneArg = args.find((a) => a.startsWith("zone=")) ?? "zone=one:10m";
      const rateArg = args.find((a) => a.startsWith("rate=")) ?? "rate=10r/s";
      const m = /^zone=([^:]+):(.+)$/.exec(zoneArg);
      return {
        name: m ? m[1] : "one",
        key,
        size: m ? m[2] : "10m",
        rate: rateArg.slice(5),
      };
    },
    enable: (b, v) =>
      upsert(b, "limit_req_zone", [
        String(v.key || "$binary_remote_addr"),
        `zone=${String(v.name || "one")}:${String(v.size || "10m")}`,
        `rate=${String(v.rate || "10r/s")}`,
      ]),
    disable: (b) => removeAll(b, (d) => d.name === "limit_req_zone"),
  },

  // ── Worker (top-level) ─────────────────────────────────
  {
    id: "worker-processes",
    group: "worker",
    contexts: ["main"],
    title: "Worker processes",
    description:
      "How many worker processes nginx spawns. `auto` matches the CPU core count.",
    fields: [
      {
        key: "count",
        type: "text",
        label: "worker_processes",
        placeholder: "auto",
      },
    ],
    defaults: { count: "auto" },
    detect: (b) => !!findFirst(b, "worker_processes"),
    read: (b) => ({
      count: findFirst(b, "worker_processes")?.d.args[0] ?? "auto",
    }),
    enable: (b, v) =>
      upsert(b, "worker_processes", [String(v.count || "auto")]),
    disable: (b) => removeAll(b, (d) => d.name === "worker_processes"),
  },
  {
    id: "worker-rlimit",
    group: "worker",
    contexts: ["main"],
    title: "Worker file limit",
    description:
      "Raise the per-worker file-descriptor cap (matches your OS ulimit).",
    fields: [
      {
        key: "limit",
        type: "text",
        label: "worker_rlimit_nofile",
        placeholder: "65535",
      },
    ],
    defaults: { limit: "65535" },
    detect: (b) => !!findFirst(b, "worker_rlimit_nofile"),
    read: (b) => ({
      limit: findFirst(b, "worker_rlimit_nofile")?.d.args[0] ?? "65535",
    }),
    enable: (b, v) =>
      upsert(b, "worker_rlimit_nofile", [String(v.limit || "65535")]),
    disable: (b) =>
      removeAll(b, (d) => d.name === "worker_rlimit_nofile"),
  },
];

export const GROUP_ORDER: FeatureGroup[] = [
  "tls",
  "security",
  "performance",
  "proxy",
  "real-ip",
  "rate-limit",
  "logging",
  "worker",
];

export const GROUP_TITLES: Record<FeatureGroup, string> = {
  tls: "TLS / HTTPS",
  security: "Security headers",
  performance: "Performance",
  logging: "Logging",
  proxy: "Reverse proxy",
  "real-ip": "Real client IP",
  "rate-limit": "Rate limiting",
  worker: "Worker tuning",
};

// ── Common directive list (for the "Add directive" composer) ────

export const COMMON_DIRECTIVES: { name: string; block: boolean }[] = [
  { name: "listen", block: false },
  { name: "server_name", block: false },
  { name: "root", block: false },
  { name: "index", block: false },
  { name: "include", block: false },
  { name: "return", block: false },
  { name: "rewrite", block: false },
  { name: "try_files", block: false },
  { name: "alias", block: false },
  { name: "client_max_body_size", block: false },
  { name: "client_body_buffer_size", block: false },
  { name: "keepalive_timeout", block: false },
  { name: "keepalive_requests", block: false },
  { name: "sendfile", block: false },
  { name: "tcp_nopush", block: false },
  { name: "tcp_nodelay", block: false },
  { name: "gzip", block: false },
  { name: "gzip_types", block: false },
  { name: "gzip_min_length", block: false },
  { name: "gzip_comp_level", block: false },
  { name: "gzip_vary", block: false },
  { name: "ssl_certificate", block: false },
  { name: "ssl_certificate_key", block: false },
  { name: "ssl_protocols", block: false },
  { name: "ssl_ciphers", block: false },
  { name: "ssl_prefer_server_ciphers", block: false },
  { name: "ssl_session_cache", block: false },
  { name: "ssl_session_timeout", block: false },
  { name: "ssl_stapling", block: false },
  { name: "ssl_stapling_verify", block: false },
  { name: "add_header", block: false },
  { name: "expires", block: false },
  { name: "access_log", block: false },
  { name: "error_log", block: false },
  { name: "log_format", block: false },
  { name: "proxy_pass", block: false },
  { name: "proxy_set_header", block: false },
  { name: "proxy_buffering", block: false },
  { name: "proxy_buffers", block: false },
  { name: "proxy_buffer_size", block: false },
  { name: "proxy_connect_timeout", block: false },
  { name: "proxy_read_timeout", block: false },
  { name: "proxy_send_timeout", block: false },
  { name: "proxy_http_version", block: false },
  { name: "fastcgi_pass", block: false },
  { name: "fastcgi_param", block: false },
  { name: "set_real_ip_from", block: false },
  { name: "real_ip_header", block: false },
  { name: "real_ip_recursive", block: false },
  { name: "limit_req", block: false },
  { name: "limit_req_zone", block: false },
  { name: "limit_conn", block: false },
  { name: "limit_conn_zone", block: false },
  { name: "deny", block: false },
  { name: "allow", block: false },
  { name: "auth_basic", block: false },
  { name: "auth_basic_user_file", block: false },
  { name: "server_tokens", block: false },
  { name: "worker_processes", block: false },
  { name: "worker_connections", block: false },
  { name: "worker_rlimit_nofile", block: false },
  { name: "user", block: false },
  { name: "pid", block: false },
  { name: "events", block: true },
  { name: "http", block: true },
  { name: "server", block: true },
  { name: "location", block: true },
  { name: "upstream", block: true },
  { name: "if", block: true },
  { name: "map", block: true },
  { name: "geo", block: true },
  { name: "split_clients", block: true },
  { name: "limit_except", block: true },
  { name: "types", block: true },
];
