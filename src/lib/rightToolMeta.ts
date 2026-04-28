import {
  ChartNoAxesCombined,
  FileText,
  FolderSync,
  GitBranch,
  Globe,
  Package,
  Shield,
} from "lucide-react";
import type { ComponentType, SVGProps } from "react";
import DockerIcon from "../components/icons/DockerIcon";
import LogIcon from "../components/icons/LogIcon";
import MySqlIcon from "../components/icons/MySqlIcon";
import PostgresIcon from "../components/icons/PostgresIcon";
import RedisIcon from "../components/icons/RedisIcon";
import SqliteIcon from "../components/icons/SqliteIcon";
import type { RightTool } from "./types";

export type LucideIcon = ComponentType<SVGProps<SVGSVGElement> & { size?: number | string }>;

/** Logical grouping for the right-side ToolStrip. The strip renders
 *  a thin divider between categories so related tools cluster
 *  visually without needing horizontal space for text labels. */
export type RightToolCategory =
  | "workspace"  // markdown, git
  | "host"       // monitor, firewall
  | "files"      // sftp, log
  | "containers" // docker
  | "database"   // mysql, postgres, redis, sqlite
  | "service";   // webserver, software

export const CATEGORY_LABELS: Record<RightToolCategory, string> = {
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
//   database   → mysql / postgres / redis / sqlite
//   service    → webserver / software     (host-level service management)
//
// Within a category, items are ordered by frequency-of-use among the
// tools that share that category. SQLite, like the other DB clients,
// targets the *server* — it scans the connected host for `.db` /
// `.sqlite` files. Without a remote context the strip button is dim,
// matching mysql / postgres / redis.
export const RIGHT_TOOL_ORDER: RightTool[] = [
  "markdown",
  "git",
  "monitor",
  "firewall",
  "sftp",
  "log",
  "docker",
  "mysql",
  "postgres",
  "redis",
  "sqlite",
  "webserver",
  "software",
];

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
];

export const RIGHT_TOOL_META: Record<RightTool, RightToolMeta> = {
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
  docker: {
    label: "Docker",
    category: "containers",
    icon: DockerIcon,
    remoteOnly: true,
    tintVar: "var(--svc-docker)",
    splashTitle: "Docker",
    splashSubtitle: "Pick a host to list containers, images, networks, and compose stacks.",
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
      "Manage the web server on a saved host — currently nginx (Apache and Caddy support is planned).",
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
};
