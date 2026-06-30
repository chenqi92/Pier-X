import {
  Activity,
  ChartNoAxesCombined,
  Database,
  FileText,
  FolderSync,
  GitBranch,
  Globe,
  Package,
  Search,
  Server,
  Shield,
  Sparkles,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import DockerIcon from "../components/icons/DockerIcon";
import NanoLinkIcon from "../components/icons/NanoLinkIcon";
import LogIcon from "../components/icons/LogIcon";
import MySqlIcon from "../components/icons/MySqlIcon";
import PostgresIcon from "../components/icons/PostgresIcon";
import RedisIcon from "../components/icons/RedisIcon";
import SqliteIcon from "../components/icons/SqliteIcon";
import type { DbProduct, RightTool } from "./types";

export type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

/** Logical grouping for the right-side ToolStrip. The strip renders
 *  a thin divider between categories so related tools cluster
 *  visually without needing horizontal space for text labels. */
export type RightToolCategory =
  | "assistant"  // ai
  | "workspace"  // markdown, git
  | "host"       // monitor, firewall
  | "files"      // sftp, log
  | "containers" // docker
  | "database"   // mysql, postgres, redis, sqlite
  | "service";   // webserver, software

export const CATEGORY_LABELS: Record<RightToolCategory, string> = {
  assistant: "AI assistant",
  workspace: "Workspace",
  host: "Host overview",
  files: "Files & logs",
  containers: "Containers",
  database: "Databases",
  service: "Services",
};

export type RightToolMeta = {
  label: string;
  icon: LucideIcon;
  category: RightToolCategory;
  remoteOnly?: boolean;
  /** Deprecated — use `category` so dividers come from category
   *  boundaries automatically. Kept for any callers that still read it. */
  dividerAfter?: boolean;
  tintVar?: string;
  splashTitle?: string;
  splashSubtitle?: string;
};

// Tool ordering — the strip groups by `category` and renders a thin
// divider on each category change. The category order itself encodes
// "outermost layer first":
//
//   workspace  → markdown / git           (purely local files, every tab)
//   host       → monitor / firewall       (read-mostly OS-level overviews)
//   files      → sftp / log               (filesystem + log tails)
//   containers → docker
//   database   → database (mysql / postgres / sqlite / …) · redis
//   service    → webserver / software     (host-level service management)
//
// Within a category, items are ordered by frequency-of-use among the
// tools that share that category. The relational clients (MySQL,
// PostgreSQL, SQLite, SQL Server, …) collapse into ONE `database` strip
// entry; `DatabasePanel` switches between them in-panel, like the single
// `webserver` tool switches nginx / apache / caddy. Redis keeps its own
// entry — its key-value model doesn't share the relational grid UI.
export const RIGHT_TOOL_ORDER: RightTool[] = [
  "ai",
  "markdown",
  "git",
  "monitor",
  "firewall",
  "sftp",
  "log",
  "search",
  "docker",
  "database",
  "redis",
  "webserver",
  "software",
  "nanolink",
];

// Tools that only appear in the strip once `detectServices` reports them
// on the active host (vs. rendering unconditionally and only dimming via
// `REMOTE_ONLY_TOOLS`). Currently EMPTY: NanoLink used to live here, but
// hiding it on un-installed hosts made the panel's own "install NanoLink"
// flow unreachable — so it now renders on every SSH tab (still showing a
// `detected` dot when found). The mechanism is kept for a future tool
// that genuinely has no use until detected.
export const DETECTION_GATED_TOOLS = new Set<RightTool>([]);

// Firewall is intentionally NOT here: it's a universal capability of any
// Linux host, not a "detected service" — chips here only render when
// `detectServices` returns a matching name, and firewall has no service
// daemon to detect. The tool strip button is enough exposure.
export const SERVICE_CHIP_TOOLS: RightTool[] = [
  "monitor",
  "sftp",
  "log",
  "docker",
  "mysql",
  "postgres",
  "redis",
  "sqlite",
  "nanolink",
];

export const RIGHT_TOOL_META: Record<RightTool, RightToolMeta> = {
  // PRODUCT-SPEC §5.14 — fixed first position. Works on any tab
  // (local / SSH / welcome), so no `remoteOnly` and no splash.
  ai: {
    label: "AI",
    category: "assistant",
    icon: Sparkles,
  },
  markdown: {
    label: "Markdown",
    category: "workspace",
    icon: FileText,
  },
  git: {
    label: "Git",
    category: "workspace",
    icon: GitBranch,
  },
  monitor: {
    label: "Server Monitor",
    category: "host",
    icon: ChartNoAxesCombined,
    // Local-capable: the panel switches to `local_system_info` when
    // the active tab has no SSH target, so the strip button stays
    // enabled on plain local terminals too. This is also the
    // designated fallback when a tab's persisted rightTool is no
    // longer reachable (e.g. local tab restored with rightTool="mysql").
    tintVar: "var(--svc-monitor)",
    splashTitle: "Server Monitor",
    splashSubtitle: "Live CPU, memory, disks, and top processes for the active host (local or SSH).",
  },
  firewall: {
    label: "Firewall",
    category: "host",
    icon: Shield,
    remoteOnly: true,
    tintVar: "var(--svc-firewall)",
    splashTitle: "Firewall",
    splashSubtitle: "Open a saved server to view firewall rules, listening ports, and per-interface traffic.",
  },
  sftp: {
    label: "SFTP",
    category: "files",
    icon: FolderSync,
    remoteOnly: true,
    tintVar: "var(--svc-sftp)",
    splashTitle: "SFTP",
    splashSubtitle: "Browse a remote filesystem, preview files, and transfer in either direction.",
  },
  log: {
    label: "Logs",
    category: "files",
    icon: LogIcon,
    remoteOnly: true,
    tintVar: "var(--svc-log)",
    splashTitle: "Log Viewer",
    splashSubtitle: "Stream journal, nginx, or custom log tails from a saved server.",
  },
  search: {
    label: "Code Search",
    category: "files",
    icon: Search,
    remoteOnly: true,
    tintVar: "var(--svc-search)",
    splashTitle: "Code Search",
    splashSubtitle: "Run ripgrep / git grep across the terminal's working directory and jump straight to the file.",
  },
  docker: {
    label: "Docker",
    category: "containers",
    icon: DockerIcon,
    remoteOnly: true,
    tintVar: "var(--svc-docker)",
    splashTitle: "Docker",
    splashSubtitle: "Pick a host to list containers, images, networks, and compose stacks.",
  },
  // Unified relational-database tool. The strip shows this single entry;
  // DatabasePanel routes the tab's `dbKind` to the matching client. The
  // per-product mysql / postgres / sqlite entries below are retained for
  // their icons and splash copy (reused by the in-panel switcher and the
  // credential dialog) but no longer appear in RIGHT_TOOL_ORDER.
  database: {
    label: "Database",
    category: "database",
    icon: Database,
    remoteOnly: true,
    tintVar: "var(--svc-database)",
    splashTitle: "Database",
    splashSubtitle:
      "Connect through SSH to browse MySQL, PostgreSQL, SQLite, SQL Server, and more — switch products in-panel.",
  },
  mysql: {
    label: "MySQL",
    category: "database",
    icon: MySqlIcon,
    remoteOnly: true,
    tintVar: "var(--svc-mysql)",
    splashTitle: "MySQL",
    splashSubtitle: "Connect through SSH to browse databases, run queries, and edit rows.",
  },
  postgres: {
    label: "PostgreSQL",
    category: "database",
    icon: PostgresIcon,
    remoteOnly: true,
    tintVar: "var(--svc-postgres)",
    splashTitle: "PostgreSQL",
    splashSubtitle: "Connect through SSH to explore schemas, tables, and run SQL.",
  },
  redis: {
    label: "Redis",
    category: "database",
    icon: RedisIcon,
    remoteOnly: true,
    tintVar: "var(--svc-redis)",
    splashTitle: "Redis",
    splashSubtitle: "Tunnel into a host to browse keyspaces, inspect values, and tail keys.",
  },
  sqlite: {
    label: "SQLite",
    category: "database",
    icon: SqliteIcon,
    remoteOnly: true,
    tintVar: "var(--svc-sqlite)",
    splashTitle: "SQLite",
    splashSubtitle: "Connect through SSH to scan a host for .db / .sqlite files and read or edit them.",
  },
  webserver: {
    label: "Web Server",
    category: "service",
    icon: Globe,
    remoteOnly: true,
    tintVar: "var(--svc-webserver)",
    splashTitle: "Web Server",
    splashSubtitle:
      "Manage nginx, Apache, or Caddy on a saved host — browse and edit config, validate, and reload.",
  },
  software: {
    label: "Software",
    category: "service",
    icon: Package,
    remoteOnly: true,
    tintVar: "var(--svc-software)",
    splashTitle: "Software",
    splashSubtitle:
      "Open an SSH tab to view the host's tool stack and install or update packages with live progress.",
  },
  // Detection-gated (see `DETECTION_GATED_TOOLS`): only appears once the
  // NanoLink agent or server is found on the active host. The panel then
  // renders a role-aware view — install / server / client / both.
  nanolink: {
    label: "NanoLink",
    category: "service",
    icon: NanoLinkIcon,
    remoteOnly: true,
    tintVar: "var(--svc-nanolink)",
    splashTitle: "NanoLink",
    splashSubtitle:
      "Manage NanoLink on this host — install the agent, control its connections, or browse the server dashboard.",
  },
};

// ── Database sub-products ───────────────────────────────────────────
//
// Drives the in-panel product switcher, the per-product splash, and the
// add-credential dialog. Adding a new database (Oracle, 达梦 DM, …) means
// appending an entry here, a `DbKind` member, a client panel, and a
// pier-core driver — no changes to the strip or routing.

export type DbKindMeta = {
  label: string;
  icon: LucideIcon;
  tintVar: string;
  splashSubtitle: string;
  /** Default TCP port shown in the connect form. SQLite has none. */
  defaultPort: number | null;
  /** Whether the client browses by schema (PostgreSQL / SQL Server) on
   *  top of database selection. Lets the switcher pre-flag UI affordances. */
  hasSchema: boolean;
  /** False while the backend driver is still landing — the switcher shows
   *  the product but the panel renders a "coming soon" note instead of a
   *  broken connect form. */
  available: boolean;
};

export const DB_KIND_META: Record<DbProduct, DbKindMeta> = {
  mysql: {
    label: "MySQL",
    icon: MySqlIcon,
    tintVar: "var(--svc-mysql)",
    splashSubtitle: "Browse databases, run queries, and edit rows over SSH.",
    defaultPort: 3306,
    hasSchema: false,
    available: true,
  },
  postgres: {
    label: "PostgreSQL",
    icon: PostgresIcon,
    tintVar: "var(--svc-postgres)",
    splashSubtitle: "Explore schemas, tables, and routines, and run SQL over SSH.",
    defaultPort: 5432,
    hasSchema: true,
    available: true,
  },
  sqlite: {
    label: "SQLite",
    icon: SqliteIcon,
    tintVar: "var(--svc-sqlite)",
    splashSubtitle: "Scan a host for .db / .sqlite files and read or edit them.",
    defaultPort: null,
    hasSchema: false,
    available: true,
  },
  sqlserver: {
    label: "SQL Server",
    icon: Server,
    tintVar: "var(--svc-sqlserver)",
    splashSubtitle: "Connect to Microsoft SQL Server to browse schemas and run T-SQL.",
    defaultPort: 1433,
    hasSchema: true,
    available: true,
  },
  influx: {
    label: "InfluxDB",
    icon: Activity,
    tintVar: "var(--svc-influx)",
    splashSubtitle: "Query measurements with InfluxQL on an InfluxDB time-series host.",
    defaultPort: 8086,
    hasSchema: false,
    available: true,
  },
  oracle: {
    label: "Oracle",
    icon: Database,
    tintVar: "var(--svc-oracle)",
    splashSubtitle: "Run SQL via the remote host's sqlplus — connects over SSH, no local client.",
    defaultPort: 1521,
    hasSchema: true,
    available: true,
  },
  dameng: {
    label: "达梦 DM",
    icon: Database,
    tintVar: "var(--svc-dameng)",
    splashSubtitle: "Run SQL via the remote host's disql — connects over SSH, no local client.",
    defaultPort: 5236,
    hasSchema: true,
    available: true,
  },
};
