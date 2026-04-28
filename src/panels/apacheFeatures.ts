// Apache feature catalog — AST helpers and feature definitions.
//
// Apache config has one dominant scope: inside a `<VirtualHost>` block.
// A few directives also work at the top level (`ServerTokens`, etc.)
// but the panel focuses on per-vhost features, since that's where
// 90% of edits happen. Top-level vhost-less config can be edited in
// raw mode.
//
// Mirrors the shape of `caddyFeatures.ts`.

import type { ApacheNode } from "../lib/commands";

export type Directive = Extract<ApacheNode, { kind: "directive" }>;
export type FieldValue = string | boolean;
export type FieldValues = Record<string, FieldValue>;

export type FieldDef =
  | { type: "text"; key: string; label: string; placeholder?: string }
  | {
      type: "select";
      key: string;
      label: string;
      options: { value: string; label: string }[];
    }
  | { type: "switch"; key: string; label: string };

export type ScopeKind = "main" | "vhost";

export type Scope = {
  kind: ScopeKind;
  /** Path into the top-level node array. Empty path = top-level. */
  path: number[];
  label: string;
};

export type Feature = {
  id: string;
  title: string;
  description: string;
  contexts: ScopeKind[];
  fields: FieldDef[];
  defaults: FieldValues;
  detect: (block: ApacheNode[]) => boolean;
  read: (block: ApacheNode[]) => FieldValues;
  enable: (block: ApacheNode[], values: FieldValues) => ApacheNode[];
  disable: (block: ApacheNode[]) => ApacheNode[];
};

// ── AST helpers ─────────────────────────────────────────────────────

export function isDirective(n: ApacheNode): n is Directive {
  return n.kind === "directive";
}

export function newDirective(name: string, args: string[]): Directive {
  return {
    kind: "directive",
    name,
    args,
    leadingComments: [],
    leadingBlanks: 0,
    inlineComment: null,
    section: null,
  };
}

export function newSection(
  name: string,
  args: string[],
  body: ApacheNode[],
): Directive {
  return {
    kind: "directive",
    name,
    args,
    leadingComments: [],
    leadingBlanks: 0,
    inlineComment: null,
    section: body,
  };
}

export function findFirst(
  block: ApacheNode[],
  name: string,
): Directive | null {
  for (const n of block) {
    if (isDirective(n) && n.name.toLowerCase() === name.toLowerCase()) return n;
  }
  return null;
}

export function findAll(block: ApacheNode[], name: string): Directive[] {
  return block.filter(
    (n): n is Directive =>
      isDirective(n) && n.name.toLowerCase() === name.toLowerCase(),
  );
}

export function removeAll(block: ApacheNode[], name: string): ApacheNode[] {
  return block.filter(
    (n) => !(isDirective(n) && n.name.toLowerCase() === name.toLowerCase()),
  );
}

/** Insert directive before the first nested section in `block`, or
 *  append at the end if there are none. Keeps directives grouped
 *  above sub-sections — matches typical Apache style. */
export function insertSimple(
  block: ApacheNode[],
  d: Directive,
): ApacheNode[] {
  const idx = block.findIndex(
    (n) => isDirective(n) && n.section !== null,
  );
  const out = block.slice();
  if (idx < 0) {
    out.push(d);
  } else {
    out.splice(idx, 0, d);
  }
  return out;
}

export function upsert(
  block: ApacheNode[],
  d: Directive,
): ApacheNode[] {
  const idx = block.findIndex(
    (n) => isDirective(n) && n.name.toLowerCase() === d.name.toLowerCase(),
  );
  if (idx < 0) return insertSimple(block, d);
  const out = block.slice();
  const prev = out[idx] as Directive;
  out[idx] = {
    ...d,
    leadingComments: prev.leadingComments,
    leadingBlanks: prev.leadingBlanks,
  };
  return out;
}

// ── Scope discovery ─────────────────────────────────────────────────

/** Walk top-level looking for `<VirtualHost>` sections. Each vhost
 *  is one scope. `<IfModule>` / `<IfDefine>` wrappers are crawled
 *  through so vhosts inside conditional blocks still surface. */
export function collectScopes(nodes: ApacheNode[]): Scope[] {
  const out: Scope[] = [];
  // Always include the main file itself as a scope (for top-level
  // server-wide directives like ServerName, Listen, etc.).
  out.push({ kind: "main", path: [], label: "(top-level)" });

  function walk(block: ApacheNode[], path: number[]) {
    for (let i = 0; i < block.length; i++) {
      const n = block[i];
      if (!isDirective(n) || n.section === null) continue;
      const lname = n.name.toLowerCase();
      if (lname === "virtualhost") {
        const addr = n.args.join(" ");
        out.push({
          kind: "vhost",
          path: [...path, i],
          label: `<VirtualHost ${addr}>`,
        });
      } else if (lname === "ifmodule" || lname === "ifdefine") {
        walk(n.section, [...path, i]);
      }
    }
  }
  walk(nodes, []);
  return out;
}

export function getBlockAt(
  nodes: ApacheNode[],
  path: number[],
): ApacheNode[] | null {
  if (path.length === 0) return nodes;
  let cur: ApacheNode[] = nodes;
  for (const idx of path) {
    const node = cur[idx];
    if (!node || !isDirective(node) || node.section === null) return null;
    cur = node.section;
  }
  return cur;
}

export function replaceBlockAt(
  nodes: ApacheNode[],
  path: number[],
  newBlock: ApacheNode[],
): ApacheNode[] {
  if (path.length === 0) return newBlock;
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!target || !isDirective(target) || target.section === null) {
    return nodes;
  }
  const replaced =
    rest.length === 0
      ? newBlock
      : replaceBlockAt(target.section, rest, newBlock);
  const out = nodes.slice();
  out[head] = { ...target, section: replaced };
  return out;
}

// ── Quote helpers ───────────────────────────────────────────────────

export function unquote(s: string): string {
  if (s.length < 2) return s;
  if (s[0] === '"' && s[s.length - 1] === '"') {
    return s
      .slice(1, -1)
      .replace(/\\"/g, '"')
      .replace(/\\\\/g, "\\");
  }
  return s;
}

export function quoteArgIfNeeded(s: string): string {
  if (s === "") return '""';
  if (/[\s"#]/.test(s)) {
    return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  }
  return s;
}

// ── Feature catalog ─────────────────────────────────────────────────

export type FeatureGroup =
  | "identity"
  | "network"
  | "tls"
  | "proxy"
  | "alias"
  | "rewrite"
  | "headers"
  | "auth"
  | "directory"
  | "performance"
  | "tuning"
  | "logging";

export const GROUP_ORDER: FeatureGroup[] = [
  "identity",
  "network",
  "tls",
  "proxy",
  "alias",
  "rewrite",
  "headers",
  "auth",
  "directory",
  "performance",
  "tuning",
  "logging",
];

export const GROUP_TITLES: Record<FeatureGroup, string> = {
  identity: "Server identity",
  network: "Listen ports",
  tls: "TLS / HTTPS",
  proxy: "Reverse proxy",
  alias: "Aliases",
  rewrite: "URL rewriting",
  headers: "Security headers",
  auth: "Authentication",
  directory: "Directory access",
  performance: "Performance",
  tuning: "Connection tuning",
  logging: "Logging",
};

// ── Individual features ─────────────────────────────────────────────

const serverIdentity: Feature & { group: FeatureGroup } = {
  id: "server-identity",
  group: "identity",
  title: "ServerName + DocumentRoot",
  description:
    "The hostname this vhost answers for and the directory it serves.",
  contexts: ["vhost"],
  fields: [
    {
      type: "text",
      key: "serverName",
      label: "ServerName",
      placeholder: "example.com",
    },
    {
      type: "text",
      key: "serverAlias",
      label: "ServerAlias (space-separated, optional)",
      placeholder: "www.example.com",
    },
    {
      type: "text",
      key: "documentRoot",
      label: "DocumentRoot",
      placeholder: "/var/www/html",
    },
  ],
  defaults: { serverName: "", serverAlias: "", documentRoot: "/var/www/html" },
  detect: (b) => findFirst(b, "ServerName") !== null,
  read: (b) => {
    const sn = findFirst(b, "ServerName");
    const sa = findAll(b, "ServerAlias")
      .flatMap((d) => d.args.map(unquote))
      .join(" ");
    const dr = findFirst(b, "DocumentRoot");
    return {
      serverName: sn?.args[0] ? unquote(sn.args[0]) : "",
      serverAlias: sa,
      documentRoot: dr?.args[0] ? unquote(dr.args[0]) : "",
    };
  },
  enable: (b, v) => {
    const sn = String(v.serverName ?? "").trim();
    const sa = String(v.serverAlias ?? "").trim();
    const dr = String(v.documentRoot ?? "").trim();
    let next = b;
    if (sn) {
      next = upsert(next, newDirective("ServerName", [quoteArgIfNeeded(sn)]));
    } else {
      next = removeAll(next, "ServerName");
    }
    next = removeAll(next, "ServerAlias");
    if (sa) {
      next = insertSimple(
        next,
        newDirective(
          "ServerAlias",
          sa.split(/\s+/).filter(Boolean).map(quoteArgIfNeeded),
        ),
      );
    }
    if (dr) {
      next = upsert(
        next,
        newDirective("DocumentRoot", [quoteArgIfNeeded(dr)]),
      );
    } else {
      next = removeAll(next, "DocumentRoot");
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "ServerName");
    next = removeAll(next, "ServerAlias");
    next = removeAll(next, "DocumentRoot");
    return next;
  },
};

const ssl: Feature & { group: FeatureGroup } = {
  id: "ssl",
  group: "tls",
  title: "SSL / HTTPS",
  description:
    "Serve over TLS with mod_ssl. Requires `<VirtualHost *:443>` (or similar).",
  contexts: ["vhost"],
  fields: [
    {
      type: "text",
      key: "certFile",
      label: "SSLCertificateFile",
      placeholder: "/etc/letsencrypt/live/example.com/fullchain.pem",
    },
    {
      type: "text",
      key: "keyFile",
      label: "SSLCertificateKeyFile",
      placeholder: "/etc/letsencrypt/live/example.com/privkey.pem",
    },
    {
      type: "text",
      key: "protocols",
      label: "SSLProtocol",
      placeholder: "all -SSLv3 -TLSv1 -TLSv1.1",
    },
  ],
  defaults: {
    certFile: "",
    keyFile: "",
    protocols: "all -SSLv3 -TLSv1 -TLSv1.1",
  },
  detect: (b) => {
    const eng = findFirst(b, "SSLEngine");
    return !!eng && eng.args[0]?.toLowerCase() === "on";
  },
  read: (b) => {
    const cert = findFirst(b, "SSLCertificateFile");
    const key = findFirst(b, "SSLCertificateKeyFile");
    const proto = findFirst(b, "SSLProtocol");
    return {
      certFile: cert?.args[0] ? unquote(cert.args[0]) : "",
      keyFile: key?.args[0] ? unquote(key.args[0]) : "",
      protocols: proto?.args.map(unquote).join(" ") || "",
    };
  },
  enable: (b, v) => {
    const cert = String(v.certFile ?? "").trim();
    const key = String(v.keyFile ?? "").trim();
    const protocols = String(v.protocols ?? "").trim();
    let next = upsert(b, newDirective("SSLEngine", ["on"]));
    if (cert) {
      next = upsert(
        next,
        newDirective("SSLCertificateFile", [quoteArgIfNeeded(cert)]),
      );
    }
    if (key) {
      next = upsert(
        next,
        newDirective("SSLCertificateKeyFile", [quoteArgIfNeeded(key)]),
      );
    }
    if (protocols) {
      next = upsert(
        next,
        newDirective(
          "SSLProtocol",
          protocols.split(/\s+/).filter(Boolean),
        ),
      );
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "SSLEngine");
    next = removeAll(next, "SSLCertificateFile");
    next = removeAll(next, "SSLCertificateKeyFile");
    next = removeAll(next, "SSLProtocol");
    return next;
  },
};

const rewrite: Feature & { group: FeatureGroup } = {
  id: "rewrite",
  group: "rewrite",
  title: "Rewrite engine",
  description: "Enable mod_rewrite. Add rules below; one per line.",
  contexts: ["vhost"],
  fields: [
    {
      type: "text",
      key: "rules",
      label: "RewriteRule (one rule per `;`-separated entry)",
      placeholder: "^/old$ /new [R=301,L]",
    },
  ],
  defaults: { rules: "" },
  detect: (b) => {
    const eng = findFirst(b, "RewriteEngine");
    return !!eng && eng.args[0]?.toLowerCase() === "on";
  },
  read: (b) => {
    const rules = findAll(b, "RewriteRule")
      .map((d) => d.args.map(unquote).join(" "))
      .join("; ");
    return { rules };
  },
  enable: (b, v) => {
    const rules = String(v.rules ?? "")
      .split(";")
      .map((s) => s.trim())
      .filter(Boolean);
    let next = upsert(b, newDirective("RewriteEngine", ["on"]));
    next = removeAll(next, "RewriteRule");
    for (const r of rules) {
      // Simple split: quote every arg that has whitespace.
      const parts = r.match(/\S+/g) ?? [];
      next = insertSimple(
        next,
        newDirective("RewriteRule", parts.map(quoteArgIfNeeded)),
      );
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "RewriteEngine");
    next = removeAll(next, "RewriteRule");
    return next;
  },
};

const headers: Feature & { group: FeatureGroup } = {
  id: "security-headers",
  group: "headers",
  title: "Security headers",
  description:
    "Add X-Frame-Options, X-Content-Type-Options, and HSTS via mod_headers.",
  contexts: ["vhost"],
  fields: [
    { type: "switch", key: "frameOptions", label: "X-Frame-Options: SAMEORIGIN" },
    { type: "switch", key: "noSniff", label: "X-Content-Type-Options: nosniff" },
    { type: "switch", key: "hsts", label: "HSTS (max-age=31536000)" },
  ],
  defaults: { frameOptions: true, noSniff: true, hsts: false },
  detect: (b) => {
    return findAll(b, "Header").some((d) => {
      const head = d.args.map(unquote).join(" ").toLowerCase();
      return (
        head.includes("x-frame-options") ||
        head.includes("x-content-type-options") ||
        head.includes("strict-transport-security")
      );
    });
  },
  read: (b) => {
    let frameOptions = false;
    let noSniff = false;
    let hsts = false;
    for (const d of findAll(b, "Header")) {
      const head = d.args.map(unquote).join(" ").toLowerCase();
      if (head.includes("x-frame-options")) frameOptions = true;
      if (head.includes("x-content-type-options")) noSniff = true;
      if (head.includes("strict-transport-security")) hsts = true;
    }
    return { frameOptions, noSniff, hsts };
  },
  enable: (b, v) => {
    // Strip anything we manage, then re-emit.
    const ours = (d: Directive) => {
      const head = d.args.map(unquote).join(" ").toLowerCase();
      return (
        head.includes("x-frame-options") ||
        head.includes("x-content-type-options") ||
        head.includes("strict-transport-security")
      );
    };
    let next = b.filter(
      (n) => !(isDirective(n) && n.name.toLowerCase() === "header" && ours(n)),
    );
    if (v.frameOptions) {
      next = insertSimple(
        next,
        newDirective("Header", [
          "always",
          "set",
          "X-Frame-Options",
          quoteArgIfNeeded("SAMEORIGIN"),
        ]),
      );
    }
    if (v.noSniff) {
      next = insertSimple(
        next,
        newDirective("Header", [
          "always",
          "set",
          "X-Content-Type-Options",
          quoteArgIfNeeded("nosniff"),
        ]),
      );
    }
    if (v.hsts) {
      next = insertSimple(
        next,
        newDirective("Header", [
          "always",
          "set",
          "Strict-Transport-Security",
          quoteArgIfNeeded("max-age=31536000; includeSubDomains"),
        ]),
      );
    }
    return next;
  },
  disable: (b) => {
    return b.filter((n) => {
      if (!isDirective(n) || n.name.toLowerCase() !== "header") return true;
      const head = n.args.map(unquote).join(" ").toLowerCase();
      return !(
        head.includes("x-frame-options") ||
        head.includes("x-content-type-options") ||
        head.includes("strict-transport-security")
      );
    });
  },
};

const directoryAccess: Feature & { group: FeatureGroup } = {
  id: "directory-access",
  group: "directory",
  title: "Directory access",
  description:
    "Add a `<Directory>` block with the standard `Require all granted` (or denied) policy.",
  contexts: ["vhost"],
  fields: [
    {
      type: "text",
      key: "path",
      label: "Directory path",
      placeholder: "/var/www/html",
    },
    {
      type: "select",
      key: "policy",
      label: "Access policy",
      options: [
        { value: "all granted", label: "Require all granted" },
        { value: "all denied", label: "Require all denied" },
      ],
    },
    { type: "switch", key: "allowOverride", label: "AllowOverride All" },
  ],
  defaults: { path: "/var/www/html", policy: "all granted", allowOverride: true },
  detect: (b) =>
    b.some(
      (n) =>
        isDirective(n) && n.name.toLowerCase() === "directory" && n.section,
    ),
  read: (b) => {
    const dir = b.find(
      (n): n is Directive =>
        isDirective(n) && n.name.toLowerCase() === "directory" && n.section !== null,
    );
    if (!dir || !dir.section) {
      return { path: "", policy: "all granted", allowOverride: false };
    }
    const path = dir.args[0] ? unquote(dir.args[0]) : "";
    const req = findFirst(dir.section, "Require");
    const policy = req ? req.args.map(unquote).join(" ") : "all granted";
    const ao = findFirst(dir.section, "AllowOverride");
    const allowOverride =
      !!ao && ao.args[0]?.toLowerCase() === "all";
    return { path, policy, allowOverride };
  },
  enable: (b, v) => {
    const path = String(v.path ?? "").trim() || "/var/www/html";
    const policy = String(v.policy ?? "all granted");
    const allowOverride = !!v.allowOverride;
    const body: ApacheNode[] = [];
    if (allowOverride) {
      body.push(newDirective("AllowOverride", ["All"]));
    }
    body.push(
      newDirective("Require", policy.split(/\s+/).filter(Boolean)),
    );
    // Replace any existing Directory section that matches this path,
    // otherwise append a new one before nested sub-sections.
    const idx = b.findIndex(
      (n) =>
        isDirective(n) &&
        n.name.toLowerCase() === "directory" &&
        n.args[0] &&
        unquote(n.args[0]) === path,
    );
    if (idx >= 0) {
      const out = b.slice();
      const prev = out[idx] as Directive;
      out[idx] = {
        ...newSection("Directory", [quoteArgIfNeeded(path)], body),
        leadingComments: prev.leadingComments,
        leadingBlanks: prev.leadingBlanks,
      };
      return out;
    }
    return insertSimple(
      b,
      newSection("Directory", [quoteArgIfNeeded(path)], body),
    );
  },
  disable: (b) =>
    b.filter(
      (n) =>
        !(
          isDirective(n) &&
          n.name.toLowerCase() === "directory" &&
          n.section
        ),
    ),
};

const proxyPass: Feature & { group: FeatureGroup } = {
  id: "proxy-pass",
  group: "proxy",
  title: "Reverse proxy (mod_proxy)",
  description:
    "Forward requests under a path prefix to an upstream backend.",
  contexts: ["vhost"],
  fields: [
    { type: "text", key: "path", label: "Local path", placeholder: "/" },
    {
      type: "text",
      key: "upstream",
      label: "Upstream URL",
      placeholder: "http://127.0.0.1:8080/",
    },
    {
      type: "switch",
      key: "preserveHost",
      label: "ProxyPreserveHost on",
    },
  ],
  defaults: { path: "/", upstream: "", preserveHost: true },
  detect: (b) => findFirst(b, "ProxyPass") !== null,
  read: (b) => {
    const pp = findFirst(b, "ProxyPass");
    const ph = findFirst(b, "ProxyPreserveHost");
    const path = pp?.args[0] ? unquote(pp.args[0]) : "/";
    const upstream = pp?.args[1] ? unquote(pp.args[1]) : "";
    const preserveHost =
      !!ph && ph.args[0]?.toLowerCase() === "on";
    return { path, upstream, preserveHost };
  },
  enable: (b, v) => {
    const path = String(v.path ?? "").trim() || "/";
    const upstream = String(v.upstream ?? "").trim();
    if (!upstream) return b;
    let next = removeAll(b, "ProxyPass");
    next = removeAll(next, "ProxyPassReverse");
    next = insertSimple(
      next,
      newDirective("ProxyPass", [
        quoteArgIfNeeded(path),
        quoteArgIfNeeded(upstream),
      ]),
    );
    next = insertSimple(
      next,
      newDirective("ProxyPassReverse", [
        quoteArgIfNeeded(path),
        quoteArgIfNeeded(upstream),
      ]),
    );
    if (v.preserveHost) {
      next = upsert(next, newDirective("ProxyPreserveHost", ["on"]));
    } else {
      next = removeAll(next, "ProxyPreserveHost");
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "ProxyPass");
    next = removeAll(next, "ProxyPassReverse");
    next = removeAll(next, "ProxyPreserveHost");
    return next;
  },
};

const alias: Feature & { group: FeatureGroup } = {
  id: "alias",
  group: "alias",
  title: "Path alias (mod_alias)",
  description:
    "Map a URL path to a local filesystem directory outside DocumentRoot.",
  contexts: ["vhost"],
  fields: [
    {
      type: "text",
      key: "from",
      label: "URL prefix",
      placeholder: "/static",
    },
    {
      type: "text",
      key: "to",
      label: "Local path",
      placeholder: "/var/www/static",
    },
  ],
  defaults: { from: "", to: "" },
  detect: (b) => findFirst(b, "Alias") !== null,
  read: (b) => {
    const a = findFirst(b, "Alias");
    return {
      from: a?.args[0] ? unquote(a.args[0]) : "",
      to: a?.args[1] ? unquote(a.args[1]) : "",
    };
  },
  enable: (b, v) => {
    const from = String(v.from ?? "").trim();
    const to = String(v.to ?? "").trim();
    if (!from || !to) return b;
    return upsert(
      b,
      newDirective("Alias", [quoteArgIfNeeded(from), quoteArgIfNeeded(to)]),
    );
  },
  disable: (b) => removeAll(b, "Alias"),
};

const basicAuth: Feature & { group: FeatureGroup } = {
  id: "basic-auth",
  group: "auth",
  title: "Basic authentication",
  description:
    "Require an htpasswd login at the vhost root. Generates a `<Location />` block.",
  contexts: ["vhost"],
  fields: [
    {
      type: "text",
      key: "realm",
      label: "Realm",
      placeholder: "Restricted",
    },
    {
      type: "text",
      key: "userFile",
      label: "AuthUserFile",
      placeholder: "/etc/apache2/.htpasswd",
    },
    {
      type: "text",
      key: "scope",
      label: "Protect path",
      placeholder: "/",
    },
  ],
  defaults: {
    realm: "Restricted",
    userFile: "/etc/apache2/.htpasswd",
    scope: "/",
  },
  detect: (b) =>
    b.some(
      (n) =>
        isDirective(n) &&
        n.name.toLowerCase() === "location" &&
        n.section !== null &&
        findFirst(n.section, "AuthType")?.args[0]?.toLowerCase() === "basic",
    ),
  read: (b) => {
    const loc = b.find(
      (n): n is Directive =>
        isDirective(n) &&
        n.name.toLowerCase() === "location" &&
        n.section !== null &&
        findFirst(n.section, "AuthType")?.args[0]?.toLowerCase() === "basic",
    );
    if (!loc || !loc.section) {
      return { realm: "", userFile: "", scope: "" };
    }
    const realm = findFirst(loc.section, "AuthName");
    const file = findFirst(loc.section, "AuthUserFile");
    return {
      realm: realm?.args[0] ? unquote(realm.args[0]) : "",
      userFile: file?.args[0] ? unquote(file.args[0]) : "",
      scope: loc.args[0] ? unquote(loc.args[0]) : "/",
    };
  },
  enable: (b, v) => {
    const realm = String(v.realm ?? "").trim() || "Restricted";
    const userFile =
      String(v.userFile ?? "").trim() || "/etc/apache2/.htpasswd";
    const scope = String(v.scope ?? "").trim() || "/";
    const body: ApacheNode[] = [
      newDirective("AuthType", ["Basic"]),
      newDirective("AuthName", [quoteArgIfNeeded(realm)]),
      newDirective("AuthUserFile", [quoteArgIfNeeded(userFile)]),
      newDirective("Require", ["valid-user"]),
    ];
    // Replace the existing auth Location, otherwise insert a new one.
    const idx = b.findIndex(
      (n) =>
        isDirective(n) &&
        n.name.toLowerCase() === "location" &&
        n.section !== null &&
        findFirst(n.section, "AuthType")?.args[0]?.toLowerCase() === "basic",
    );
    const block = newSection("Location", [quoteArgIfNeeded(scope)], body);
    if (idx >= 0) {
      const out = b.slice();
      const prev = out[idx] as Directive;
      out[idx] = {
        ...block,
        leadingComments: prev.leadingComments,
        leadingBlanks: prev.leadingBlanks,
      };
      return out;
    }
    return insertSimple(b, block);
  },
  disable: (b) =>
    b.filter((n) => {
      if (
        !isDirective(n) ||
        n.name.toLowerCase() !== "location" ||
        n.section === null
      ) {
        return true;
      }
      const at = findFirst(n.section, "AuthType");
      return !at || at.args[0]?.toLowerCase() !== "basic";
    }),
};

const logging: Feature & { group: FeatureGroup } = {
  id: "logging",
  group: "logging",
  title: "Access & error logs",
  description:
    "Where this vhost writes its diagnostic and request logs.",
  contexts: ["vhost", "main"],
  fields: [
    {
      type: "text",
      key: "errorLog",
      label: "ErrorLog",
      placeholder: "${APACHE_LOG_DIR}/error.log",
    },
    {
      type: "text",
      key: "accessLog",
      label: "CustomLog path",
      placeholder: "${APACHE_LOG_DIR}/access.log",
    },
    {
      type: "select",
      key: "format",
      label: "Access log format",
      options: [
        { value: "combined", label: "combined" },
        { value: "common", label: "common" },
      ],
    },
  ],
  defaults: {
    errorLog: "",
    accessLog: "",
    format: "combined",
  },
  detect: (b) =>
    findFirst(b, "ErrorLog") !== null || findFirst(b, "CustomLog") !== null,
  read: (b) => {
    const er = findFirst(b, "ErrorLog");
    const cl = findFirst(b, "CustomLog");
    return {
      errorLog: er?.args[0] ? unquote(er.args[0]) : "",
      accessLog: cl?.args[0] ? unquote(cl.args[0]) : "",
      format: cl?.args[1] ? unquote(cl.args[1]) : "combined",
    };
  },
  enable: (b, v) => {
    const errorLog = String(v.errorLog ?? "").trim();
    const accessLog = String(v.accessLog ?? "").trim();
    const format = String(v.format ?? "combined");
    let next = b;
    if (errorLog) {
      next = upsert(
        next,
        newDirective("ErrorLog", [quoteArgIfNeeded(errorLog)]),
      );
    } else {
      next = removeAll(next, "ErrorLog");
    }
    if (accessLog) {
      next = upsert(
        next,
        newDirective("CustomLog", [quoteArgIfNeeded(accessLog), format]),
      );
    } else {
      next = removeAll(next, "CustomLog");
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "ErrorLog");
    next = removeAll(next, "CustomLog");
    return next;
  },
};

const listen: Feature & { group: FeatureGroup } = {
  id: "listen",
  group: "network",
  title: "Listen ports",
  description:
    "Which TCP ports + addresses Apache binds to. Multiple `Listen` directives allowed; one per line.",
  contexts: ["main"],
  fields: [
    {
      type: "text",
      key: "ports",
      label: "Listen entries (`;`-separated)",
      placeholder: "80; 443; *:8080",
    },
  ],
  defaults: { ports: "80" },
  detect: (b) => findFirst(b, "Listen") !== null,
  read: (b) => {
    const ports = findAll(b, "Listen")
      .map((d) => d.args.map(unquote).join(" "))
      .join("; ");
    return { ports };
  },
  enable: (b, v) => {
    const entries = String(v.ports ?? "")
      .split(";")
      .map((s) => s.trim())
      .filter(Boolean);
    let next = removeAll(b, "Listen");
    for (const e of entries) {
      const parts = e.split(/\s+/).map(quoteArgIfNeeded);
      next = insertSimple(next, newDirective("Listen", parts));
    }
    return next;
  },
  disable: (b) => removeAll(b, "Listen"),
};

const connectionTuning: Feature & { group: FeatureGroup } = {
  id: "connection-tuning",
  group: "tuning",
  title: "Connection tuning",
  description:
    "Timeout / KeepAlive / KeepAliveTimeout / MaxKeepAliveRequests. Override defaults per host or per vhost.",
  contexts: ["main", "vhost"],
  fields: [
    { type: "text", key: "timeout", label: "Timeout (s)", placeholder: "60" },
    { type: "switch", key: "keepAlive", label: "KeepAlive on" },
    {
      type: "text",
      key: "keepAliveTimeout",
      label: "KeepAliveTimeout (s)",
      placeholder: "5",
    },
    {
      type: "text",
      key: "maxKeepAliveRequests",
      label: "MaxKeepAliveRequests",
      placeholder: "100",
    },
  ],
  defaults: {
    timeout: "60",
    keepAlive: true,
    keepAliveTimeout: "5",
    maxKeepAliveRequests: "100",
  },
  detect: (b) =>
    findFirst(b, "Timeout") !== null ||
    findFirst(b, "KeepAlive") !== null ||
    findFirst(b, "KeepAliveTimeout") !== null ||
    findFirst(b, "MaxKeepAliveRequests") !== null,
  read: (b) => {
    const t = findFirst(b, "Timeout");
    const ka = findFirst(b, "KeepAlive");
    const kat = findFirst(b, "KeepAliveTimeout");
    const mka = findFirst(b, "MaxKeepAliveRequests");
    return {
      timeout: t?.args[0] ? unquote(t.args[0]) : "",
      keepAlive: !!ka && ka.args[0]?.toLowerCase() === "on",
      keepAliveTimeout: kat?.args[0] ? unquote(kat.args[0]) : "",
      maxKeepAliveRequests: mka?.args[0] ? unquote(mka.args[0]) : "",
    };
  },
  enable: (b, v) => {
    const timeout = String(v.timeout ?? "").trim();
    const keepAlive = !!v.keepAlive;
    const kat = String(v.keepAliveTimeout ?? "").trim();
    const mka = String(v.maxKeepAliveRequests ?? "").trim();
    let next = b;
    if (timeout) {
      next = upsert(next, newDirective("Timeout", [timeout]));
    } else {
      next = removeAll(next, "Timeout");
    }
    next = upsert(
      next,
      newDirective("KeepAlive", [keepAlive ? "on" : "off"]),
    );
    if (kat) {
      next = upsert(next, newDirective("KeepAliveTimeout", [kat]));
    } else {
      next = removeAll(next, "KeepAliveTimeout");
    }
    if (mka) {
      next = upsert(next, newDirective("MaxKeepAliveRequests", [mka]));
    } else {
      next = removeAll(next, "MaxKeepAliveRequests");
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "Timeout");
    next = removeAll(next, "KeepAlive");
    next = removeAll(next, "KeepAliveTimeout");
    next = removeAll(next, "MaxKeepAliveRequests");
    return next;
  },
};

const requestBodyLimit: Feature & { group: FeatureGroup } = {
  id: "request-body-limit",
  group: "tuning",
  title: "Upload size limit",
  description:
    "Maximum size of a client request body in bytes. Common for upload endpoints / big POST bodies.",
  contexts: ["vhost", "main"],
  fields: [
    {
      type: "text",
      key: "bytes",
      label: "LimitRequestBody (bytes; 0 = unlimited)",
      placeholder: "10485760",
    },
  ],
  defaults: { bytes: "10485760" },
  detect: (b) => findFirst(b, "LimitRequestBody") !== null,
  read: (b) => {
    const d = findFirst(b, "LimitRequestBody");
    return { bytes: d?.args[0] ? unquote(d.args[0]) : "" };
  },
  enable: (b, v) => {
    const bytes = String(v.bytes ?? "").trim();
    if (!bytes) return b;
    return upsert(b, newDirective("LimitRequestBody", [bytes]));
  },
  disable: (b) => removeAll(b, "LimitRequestBody"),
};

const deflate: Feature & { group: FeatureGroup } = {
  id: "deflate",
  group: "performance",
  title: "Compression (mod_deflate)",
  description:
    "Compress text-like responses with mod_deflate. Adds `AddOutputFilterByType DEFLATE …` for the listed MIME types.",
  contexts: ["vhost", "main"],
  fields: [
    {
      type: "text",
      key: "types",
      label: "MIME types (space-separated)",
      placeholder:
        "text/html text/css text/plain text/xml application/json application/javascript",
    },
  ],
  defaults: {
    types:
      "text/html text/css text/plain text/xml application/json application/javascript application/xml",
  },
  detect: (b) =>
    findAll(b, "AddOutputFilterByType").some(
      (d) => d.args[0]?.toUpperCase() === "DEFLATE",
    ),
  read: (b) => {
    const lines = findAll(b, "AddOutputFilterByType").filter(
      (d) => d.args[0]?.toUpperCase() === "DEFLATE",
    );
    const types = lines
      .flatMap((d) => d.args.slice(1).map(unquote))
      .join(" ");
    return { types };
  },
  enable: (b, v) => {
    const types = String(v.types ?? "")
      .split(/\s+/)
      .filter(Boolean);
    if (types.length === 0) return b;
    // Strip our existing DEFLATE entries first; preserve other
    // AddOutputFilterByType lines (e.g. for INFLATE chains).
    const next = b.filter(
      (n) =>
        !(
          isDirective(n) &&
          n.name.toLowerCase() === "addoutputfilterbytype" &&
          n.args[0]?.toUpperCase() === "DEFLATE"
        ),
    );
    return insertSimple(
      next,
      newDirective("AddOutputFilterByType", [
        "DEFLATE",
        ...types.map(quoteArgIfNeeded),
      ]),
    );
  },
  disable: (b) =>
    b.filter(
      (n) =>
        !(
          isDirective(n) &&
          n.name.toLowerCase() === "addoutputfilterbytype" &&
          n.args[0]?.toUpperCase() === "DEFLATE"
        ),
    ),
};

export type GroupedFeature = Feature & { group: FeatureGroup };

export const FEATURES: GroupedFeature[] = [
  serverIdentity,
  listen,
  ssl,
  proxyPass,
  alias,
  rewrite,
  headers,
  basicAuth,
  directoryAccess,
  deflate,
  connectionTuning,
  requestBodyLimit,
  logging,
];
