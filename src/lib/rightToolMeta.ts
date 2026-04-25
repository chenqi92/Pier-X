import {
  ChartNoAxesCombined,
  FileText,
  FolderSync,
  GitBranch,
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

export type RightToolMeta = {
  label: string;
  icon: LucideIcon;
  remoteOnly?: boolean;
  dividerAfter?: boolean;
  tintVar?: string;
  splashTitle?: string;
  splashSubtitle?: string;
};

// Remote-tool ordering after the divider: monitor first (broad server
// vitals apply to every host), then sftp (filesystem access is the
// most-used lever on any box), then log tail (paired with sftp since
// both deal with files on the host), then docker, then the database
// stack, with sqlite at the tail since it's not remote-only.
// Markdown + git stay above the divider as local-workspace tools.
export const RIGHT_TOOL_ORDER: RightTool[] = [
  "markdown",
  "git",
  "monitor",
  "sftp",
  "log",
  "docker",
  "firewall",
  "mysql",
  "postgres",
  "redis",
  "sqlite",
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
    icon: FileText,
  },
  git: {
    label: "Git",
    icon: GitBranch,
    dividerAfter: true,
  },
  monitor: {
    label: "Server Monitor",
    icon: ChartNoAxesCombined,
    remoteOnly: true,
    tintVar: "var(--svc-monitor)",
    splashTitle: "Server Monitor",
    splashSubtitle: "Open a saved server to see live CPU, memory, disks, and top processes.",
  },
  docker: {
    label: "Docker",
    icon: DockerIcon,
    remoteOnly: true,
    tintVar: "var(--svc-docker)",
    splashTitle: "Docker",
    splashSubtitle: "Pick a host to list containers, images, networks, and compose stacks.",
  },
  mysql: {
    label: "MySQL",
    icon: MySqlIcon,
    remoteOnly: true,
    tintVar: "var(--svc-mysql)",
    splashTitle: "MySQL",
    splashSubtitle: "Connect through SSH to browse databases, run queries, and edit rows.",
  },
  postgres: {
    label: "PostgreSQL",
    icon: PostgresIcon,
    remoteOnly: true,
    tintVar: "var(--svc-postgres)",
    splashTitle: "PostgreSQL",
    splashSubtitle: "Connect through SSH to explore schemas, tables, and run SQL.",
  },
  redis: {
    label: "Redis",
    icon: RedisIcon,
    remoteOnly: true,
    tintVar: "var(--svc-redis)",
    splashTitle: "Redis",
    splashSubtitle: "Tunnel into a host to browse keyspaces, inspect values, and tail keys.",
  },
  log: {
    label: "Logs",
    icon: LogIcon,
    remoteOnly: true,
    tintVar: "var(--svc-log)",
    splashTitle: "Log Viewer",
    splashSubtitle: "Stream journal, nginx, or custom log tails from a saved server.",
  },
  sftp: {
    label: "SFTP",
    icon: FolderSync,
    remoteOnly: true,
    tintVar: "var(--svc-sftp)",
    splashTitle: "SFTP",
    splashSubtitle: "Browse a remote filesystem, preview files, and transfer in either direction.",
  },
  sqlite: {
    label: "SQLite",
    icon: SqliteIcon,
    tintVar: "var(--svc-sqlite)",
  },
  firewall: {
    label: "Firewall",
    icon: Shield,
    remoteOnly: true,
    tintVar: "var(--svc-firewall)",
    splashTitle: "Firewall",
    splashSubtitle: "Open a saved server to view firewall rules, listening ports, and per-interface traffic.",
  },
};
