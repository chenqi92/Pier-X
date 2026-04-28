// Caddy feature catalog — AST helpers and feature definitions.
//
// Mirrors the shape of `nginxFeatures.ts`: each feature exposes a
// `detect` predicate, a `read` extractor (returns the form-field
// values from the AST), and `enable` / `disable` mutators. The catalog
// component runs `detect` to decide on/off state, `read` to populate
// fields, and pipes mutations back through `replaceBlockAt`.

import type { CaddyNode } from "../lib/commands";

export type Directive = Extract<CaddyNode, { kind: "directive" }>;
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

export type ScopeKind = "global" | "site" | "snippet";

export type Scope = {
  kind: ScopeKind;
  /** Path into the top-level node array reaching the block whose
   *  contents this scope edits. Empty path = top-level. */
  path: number[];
  /** Display label, e.g. "example.com { … }" or "(common) { … }". */
  label: string;
};

export type Feature = {
  id: string;
  title: string;
  description: string;
  /** Which scope kinds this feature applies to. */
  contexts: ScopeKind[];
  fields: FieldDef[];
  defaults: FieldValues;
  /** Detect whether the feature is currently active in `block`. */
  detect: (block: CaddyNode[]) => boolean;
  /** Pull current field values from `block`. Called only when
   *  `detect` is true. */
  read: (block: CaddyNode[]) => FieldValues;
  /** Apply `values` and return a new block with the feature enabled
   *  / updated. */
  enable: (block: CaddyNode[], values: FieldValues) => CaddyNode[];
  /** Remove all directives this feature owns. */
  disable: (block: CaddyNode[]) => CaddyNode[];
};

// ── AST helpers ─────────────────────────────────────────────────────

export function isDirective(n: CaddyNode): n is Directive {
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
    block: null,
  };
}

export function newBlockDirective(
  name: string,
  args: string[],
  block: CaddyNode[],
): Directive {
  return {
    kind: "directive",
    name,
    args,
    leadingComments: [],
    leadingBlanks: 0,
    inlineComment: null,
    block,
  };
}

export function findFirst(
  block: CaddyNode[],
  name: string,
): Directive | null {
  for (const n of block) {
    if (isDirective(n) && n.name === name) return n;
  }
  return null;
}

export function findAll(block: CaddyNode[], name: string): Directive[] {
  return block.filter(
    (n): n is Directive => isDirective(n) && n.name === name,
  );
}

export function removeAll(block: CaddyNode[], name: string): CaddyNode[] {
  return block.filter((n) => !(isDirective(n) && n.name === name));
}

/** Insert directive before the first nested block in `block`, or
 *  append at the end. Keeps directives grouped above sub-blocks for
 *  readable round-trips. */
export function insertSimple(block: CaddyNode[], d: Directive): CaddyNode[] {
  const idx = block.findIndex(
    (n) => isDirective(n) && n.block !== null,
  );
  const out = block.slice();
  if (idx < 0) {
    out.push(d);
  } else {
    out.splice(idx, 0, d);
  }
  return out;
}

/** If a directive with `name` exists, replace its first occurrence in
 *  place; otherwise insert via `insertSimple`. */
export function upsert(
  block: CaddyNode[],
  d: Directive,
): CaddyNode[] {
  const idx = block.findIndex((n) => isDirective(n) && n.name === d.name);
  if (idx < 0) return insertSimple(block, d);
  const out = block.slice();
  // Preserve halo (leadingComments / leadingBlanks) from the original.
  const prev = out[idx] as Directive;
  out[idx] = {
    ...d,
    leadingComments: prev.leadingComments,
    leadingBlanks: prev.leadingBlanks,
  };
  return out;
}

// ── Scope discovery ─────────────────────────────────────────────────

/** Walk the top-level nodes and emit one Scope per editable context.
 *  Caddy scopes are: the global options block (unnamed `{ ... }`),
 *  every site block (`addr1 addr2 { ... }`), and every snippet
 *  (`(name) { ... }`). Snippet scopes share the `site` editing
 *  surface — most directives that work in a site work in a snippet. */
export function collectScopes(nodes: CaddyNode[]): Scope[] {
  const out: Scope[] = [];
  for (let i = 0; i < nodes.length; i++) {
    const n = nodes[i];
    if (!isDirective(n) || n.block === null) continue;
    if (n.name === "") {
      out.push({
        kind: "global",
        path: [i],
        label: "{ … }",
      });
    } else if (n.name.startsWith("(") && n.name.endsWith(")")) {
      out.push({
        kind: "snippet",
        path: [i],
        label: `${n.name} { … }`,
      });
    } else {
      const addresses = [n.name, ...n.args].join(" ");
      out.push({
        kind: "site",
        path: [i],
        label: `${addresses} { … }`,
      });
    }
  }
  return out;
}

export function getBlockAt(
  nodes: CaddyNode[],
  path: number[],
): CaddyNode[] | null {
  let cur: CaddyNode[] = nodes;
  for (const idx of path) {
    const node = cur[idx];
    if (!node || !isDirective(node) || node.block === null) return null;
    cur = node.block;
  }
  return cur;
}

export function replaceBlockAt(
  nodes: CaddyNode[],
  path: number[],
  newBlock: CaddyNode[],
): CaddyNode[] {
  if (path.length === 0) return newBlock;
  const [head, ...rest] = path;
  const target = nodes[head];
  if (!target || !isDirective(target) || target.block === null) {
    return nodes;
  }
  const replaced =
    rest.length === 0
      ? newBlock
      : replaceBlockAt(target.block, rest, newBlock);
  const out = nodes.slice();
  out[head] = { ...target, block: replaced };
  return out;
}

// ── Quote helpers ───────────────────────────────────────────────────

/** Strip the surrounding quote characters from an arg if it's
 *  enclosed in `"..."` or `` `...` ``. Otherwise return unchanged.
 *  The Rust lexer stores quoted args with the quotes preserved so the
 *  renderer can emit the same style; the form fields want the inner
 *  text. */
export function unquote(s: string): string {
  if (s.length < 2) return s;
  const first = s[0];
  const last = s[s.length - 1];
  if ((first === '"' && last === '"') || (first === "`" && last === "`")) {
    return s.slice(1, -1);
  }
  return s;
}

/** Re-quote a value for a Caddy arg position. We use double-quotes
 *  when the value contains whitespace or special chars. */
export function quoteArgIfNeeded(s: string): string {
  if (s === "") return '""';
  if (/[\s{}#"`\\]/.test(s)) {
    return `"${s.replace(/\\/g, "\\\\").replace(/"/g, '\\"')}"`;
  }
  return s;
}

// ── Feature catalog ─────────────────────────────────────────────────

export type FeatureGroup =
  | "global-options"
  | "tls"
  | "proxy"
  | "php"
  | "static"
  | "performance"
  | "headers"
  | "auth"
  | "routing"
  | "logging";

export const GROUP_ORDER: FeatureGroup[] = [
  "global-options",
  "tls",
  "proxy",
  "php",
  "static",
  "performance",
  "headers",
  "auth",
  "routing",
  "logging",
];

export const GROUP_TITLES: Record<FeatureGroup, string> = {
  "global-options": "Global options",
  tls: "TLS / HTTPS",
  proxy: "Reverse proxy",
  php: "PHP",
  static: "Static files",
  performance: "Performance",
  headers: "Security headers",
  auth: "Authentication",
  routing: "Routing",
  logging: "Logging",
};

// ── Individual features ─────────────────────────────────────────────

const reverseProxy: Feature & { group: FeatureGroup } = {
  id: "reverse-proxy",
  group: "proxy",
  title: "Reverse proxy",
  description: "Forward incoming requests to an upstream backend.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "upstream",
      label: "Upstream",
      placeholder: "127.0.0.1:8080",
    },
  ],
  defaults: { upstream: "127.0.0.1:8080" },
  detect: (b) => findFirst(b, "reverse_proxy") !== null,
  read: (b) => {
    const d = findFirst(b, "reverse_proxy");
    return { upstream: d ? d.args.map(unquote).join(" ") : "" };
  },
  enable: (b, v) => {
    const upstream = String(v.upstream ?? "").trim() || "127.0.0.1:8080";
    return upsert(
      b,
      newDirective(
        "reverse_proxy",
        upstream.split(/\s+/).filter(Boolean).map(quoteArgIfNeeded),
      ),
    );
  },
  disable: (b) => removeAll(b, "reverse_proxy"),
};

const fileServer: Feature & { group: FeatureGroup } = {
  id: "file-server",
  group: "static",
  title: "Static file server",
  description: "Serve files from a directory.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "root",
      label: "Document root",
      placeholder: "/var/www/html",
    },
    { type: "switch", key: "browse", label: "Enable directory browse" },
  ],
  defaults: { root: "/var/www/html", browse: false },
  detect: (b) => findFirst(b, "file_server") !== null,
  read: (b) => {
    const fs = findFirst(b, "file_server");
    const root = findFirst(b, "root");
    const browse = !!fs?.args.includes("browse");
    // `root * /path` — args are ["*", "/path"]. Older form: just one
    // path arg. Take the last arg as the path either way.
    const rootPath =
      root && root.args.length > 0
        ? unquote(root.args[root.args.length - 1])
        : "";
    return { root: rootPath, browse };
  },
  enable: (b, v) => {
    const root = String(v.root ?? "").trim();
    const browse = !!v.browse;
    let next = b;
    if (root) {
      next = upsert(
        next,
        newDirective("root", ["*", quoteArgIfNeeded(root)]),
      );
    } else {
      next = removeAll(next, "root");
    }
    next = upsert(
      next,
      newDirective("file_server", browse ? ["browse"] : []),
    );
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "file_server");
    next = removeAll(next, "root");
    return next;
  },
};

const encode: Feature & { group: FeatureGroup } = {
  id: "encode",
  group: "performance",
  title: "Compression",
  description: "Compress responses with gzip / zstd.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "algorithms",
      label: "Algorithms (space-separated)",
      placeholder: "gzip zstd",
    },
  ],
  defaults: { algorithms: "gzip zstd" },
  detect: (b) => findFirst(b, "encode") !== null,
  read: (b) => {
    const d = findFirst(b, "encode");
    return { algorithms: d ? d.args.map(unquote).join(" ") : "gzip zstd" };
  },
  enable: (b, v) => {
    const algos = String(v.algorithms ?? "")
      .split(/\s+/)
      .filter(Boolean)
      .map(quoteArgIfNeeded);
    return upsert(
      b,
      newDirective("encode", algos.length ? algos : ["gzip", "zstd"]),
    );
  },
  disable: (b) => removeAll(b, "encode"),
};

const log: Feature & { group: FeatureGroup } = {
  id: "log",
  group: "logging",
  title: "Access log",
  description: "Where access requests are written.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "path",
      label: "Log path",
      placeholder: "/var/log/caddy/access.log",
    },
    {
      type: "select",
      key: "format",
      label: "Format",
      options: [
        { value: "default", label: "default (single-line)" },
        { value: "json", label: "json" },
      ],
    },
  ],
  defaults: { path: "/var/log/caddy/access.log", format: "default" },
  detect: (b) => findFirst(b, "log") !== null,
  read: (b) => {
    const d = findFirst(b, "log");
    if (!d || d.block === null) return { path: "", format: "default" };
    const out = findFirst(d.block, "output");
    const fmt = findFirst(d.block, "format");
    let path = "";
    if (out && out.args.length >= 2 && out.args[0] === "file") {
      path = unquote(out.args[1]);
    }
    let format: string = "default";
    if (fmt && fmt.args[0]) {
      format = unquote(fmt.args[0]);
    }
    return { path, format };
  },
  enable: (b, v) => {
    const path = String(v.path ?? "").trim() || "/var/log/caddy/access.log";
    const format = String(v.format ?? "default");
    const inner: CaddyNode[] = [
      newDirective("output", ["file", quoteArgIfNeeded(path)]),
    ];
    if (format !== "default") {
      inner.push(newDirective("format", [format]));
    }
    return upsert(b, newBlockDirective("log", [], inner));
  },
  disable: (b) => removeAll(b, "log"),
};

const redir: Feature & { group: FeatureGroup } = {
  id: "redir",
  group: "routing",
  title: "Redirect",
  description: "Send requests to a different URL.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "target",
      label: "Target URL",
      placeholder: "https://example.com{uri}",
    },
    {
      type: "select",
      key: "status",
      label: "Status",
      options: [
        { value: "permanent", label: "301 (permanent)" },
        { value: "temporary", label: "302 (temporary)" },
        { value: "html", label: "html (browser-side)" },
      ],
    },
  ],
  defaults: { target: "", status: "permanent" },
  detect: (b) => findFirst(b, "redir") !== null,
  read: (b) => {
    const d = findFirst(b, "redir");
    if (!d) return { target: "", status: "permanent" };
    const target = d.args[0] ? unquote(d.args[0]) : "";
    const status = d.args[1] ? unquote(d.args[1]) : "permanent";
    return { target, status };
  },
  enable: (b, v) => {
    const target = String(v.target ?? "").trim();
    const status = String(v.status ?? "permanent");
    if (!target) return b;
    return upsert(
      b,
      newDirective("redir", [quoteArgIfNeeded(target), status]),
    );
  },
  disable: (b) => removeAll(b, "redir"),
};

const tls: Feature & { group: FeatureGroup } = {
  id: "tls",
  group: "tls",
  title: "TLS",
  description:
    "Configure HTTPS. Caddy auto-issues certs when you give an email; use a cert/key pair for manual control or `internal` for the dev CA.",
  contexts: ["site", "global"],
  fields: [
    {
      type: "select",
      key: "mode",
      label: "Mode",
      options: [
        { value: "auto", label: "Auto (with email)" },
        { value: "manual", label: "Manual cert + key" },
        { value: "internal", label: "Internal (Caddy CA)" },
      ],
    },
    {
      type: "text",
      key: "email",
      label: "ACME contact email",
      placeholder: "you@example.com",
    },
    {
      type: "text",
      key: "certFile",
      label: "Certificate path",
      placeholder: "/etc/ssl/certs/site.crt",
    },
    {
      type: "text",
      key: "keyFile",
      label: "Private key path",
      placeholder: "/etc/ssl/private/site.key",
    },
  ],
  defaults: { mode: "auto", email: "", certFile: "", keyFile: "" },
  detect: (b) => findFirst(b, "tls") !== null,
  read: (b) => {
    const d = findFirst(b, "tls");
    if (!d || d.args.length === 0) {
      return { mode: "auto", email: "", certFile: "", keyFile: "" };
    }
    const a0 = unquote(d.args[0]);
    if (a0 === "internal") {
      return { mode: "internal", email: "", certFile: "", keyFile: "" };
    }
    if (d.args.length >= 2) {
      return {
        mode: "manual",
        email: "",
        certFile: a0,
        keyFile: unquote(d.args[1]),
      };
    }
    return { mode: "auto", email: a0, certFile: "", keyFile: "" };
  },
  enable: (b, v) => {
    const mode = String(v.mode ?? "auto");
    if (mode === "internal") {
      return upsert(b, newDirective("tls", ["internal"]));
    }
    if (mode === "manual") {
      const cert = String(v.certFile ?? "").trim();
      const key = String(v.keyFile ?? "").trim();
      if (!cert || !key) return b;
      return upsert(
        b,
        newDirective("tls", [
          quoteArgIfNeeded(cert),
          quoteArgIfNeeded(key),
        ]),
      );
    }
    // auto mode
    const email = String(v.email ?? "").trim();
    if (!email) return b;
    return upsert(b, newDirective("tls", [quoteArgIfNeeded(email)]));
  },
  disable: (b) => removeAll(b, "tls"),
};

const basicAuth: Feature & { group: FeatureGroup } = {
  id: "basic-auth",
  group: "auth",
  title: "Basic auth",
  description:
    "HTTP basic authentication. Generate a hash with `caddy hash-password` and paste it below.",
  contexts: ["site", "snippet"],
  fields: [
    { type: "text", key: "user", label: "Username", placeholder: "admin" },
    {
      type: "text",
      key: "hash",
      label: "Bcrypt hash",
      placeholder: "$2a$14$...",
    },
  ],
  defaults: { user: "", hash: "" },
  detect: (b) => findFirst(b, "basicauth") !== null,
  read: (b) => {
    const ba = findFirst(b, "basicauth");
    if (!ba || ba.block === null || ba.block.length === 0) {
      return { user: "", hash: "" };
    }
    // Inside the block, each entry is `<user> <hash>`. Take the first.
    const first = ba.block.find(
      (n): n is Directive => isDirective(n),
    );
    if (!first) return { user: "", hash: "" };
    return {
      user: first.name,
      hash: first.args[0] ? unquote(first.args[0]) : "",
    };
  },
  enable: (b, v) => {
    const user = String(v.user ?? "").trim();
    const hash = String(v.hash ?? "").trim();
    if (!user || !hash) return b;
    return upsert(
      b,
      newBlockDirective("basicauth", [], [
        newDirective(user, [quoteArgIfNeeded(hash)]),
      ]),
    );
  },
  disable: (b) => removeAll(b, "basicauth"),
};

const securityHeaders: Feature & { group: FeatureGroup } = {
  id: "security-headers",
  group: "headers",
  title: "Security headers",
  description:
    "Add common security headers via the `header` block.",
  contexts: ["site", "snippet"],
  fields: [
    { type: "switch", key: "frameOptions", label: "X-Frame-Options: SAMEORIGIN" },
    { type: "switch", key: "noSniff", label: "X-Content-Type-Options: nosniff" },
    { type: "switch", key: "hsts", label: "Strict-Transport-Security (HSTS)" },
    { type: "switch", key: "referrer", label: "Referrer-Policy: strict-origin-when-cross-origin" },
  ],
  defaults: {
    frameOptions: true,
    noSniff: true,
    hsts: false,
    referrer: false,
  },
  detect: (b) => {
    const h = findFirst(b, "header");
    if (!h || h.block === null) return false;
    return h.block.some((n) => {
      if (!isDirective(n)) return false;
      const lname = n.name.toLowerCase();
      return (
        lname === "x-frame-options" ||
        lname === "x-content-type-options" ||
        lname === "strict-transport-security" ||
        lname === "referrer-policy"
      );
    });
  },
  read: (b) => {
    const h = findFirst(b, "header");
    let frameOptions = false;
    let noSniff = false;
    let hsts = false;
    let referrer = false;
    if (h && h.block) {
      for (const n of h.block) {
        if (!isDirective(n)) continue;
        const lname = n.name.toLowerCase();
        if (lname === "x-frame-options") frameOptions = true;
        if (lname === "x-content-type-options") noSniff = true;
        if (lname === "strict-transport-security") hsts = true;
        if (lname === "referrer-policy") referrer = true;
      }
    }
    return { frameOptions, noSniff, hsts, referrer };
  },
  enable: (b, v) => {
    // Build (or rebuild) the inner header lines we own. Preserve any
    // user-added headers that aren't part of this feature's set.
    const ours = new Set([
      "x-frame-options",
      "x-content-type-options",
      "strict-transport-security",
      "referrer-policy",
    ]);
    const existing = findFirst(b, "header");
    const preserved: CaddyNode[] = [];
    if (existing && existing.block) {
      for (const n of existing.block) {
        if (
          isDirective(n) &&
          ours.has(n.name.toLowerCase())
        ) {
          continue;
        }
        preserved.push(n);
      }
    }
    const inner: CaddyNode[] = [...preserved];
    if (v.frameOptions) {
      inner.push(newDirective("X-Frame-Options", ["SAMEORIGIN"]));
    }
    if (v.noSniff) {
      inner.push(newDirective("X-Content-Type-Options", ["nosniff"]));
    }
    if (v.hsts) {
      inner.push(
        newDirective("Strict-Transport-Security", [
          quoteArgIfNeeded("max-age=31536000; includeSubDomains"),
        ]),
      );
    }
    if (v.referrer) {
      inner.push(
        newDirective("Referrer-Policy", [
          "strict-origin-when-cross-origin",
        ]),
      );
    }
    if (inner.length === 0) {
      return removeAll(b, "header");
    }
    return upsert(b, newBlockDirective("header", [], inner));
  },
  disable: (b) => {
    // Only strip the headers we own; if anything else remains, keep
    // the block.
    const existing = findFirst(b, "header");
    if (!existing || existing.block === null) return b;
    const ours = new Set([
      "x-frame-options",
      "x-content-type-options",
      "strict-transport-security",
      "referrer-policy",
    ]);
    const remaining = existing.block.filter(
      (n) => !(isDirective(n) && ours.has(n.name.toLowerCase())),
    );
    if (remaining.length === 0) {
      return removeAll(b, "header");
    }
    return upsert(b, newBlockDirective("header", [], remaining));
  },
};

const rewrite: Feature & { group: FeatureGroup } = {
  id: "rewrite",
  group: "routing",
  title: "Rewrite (server-side)",
  description:
    "Internally rewrite a URL path before routing. Use `redir` for client-visible redirects.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "from",
      label: "Path or matcher",
      placeholder: "/old",
    },
    {
      type: "text",
      key: "to",
      label: "Rewrite target",
      placeholder: "/new",
    },
  ],
  defaults: { from: "", to: "" },
  detect: (b) => findFirst(b, "rewrite") !== null,
  read: (b) => {
    const r = findFirst(b, "rewrite");
    if (!r) return { from: "", to: "" };
    return {
      from: r.args[0] ? unquote(r.args[0]) : "",
      to: r.args[1] ? unquote(r.args[1]) : "",
    };
  },
  enable: (b, v) => {
    const from = String(v.from ?? "").trim();
    const to = String(v.to ?? "").trim();
    if (!from || !to) return b;
    return upsert(
      b,
      newDirective("rewrite", [
        quoteArgIfNeeded(from),
        quoteArgIfNeeded(to),
      ]),
    );
  },
  disable: (b) => removeAll(b, "rewrite"),
};

const tryFiles: Feature & { group: FeatureGroup } = {
  id: "try-files",
  group: "routing",
  title: "SPA fallback (try_files)",
  description:
    "Try a list of paths in order; fall back to the last one. Standard SPA pattern: `try_files {path} /index.html`.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "paths",
      label: "Paths (space-separated; first match wins)",
      placeholder: "{path} /index.html",
    },
  ],
  defaults: { paths: "{path} /index.html" },
  detect: (b) => findFirst(b, "try_files") !== null,
  read: (b) => {
    const d = findFirst(b, "try_files");
    return { paths: d ? d.args.map(unquote).join(" ") : "" };
  },
  enable: (b, v) => {
    const paths = String(v.paths ?? "")
      .split(/\s+/)
      .filter(Boolean);
    if (paths.length === 0) return b;
    return upsert(
      b,
      newDirective("try_files", paths.map(quoteArgIfNeeded)),
    );
  },
  disable: (b) => removeAll(b, "try_files"),
};

const phpFastcgi: Feature & { group: FeatureGroup } = {
  id: "php-fastcgi",
  group: "php",
  title: "PHP via FastCGI",
  description:
    "Hand requests off to PHP-FPM. Use a Unix socket (`unix//run/php/php-fpm.sock`) or `host:port`.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "upstream",
      label: "FPM upstream",
      placeholder: "unix//run/php/php-fpm.sock",
    },
  ],
  defaults: { upstream: "unix//run/php/php-fpm.sock" },
  detect: (b) => findFirst(b, "php_fastcgi") !== null,
  read: (b) => {
    const d = findFirst(b, "php_fastcgi");
    return { upstream: d?.args[0] ? unquote(d.args[0]) : "" };
  },
  enable: (b, v) => {
    const upstream = String(v.upstream ?? "").trim();
    if (!upstream) return b;
    return upsert(
      b,
      newDirective("php_fastcgi", [quoteArgIfNeeded(upstream)]),
    );
  },
  disable: (b) => removeAll(b, "php_fastcgi"),
};

const respond: Feature & { group: FeatureGroup } = {
  id: "respond",
  group: "routing",
  title: "Fixed response (respond)",
  description:
    "Return a static body + status. Common for health-check endpoints: `respond /health \"OK\" 200`.",
  contexts: ["site", "snippet"],
  fields: [
    {
      type: "text",
      key: "matcher",
      label: "Path / matcher (optional)",
      placeholder: "/health",
    },
    {
      type: "text",
      key: "body",
      label: "Response body",
      placeholder: "OK",
    },
    {
      type: "text",
      key: "status",
      label: "HTTP status",
      placeholder: "200",
    },
  ],
  defaults: { matcher: "/health", body: "OK", status: "200" },
  detect: (b) => findFirst(b, "respond") !== null,
  read: (b) => {
    const d = findFirst(b, "respond");
    if (!d) return { matcher: "", body: "", status: "" };
    // `respond [matcher] body [status]` — heuristic: a leading arg
    // that starts with `/` or `@` is the matcher.
    let i = 0;
    let matcher = "";
    if (d.args[0] && (d.args[0].startsWith("/") || d.args[0].startsWith("@"))) {
      matcher = unquote(d.args[0]);
      i = 1;
    }
    const body = d.args[i] ? unquote(d.args[i]) : "";
    const status = d.args[i + 1] ? unquote(d.args[i + 1]) : "";
    return { matcher, body, status };
  },
  enable: (b, v) => {
    const matcher = String(v.matcher ?? "").trim();
    const body = String(v.body ?? "").trim();
    const status = String(v.status ?? "").trim();
    if (!body && !status) return b;
    const args: string[] = [];
    if (matcher) args.push(quoteArgIfNeeded(matcher));
    if (body) args.push(quoteArgIfNeeded(body));
    if (status) args.push(status);
    return upsert(b, newDirective("respond", args));
  },
  disable: (b) => removeAll(b, "respond"),
};

const acmeDefaults: Feature & { group: FeatureGroup } = {
  id: "acme-defaults",
  group: "global-options",
  title: "ACME defaults",
  description:
    "Set global ACME contact email and toggle debug logging. Lives at the very top of Caddyfile inside a `{ … }` global block.",
  contexts: ["global"],
  fields: [
    {
      type: "text",
      key: "email",
      label: "Default ACME email",
      placeholder: "you@example.com",
    },
    { type: "switch", key: "debug", label: "Enable debug logging" },
  ],
  defaults: { email: "", debug: false },
  detect: (b) =>
    findFirst(b, "email") !== null || findFirst(b, "debug") !== null,
  read: (b) => {
    const e = findFirst(b, "email");
    const d = findFirst(b, "debug");
    return {
      email: e?.args[0] ? unquote(e.args[0]) : "",
      debug: !!d,
    };
  },
  enable: (b, v) => {
    const email = String(v.email ?? "").trim();
    const debug = !!v.debug;
    let next = b;
    if (email) {
      next = upsert(next, newDirective("email", [quoteArgIfNeeded(email)]));
    } else {
      next = removeAll(next, "email");
    }
    if (debug) {
      next = upsert(next, newDirective("debug", []));
    } else {
      next = removeAll(next, "debug");
    }
    return next;
  },
  disable: (b) => {
    let next = removeAll(b, "email");
    next = removeAll(next, "debug");
    return next;
  },
};

export type GroupedFeature = Feature & { group: FeatureGroup };

export const FEATURES: GroupedFeature[] = [
  acmeDefaults,
  tls,
  reverseProxy,
  phpFastcgi,
  fileServer,
  encode,
  securityHeaders,
  basicAuth,
  rewrite,
  redir,
  tryFiles,
  respond,
  log,
];
